mod support;

use std::sync::Arc;

use ai_stock_forum::{
    agents::{
        AgentBindingCatalogSnapshot, AgentBindings, AgentProfileDraft, AgentReadiness, AgentRole,
        BindingReferenceId, EngineeringBindingRef, InferenceBindingRef, ProfileDiffField,
    },
    app::{
        ApplicationCommand, ApplicationEvent, ApplicationService, AuthorizationDecision,
        CommandPolicy, CommandTransactionHook, CommandView,
    },
    config::AppPaths,
    persistence::PersistenceError,
    policy::Capability,
    ui::command::{ParsedLine, parse_line},
};
use rusqlite::Connection;
use serde_json::Value;

struct GrantAll;

impl CommandPolicy for GrantAll {
    fn authorize(&self, _capability: Capability) -> AuthorizationDecision {
        AuthorizationDecision::Granted
    }
}

struct NoopHook;

impl CommandTransactionHook for NoopHook {
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
        Ok(())
    }
}

struct Harness {
    _temporary_directory: tempfile::TempDir,
    paths: AppPaths,
    service: ApplicationService,
}

impl Harness {
    fn new(binding_catalog: AgentBindingCatalogSnapshot) -> Self {
        let temporary_directory = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temporary_directory.path());
        let service = ApplicationService::bootstrap_with_dependencies_and_binding_catalog(
            &paths,
            Arc::new(support::TestClock::new()),
            Arc::new(support::TestIds::new()),
            Arc::new(GrantAll),
            Arc::new(NoopHook),
            binding_catalog,
        )
        .unwrap();
        Self {
            _temporary_directory: temporary_directory,
            paths,
            service,
        }
    }

    fn execute(&mut self, command: ApplicationCommand) -> ai_stock_forum::app::CommandOutcome {
        self.service.execute_user(command).unwrap()
    }

    fn receipt_json(&self, command_id: ai_stock_forum::domain::CommandId) -> Value {
        let encoded = Connection::open(self.paths.database_path())
            .unwrap()
            .query_row(
                "SELECT outcome_json FROM command_receipts WHERE command_id = ?1",
                [command_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        serde_json::from_str(&encoded).unwrap()
    }
}

fn draft(name: &str, role: AgentRole, bindings: AgentBindings) -> AgentProfileDraft {
    AgentProfileDraft::new(
        name.to_owned(),
        "Evidence-led profile.".to_owned(),
        role,
        "equity research".to_owned(),
        vec!["valuation".to_owned()],
        "Calm and skeptical.".to_owned(),
        "Cite evidence before making a claim.".to_owned(),
        bindings,
        Vec::new(),
        Vec::new(),
    )
    .unwrap()
}

fn parsed_command(input: &str) -> ApplicationCommand {
    match parse_line(input.as_bytes()) {
        ParsedLine::Command(command) => command,
        other => panic!("expected direct command, got {other:?}"),
    }
}

fn values_for_key<'a>(value: &'a Value, key: &str, found: &mut Vec<&'a Value>) {
    match value {
        Value::Object(object) => {
            if let Some(value) = object.get(key) {
                found.push(value);
            }
            for value in object.values() {
                values_for_key(value, key, found);
            }
        }
        Value::Array(values) => {
            for value in values {
                values_for_key(value, key, found);
            }
        }
        _ => {}
    }
}

fn assert_bounded_receipt(receipt: &Value, collection_key: &str, total: u64, returned: u64) {
    let mut collections = Vec::new();
    values_for_key(receipt, collection_key, &mut collections);
    assert_eq!(collections.len(), 1);
    assert_eq!(collections[0].as_array().unwrap().len(), returned as usize);

    for (key, expected) in [("total_count", total), ("returned_count", returned)] {
        let mut values = Vec::new();
        values_for_key(receipt, key, &mut values);
        assert!(!values.is_empty(), "missing {key} metadata");
        assert!(values.iter().all(|value| value.as_u64() == Some(expected)));
    }
    let mut truncated = Vec::new();
    values_for_key(receipt, "truncated", &mut truncated);
    assert!(!truncated.is_empty());
    assert!(truncated.iter().all(|value| value.as_bool() == Some(true)));
}

#[test]
fn service_derives_role_specific_readiness_from_the_injected_typed_catalog() {
    let inference = InferenceBindingRef::new(
        BindingReferenceId::new("connection.openai").unwrap(),
        BindingReferenceId::new("model.gpt-5").unwrap(),
    );
    let engineering =
        EngineeringBindingRef::new(BindingReferenceId::new("runtime.local-codex").unwrap());
    let catalog =
        AgentBindingCatalogSnapshot::new(vec![inference.clone()], vec![engineering.clone()]);
    let mut harness = Harness::new(catalog);

    let cases = [
        (
            "Ready Researcher",
            AgentRole::Bull,
            AgentBindings::new(Some(inference.clone()), None),
            AgentReadiness::Ready,
        ),
        (
            "Ready Engineer",
            AgentRole::Engineering,
            AgentBindings::new(Some(inference.clone()), Some(engineering)),
            AgentReadiness::Ready,
        ),
        (
            "Unavailable Researcher",
            AgentRole::Bull,
            AgentBindings::new(
                Some(InferenceBindingRef::new(
                    BindingReferenceId::new("connection.missing").unwrap(),
                    BindingReferenceId::new("model.missing").unwrap(),
                )),
                None,
            ),
            AgentReadiness::BindingUnavailable,
        ),
        (
            "Unbound Researcher",
            AgentRole::Bull,
            AgentBindings::default(),
            AgentReadiness::Unbound,
        ),
    ];

    for (name, role, bindings, expected) in cases {
        let outcome = harness.execute(ApplicationCommand::CreateAgentProfile {
            draft: draft(name, role, bindings),
            template_provenance: None,
        });
        let CommandView::AgentProfileCreated(created) = outcome.view else {
            panic!("create did not return a profile view");
        };
        assert_eq!(created.readiness, expected, "{name}");
    }
}

#[test]
fn name_and_id_selectors_load_exact_historical_content_and_predecessor_diff() {
    let mut harness = Harness::new(AgentBindingCatalogSnapshot::default());
    let created = harness.execute(ApplicationCommand::CreateAgentProfile {
        draft: draft(
            "Historical Analyst",
            AgentRole::Custom,
            AgentBindings::default(),
        ),
        template_provenance: None,
    });
    let CommandView::AgentProfileCreated(created) = created.view else {
        panic!("create view");
    };

    let shown = harness.execute(parsed_command("/agent show \"Historical Analyst\""));
    let CommandView::AgentProfile(shown) = shown.view else {
        panic!("name selector did not return detail");
    };
    assert_eq!(shown.profile.profile_id(), created.profile_id);

    let mut candidate = shown.profile.to_draft();
    candidate.description = "Second immutable version.".to_owned();
    let preview = harness
        .service
        .preview_agent_profile_edit(
            created.profile_id,
            created.profile_version_id,
            candidate.clone(),
        )
        .unwrap();
    let activated = harness.execute(ApplicationCommand::ActivateAgentProfileVersion {
        profile_id: created.profile_id,
        expected_active_version_id: created.profile_version_id,
        candidate,
        review_token: preview.review_token,
        review_digest: preview.review_digest,
    });
    let CommandView::AgentProfileVersionActivated(activated) = activated.view else {
        panic!("activation view");
    };

    let first = harness.execute(parsed_command("/agent history \"Historical Analyst\" 1"));
    assert!(matches!(
        first.committed_events[0].event,
        ApplicationEvent::AgentProfileVersionViewed { .. }
    ));
    let CommandView::AgentProfileVersion(first) = first.view else {
        panic!("exact version-one view");
    };
    assert_eq!(
        first.profile.profile_version_id(),
        created.profile_version_id
    );
    assert_eq!(first.profile.description(), "Evidence-led profile.");
    assert!(first.predecessor_diff.is_empty());

    let second = harness.execute(parsed_command(&format!(
        "/agent history {} 2",
        created.profile_id
    )));
    let CommandView::AgentProfileVersion(second) = second.view else {
        panic!("exact version-two view");
    };
    assert_eq!(
        second.profile.profile_version_id(),
        activated.profile_version_id
    );
    assert_eq!(second.profile.description(), "Second immutable version.");
    assert_eq!(second.predecessor_diff.len(), 1);
    assert_eq!(
        second.predecessor_diff[0].field,
        ProfileDiffField::Description
    );
}

#[test]
fn list_and_history_are_bounded_before_events_outcomes_and_receipts() {
    let mut list_harness = Harness::new(AgentBindingCatalogSnapshot::default());
    for index in 0..101 {
        list_harness.execute(ApplicationCommand::CreateAgentProfile {
            draft: draft(
                &format!("Bounded Analyst {index:03}"),
                AgentRole::Custom,
                AgentBindings::default(),
            ),
            template_provenance: None,
        });
    }
    let listed = list_harness.execute(ApplicationCommand::ListAgentProfiles);
    let list_receipt = list_harness.receipt_json(listed.command_id);
    assert!(matches!(
        listed.committed_events[0].event,
        ApplicationEvent::AgentProfilesListed {
            total_count: 101,
            returned_count: 100,
            truncated: true,
        }
    ));
    let CommandView::AgentProfiles(listed_view) = listed.view else {
        panic!("list view");
    };
    assert_eq!(listed_view.profiles.len(), 100);
    assert_eq!(listed_view.total_count, 101);
    assert_eq!(listed_view.returned_count, 100);
    assert!(listed_view.truncated);
    assert_bounded_receipt(&list_receipt, "profiles", 101, 100);

    let mut history_harness = Harness::new(AgentBindingCatalogSnapshot::default());
    let created = history_harness.execute(ApplicationCommand::CreateAgentProfile {
        draft: draft(
            "Bounded History Analyst",
            AgentRole::Custom,
            AgentBindings::default(),
        ),
        template_provenance: None,
    });
    let CommandView::AgentProfileCreated(created) = created.view else {
        panic!("history create view");
    };
    let mut active_version_id = created.profile_version_id;
    for version in 2..=101 {
        let mut candidate = draft(
            "Bounded History Analyst",
            AgentRole::Custom,
            AgentBindings::default(),
        );
        candidate.description = format!("Immutable version {version}.");
        let preview = history_harness
            .service
            .preview_agent_profile_edit(created.profile_id, active_version_id, candidate.clone())
            .unwrap();
        let activated = history_harness.execute(ApplicationCommand::ActivateAgentProfileVersion {
            profile_id: created.profile_id,
            expected_active_version_id: active_version_id,
            candidate,
            review_token: preview.review_token,
            review_digest: preview.review_digest,
        });
        let CommandView::AgentProfileVersionActivated(view) = activated.view else {
            panic!("history activation view");
        };
        active_version_id = view.profile_version_id;
    }
    let history = history_harness.execute(parsed_command(&format!(
        "/agent history {}",
        created.profile_id
    )));
    let history_receipt = history_harness.receipt_json(history.command_id);
    assert!(matches!(
        history.committed_events[0].event,
        ApplicationEvent::AgentProfileHistoryViewed {
            total_count: 101,
            returned_count: 100,
            truncated: true,
            ..
        }
    ));
    let CommandView::AgentProfileHistory(history_view) = history.view else {
        panic!("history view");
    };
    assert_eq!(history_view.versions.len(), 100);
    assert_eq!(history_view.total_count, 101);
    assert_eq!(history_view.returned_count, 100);
    assert!(history_view.truncated);
    assert_eq!(history_view.versions[0].version.get(), 101);
    assert_eq!(history_view.versions[99].version.get(), 2);
    assert_bounded_receipt(&history_receipt, "versions", 101, 100);
}
