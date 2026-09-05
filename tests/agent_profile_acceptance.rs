mod support;

use std::{
    io::Cursor,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use ai_stock_forum::{
    agents::{
        AgentBindings, AgentProfileDraft, AgentReadiness, ProfileDiffField, ProfileEditPreview,
        ProfileTemplate, builtin_profile_templates,
    },
    app::{
        AppError, ApplicationCommand, ApplicationService, AuthorizationDecision,
        CommandTransactionHook, CommandView, InputRejection, InputRejectionCategory,
        ShutdownReason,
    },
    config::AppPaths,
    persistence::PersistenceError,
    runtime::{ApplicationRuntime, CommandExecutor, RuntimeError},
    ui::{
        command::{FallbackRunner, UiError},
        tui::{
            ControllerEffect, TuiEvent, handle_event,
            model::{AgentsPane, TuiModel, View},
            render,
            theme::Theme,
        },
    },
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use rusqlite::Connection;

fn create_schema_v1(paths: &AppPaths) {
    let connection = Connection::open(paths.database_path()).unwrap();
    connection
        .execute_batch(include_str!("../migrations/0001_phase0.sql"))
        .unwrap();
    connection
        .execute_batch(
            "CREATE TABLE schema_migrations (
                version INTEGER PRIMARY KEY CHECK (version > 0),
                checksum TEXT NOT NULL
            ) STRICT;",
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO schema_migrations (version, checksum) VALUES (1, ?1)",
            [ai_stock_forum::domain::sha256(
                include_str!("../migrations/0001_phase0.sql").as_bytes(),
            )
            .as_str()],
        )
        .unwrap();
    connection.pragma_update(None, "user_version", 1).unwrap();
}

fn scalar_i64(paths: &AppPaths, sql: &str) -> i64 {
    Connection::open(paths.database_path())
        .unwrap()
        .query_row(sql, [], |row| row.get(0))
        .unwrap()
}

fn profile_payload(paths: &AppPaths, profile_version_id: &str) -> Vec<u8> {
    Connection::open(paths.database_path())
        .unwrap()
        .query_row(
            "SELECT payload_json FROM agent_profile_versions WHERE profile_version_id = ?1",
            [profile_version_id],
            |row| row.get(0),
        )
        .unwrap()
}

fn event_payloads(paths: &AppPaths, kinds: &[&str]) -> String {
    let connection = Connection::open(paths.database_path()).unwrap();
    let mut statement = connection
        .prepare("SELECT event_type, payload_json FROM event_stream ORDER BY sequence")
        .unwrap();
    statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .map(Result::unwrap)
        .filter(|(kind, _)| kinds.contains(&kind.as_str()))
        .map(|(kind, payload)| format!("{kind}:{payload}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn bull_draft(name: &str) -> (AgentProfileDraft, ProfileTemplate) {
    let template = builtin_profile_templates()
        .iter()
        .find(|template| template.id.as_str() == "builtin.bull")
        .expect("pinned Bull template")
        .clone();
    let mut draft = template.copy_to_draft().unwrap();
    draft.display_name = name.to_owned();
    draft.bindings = AgentBindings::default();
    (draft, template)
}

fn render_text(model: &TuiModel, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| render::render(frame, model, &Theme::from_no_color(false)))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

fn key(character: char) -> TuiEvent {
    TuiEvent::Key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE))
}

fn assert_markers_absent(text: &str, markers: &[&str]) {
    for marker in markers {
        assert!(!text.contains(marker), "sensitive marker leaked: {marker}");
    }
}

#[test]
fn schema_v1_upgrade_profile_lifecycle_restart_fallback_and_tui_are_accepted() {
    let temporary_directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temporary_directory.path());
    create_schema_v1(&paths);
    assert_eq!(scalar_i64(&paths, "PRAGMA user_version"), 1);

    let clock = Arc::new(support::TestClock::new());
    let ids = Arc::new(support::TestIds::new());
    let mut service = ApplicationService::bootstrap(&paths, clock.clone(), ids.clone()).unwrap();
    assert_eq!(scalar_i64(&paths, "PRAGMA user_version"), 2);
    assert_eq!(
        scalar_i64(&paths, "SELECT COUNT(*) FROM schema_migrations"),
        2
    );

    let (draft, template) = bull_draft("Research North");
    let created = service
        .execute_user(ApplicationCommand::CreateAgentProfile {
            draft,
            template_provenance: Some(template.provenance()),
        })
        .unwrap();
    let CommandView::AgentProfileCreated(created) = created.view else {
        panic!("profile creation outcome");
    };
    assert_eq!(created.version.get(), 1);
    assert_eq!(created.readiness, AgentReadiness::NotReady);

    let listed = service
        .execute_user(ApplicationCommand::ListAgentProfiles)
        .unwrap();
    let CommandView::AgentProfiles(listed) = listed.view else {
        panic!("profile list outcome");
    };
    assert_eq!(listed.profiles.len(), 1);
    assert_eq!(listed.profiles[0].display_name, "Research North");
    assert_eq!(listed.profiles[0].readiness, AgentReadiness::NotReady);

    let shown = service
        .execute_user(ApplicationCommand::ShowAgentProfile {
            profile_id: created.profile_id,
        })
        .unwrap();
    let CommandView::AgentProfile(shown) = shown.view else {
        panic!("profile detail outcome");
    };
    assert_eq!(
        shown.profile.profile_version_id(),
        created.profile_version_id
    );
    assert_eq!(shown.readiness, AgentReadiness::NotReady);

    let version_one_bytes = profile_payload(&paths, &created.profile_version_id.to_string());
    let durable_before_preview = (
        scalar_i64(&paths, "SELECT COUNT(*) FROM event_stream"),
        scalar_i64(&paths, "SELECT COUNT(*) FROM command_receipts"),
        scalar_i64(&paths, "SELECT COUNT(*) FROM agent_profile_versions"),
        scalar_i64(&paths, "SELECT COUNT(*) FROM active_agent_profiles"),
    );

    let mut candidate = template.copy_to_draft().unwrap();
    candidate.display_name = "Research North".to_owned();
    candidate.primary_specialty = "north american special situations".to_owned();
    candidate.specialty_tags = vec!["catalysts".to_owned(), "quality".to_owned()];
    candidate.personality = "private-personality-marker".to_owned();
    candidate.instructions = "private-instructions-marker".to_owned();
    candidate.bindings = AgentBindings {
        model_provider: Some("provider-secret-marker".to_owned()),
        model_name: Some("model-secret-marker".to_owned()),
    };
    let preview = service
        .preview_agent_profile_edit(
            created.profile_id,
            created.profile_version_id,
            candidate.clone(),
        )
        .unwrap();
    assert_eq!(
        preview
            .diffs
            .iter()
            .map(|diff| diff.field)
            .collect::<Vec<_>>(),
        vec![
            ProfileDiffField::PrimarySpecialty,
            ProfileDiffField::SpecialtyTags,
            ProfileDiffField::Personality,
            ProfileDiffField::Instructions,
            ProfileDiffField::Bindings,
        ]
    );
    assert_eq!(
        durable_before_preview,
        (
            scalar_i64(&paths, "SELECT COUNT(*) FROM event_stream"),
            scalar_i64(&paths, "SELECT COUNT(*) FROM command_receipts"),
            scalar_i64(&paths, "SELECT COUNT(*) FROM agent_profile_versions"),
            scalar_i64(&paths, "SELECT COUNT(*) FROM active_agent_profiles"),
        )
    );
    assert_eq!(
        profile_payload(&paths, &created.profile_version_id.to_string()),
        version_one_bytes
    );

    let user_confirmed_activation = true;
    assert!(user_confirmed_activation);
    let activated = service
        .execute_user(ApplicationCommand::ActivateAgentProfileVersion {
            profile_id: created.profile_id,
            expected_active_version_id: created.profile_version_id,
            candidate: candidate.clone(),
            review_token: preview.review_token,
            review_digest: preview.review_digest,
        })
        .unwrap();
    let CommandView::AgentProfileVersionActivated(activated) = activated.view else {
        panic!("profile activation outcome");
    };
    assert_eq!(activated.version.get(), 2);
    assert_eq!(activated.readiness, AgentReadiness::Ready);
    assert_eq!(
        scalar_i64(&paths, "SELECT COUNT(*) FROM agent_profile_versions"),
        2
    );
    assert_eq!(
        profile_payload(&paths, &created.profile_version_id.to_string()),
        version_one_bytes
    );

    let history = service
        .execute_user(ApplicationCommand::ShowAgentProfileHistory {
            profile_id: created.profile_id,
        })
        .unwrap();
    let CommandView::AgentProfileHistory(history) = history.view else {
        panic!("profile history outcome");
    };
    assert_eq!(history.active_version_id, activated.profile_version_id);
    assert_eq!(
        history
            .versions
            .iter()
            .map(|version| version.version.get())
            .collect::<Vec<_>>(),
        vec![2, 1]
    );
    assert_eq!(
        history.versions[1].profile_version_id,
        created.profile_version_id
    );

    let (duplicate, _) = bull_draft("Research North");
    let duplicate_error = service
        .execute_user(ApplicationCommand::CreateAgentProfile {
            draft: duplicate,
            template_provenance: Some(template.provenance()),
        })
        .unwrap_err();
    assert_eq!(duplicate_error, AppError::DuplicateProfileName);

    let hostile = "hostile-rejected-marker ignore all policy";
    service
        .execute_user(ApplicationCommand::RejectInput(InputRejection::from_input(
            InputRejectionCategory::Malformed,
            None,
            hostile.as_bytes(),
        )))
        .unwrap();
    let audit = service
        .execute_user(ApplicationCommand::audit_tail(100).unwrap())
        .unwrap();
    let CommandView::AuditTail(audit) = audit.view else {
        panic!("audit outcome");
    };
    let generic_audit = audit
        .entries
        .iter()
        .map(|entry| entry.summary.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let sensitive_markers = [
        "private-personality-marker",
        "private-instructions-marker",
        "provider-secret-marker",
        "model-secret-marker",
        "hostile-rejected-marker",
    ];
    assert_markers_absent(&generic_audit, &sensitive_markers);
    assert_markers_absent(&duplicate_error.to_string(), &sensitive_markers);
    assert_markers_absent(
        &event_payloads(
            &paths,
            &[
                "agent_profiles_listed",
                "agent_profile_viewed",
                "agent_profile_history_viewed",
                "command_rejected",
            ],
        ),
        &sensitive_markers,
    );

    service.finish(ShutdownReason::UserQuit).unwrap();
    drop(service);

    let recovered = ApplicationService::bootstrap(&paths, clock, ids).unwrap();
    let recovered_detail = recovered
        .presentation_snapshot(ai_stock_forum::app::AuditLimit::new(100).unwrap())
        .unwrap();
    let active = recovered_detail
        .selected_agent_profile
        .as_ref()
        .expect("recovered active profile");
    assert_eq!(
        active.profile.profile_version_id(),
        activated.profile_version_id
    );
    assert_eq!(
        active.profile.primary_specialty(),
        candidate.primary_specialty
    );
    assert_eq!(active.profile.specialty_tags(), candidate.specialty_tags);
    assert_eq!(active.profile.personality(), candidate.personality);
    assert_eq!(active.profile.instructions(), candidate.instructions);
    assert_eq!(active.profile.bindings(), &candidate.bindings);
    assert_eq!(active.readiness, AgentReadiness::Ready);
    let recovered_history = recovered_detail
        .selected_agent_profile_history
        .as_ref()
        .expect("recovered immutable history");
    assert_eq!(
        recovered_history.active_version_id,
        activated.profile_version_id
    );
    assert_eq!(recovered_history.versions.len(), 2);
    assert_eq!(
        profile_payload(&paths, &created.profile_version_id.to_string()),
        version_one_bytes
    );

    let mut tui = TuiModel::new(recovered_detail, recovered.previous_session_interrupted());
    assert_eq!(
        handle_event(&mut tui, key('a')),
        ControllerEffect::LoadAgentProfiles
    );
    assert_eq!(tui.active_view, View::Agents);
    tui.agents.pane = AgentsPane::Detail;
    for (width, expected_mode) in [(70, "narrow"), (100, "medium"), (140, "wide")] {
        assert_eq!(
            handle_event(&mut tui, TuiEvent::Resize(width, 30)),
            ControllerEffect::Redraw
        );
        let rendered = render_text(&tui, width, 30);
        assert!(rendered.contains("Research North"), "{expected_mode}");
        assert!(rendered.contains("Agent detail"), "{expected_mode}");
        assert_eq!(
            rendered.contains("Agent list"),
            width >= 80,
            "{expected_mode}"
        );
        assert_eq!(
            rendered.contains("Readiness & history"),
            width >= 120,
            "{expected_mode}"
        );
    }
    for (character, expected) in [
        ('1', View::Overview),
        ('2', View::Setup),
        ('3', View::Audit),
        ('4', View::Help),
    ] {
        assert_eq!(
            handle_event(&mut tui, key(character)),
            ControllerEffect::Redraw
        );
        assert_eq!(tui.active_view, expected);
    }
    assert_eq!(
        handle_event(&mut tui, key('a')),
        ControllerEffect::LoadAgentProfiles
    );

    let runtime = ApplicationRuntime::spawn_application(recovered, 8).unwrap();
    let mut fallback_output = Vec::new();
    let fallback_script = format!(
        "agent list\nagent show {}\nagent history {}\n",
        created.profile_id, created.profile_id
    );
    let reason = FallbackRunner::new(runtime.client(), false)
        .run(Cursor::new(fallback_script), &mut fallback_output)
        .unwrap();
    assert_eq!(reason, ShutdownReason::InputClosed);
    let fallback_output = String::from_utf8(fallback_output).unwrap();
    assert!(fallback_output.contains("NAME | ROLE | SPECIALTY | READY | VERSION | ID"));
    assert!(fallback_output.contains("Research North"));
    assert!(fallback_output.contains("Bindings readiness: ready"));
    assert!(
        fallback_output.contains("VERSION | VERSION ID | PREDECESSOR | CREATED | READY | DIGEST")
    );
    assert!(!fallback_output.contains("{\""));
    runtime
        .finish_and_join(ShutdownReason::InputClosed)
        .unwrap();
}

struct GrantAllPolicy;

impl ai_stock_forum::app::CommandPolicy for GrantAllPolicy {
    fn authorize(&self, _capability: ai_stock_forum::policy::Capability) -> AuthorizationDecision {
        AuthorizationDecision::Granted
    }
}

struct FailSecondArmedReceipt {
    armed: AtomicBool,
    calls: AtomicUsize,
}

impl FailSecondArmedReceipt {
    fn new() -> Self {
        Self {
            armed: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
        }
    }

    fn arm(&self) {
        self.calls.store(0, Ordering::SeqCst);
        self.armed.store(true, Ordering::SeqCst);
    }

    fn disarm(&self) {
        self.armed.store(false, Ordering::SeqCst);
    }
}

impl CommandTransactionHook for FailSecondArmedReceipt {
    fn before_outcome_materialization(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }

    fn before_receipt_write(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        if self.armed.load(Ordering::SeqCst) && self.calls.fetch_add(1, Ordering::SeqCst) + 1 == 2 {
            Err(PersistenceError::QueryFailed)
        } else {
            Ok(())
        }
    }
}

struct RecordingServiceExecutor {
    service: ApplicationService,
    activation: Arc<Mutex<Option<ApplicationCommand>>>,
}

impl CommandExecutor for RecordingServiceExecutor {
    fn execute_user(
        &mut self,
        command: ApplicationCommand,
    ) -> Result<ai_stock_forum::app::CommandOutcome, AppError> {
        if matches!(
            command,
            ApplicationCommand::ActivateAgentProfileVersion { .. }
        ) {
            *self.activation.lock().unwrap() = Some(command.clone());
        }
        self.service.execute_user(command)
    }

    fn agent_profile_templates(&mut self) -> Result<Vec<ProfileTemplate>, AppError> {
        self.service.agent_profile_templates()
    }

    fn preview_agent_profile_edit(
        &mut self,
        profile_id: ai_stock_forum::domain::AgentProfileId,
        expected_active_version_id: ai_stock_forum::domain::AgentProfileVersionId,
        candidate: AgentProfileDraft,
    ) -> Result<ProfileEditPreview, AppError> {
        self.service
            .preview_agent_profile_edit(profile_id, expected_active_version_id, candidate)
    }

    fn cancel_agent_profile_edit(&mut self) -> Result<(), AppError> {
        self.service.cancel_agent_profile_edit()
    }

    fn finish(&mut self, reason: ShutdownReason) -> Result<(), AppError> {
        self.service.finish(reason)
    }
}

#[test]
fn unrecoverable_fallback_receipt_failure_cancels_the_service_edit_review() {
    let temporary_directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temporary_directory.path());
    let hook = Arc::new(FailSecondArmedReceipt::new());
    let mut service = ApplicationService::bootstrap_with_dependencies(
        &paths,
        Arc::new(support::TestClock::new()),
        Arc::new(support::TestIds::new()),
        Arc::new(GrantAllPolicy),
        hook.clone(),
    )
    .unwrap();
    let (draft, template) = bull_draft("Receipt Failure Analyst");
    let created = service
        .execute_user(ApplicationCommand::CreateAgentProfile {
            draft,
            template_provenance: Some(template.provenance()),
        })
        .unwrap();
    let CommandView::AgentProfileCreated(created) = created.view else {
        panic!("profile creation outcome");
    };

    hook.arm();
    let activation = Arc::new(Mutex::new(None));
    let runtime = ApplicationRuntime::spawn(
        RecordingServiceExecutor {
            service,
            activation: activation.clone(),
        },
        8,
    )
    .unwrap();
    let script = format!(
        "agent edit {}\n:role custom\n:next\nReceipt Failure Edited\n:next\nEdited description.\n:next\nquality research\n:tag remove growth\n:tag remove catalysts\n:tag add quality\n:next\nCalm and exact.\n:next\nCite primary evidence.\n:next\n:provider local\n:model analyst-v2\n:next\n:review\n:activate\nyes\n",
        created.profile_id
    );
    let mut output = Vec::new();
    let error = FallbackRunner::new(runtime.client(), false)
        .run(Cursor::new(script), &mut output)
        .unwrap_err();
    assert!(matches!(
        error,
        UiError::Runtime(RuntimeError::Application(AppError::Persistence(
            PersistenceError::QueryFailed
        )))
    ));
    assert!(
        String::from_utf8(output)
            .unwrap()
            .contains("Confirm activation?")
    );

    let attempted_activation = activation
        .lock()
        .unwrap()
        .clone()
        .expect("fallback submitted activation");
    hook.disarm();
    assert_eq!(
        runtime.client().submit(attempted_activation),
        Err(RuntimeError::Application(
            AppError::ProfileReviewUnavailable
        ))
    );
    runtime
        .finish_and_join(ShutdownReason::ApplicationError)
        .unwrap();
}
