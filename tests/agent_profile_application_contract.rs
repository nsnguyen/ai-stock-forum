mod support;

use std::sync::Arc;

use ai_stock_forum::{
    agents::{
        AgentBindings, AgentProfileDraft, AgentReadiness, AgentRole, ProfileDiffField,
        ProfileTemplateProvenance, builtin_profile_templates,
    },
    app::{
        AppError, ApplicationCommand, AuthorizationDecision, CommandEnvelope, CommandView,
        ShutdownReason,
    },
    domain::{
        Actor, AgentProfileId, AgentProfileVersionId, CommandId, CorrelationId, Digest,
    },
    policy::{Capability, PolicyDecision},
};
use uuid::Uuid;

fn envelope(id: u128, command: ApplicationCommand) -> CommandEnvelope {
    CommandEnvelope {
        command_id: CommandId::from_uuid(Uuid::from_u128(id)),
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(id + 10_000)),
        actor: Actor::Human,
        command,
    }
}

fn custom_draft(name: &str) -> AgentProfileDraft {
    AgentProfileDraft::new(
        name.to_owned(),
        "Sensitive evidence-led profile description.".to_owned(),
        AgentRole::Custom,
        "equity research".to_owned(),
        vec!["research".to_owned()],
        "Sensitive calm and skeptical personality.".to_owned(),
        "Sensitive instructions: cite primary evidence.".to_owned(),
        AgentBindings::default(),
        Vec::new(),
        Vec::new(),
    )
    .unwrap()
}

fn edited_template_draft() -> (AgentProfileDraft, ProfileTemplateProvenance) {
    let template = &builtin_profile_templates()[0];
    let mut draft = template.copy_to_draft().unwrap();
    draft.display_name = "Independent Catalyst Analyst".to_owned();
    draft.description = "Independently edited description.".to_owned();
    draft.role = AgentRole::Engineering;
    draft.primary_specialty = "catalyst systems".to_owned();
    draft.specialty_tags = vec!["catalysts".to_owned(), "systems".to_owned()];
    draft.personality = "Methodical and constructively skeptical.".to_owned();
    draft.instructions = "Separate evidence, assumptions, and conclusions.".to_owned();
    draft.bindings = AgentBindings {
        model_provider: Some("provider-internal-name".to_owned()),
        model_name: Some("model-internal-name".to_owned()),
    };
    (draft, template.provenance())
}

fn create(
    app: &mut support::TestApp,
    id: u128,
    draft: AgentProfileDraft,
    template_provenance: Option<ProfileTemplateProvenance>,
) -> ai_stock_forum::app::AgentProfileCreatedView {
    let outcome = app
        .execute(envelope(
            id,
            ApplicationCommand::CreateAgentProfile {
                draft,
                template_provenance,
            },
        ))
        .unwrap();
    let CommandView::AgentProfileCreated(view) = outcome.view else {
        panic!("agent profile created view")
    };
    view
}

fn activate_command(
    profile_id: AgentProfileId,
    expected_active_version_id: AgentProfileVersionId,
    candidate: AgentProfileDraft,
    review_token: ai_stock_forum::domain::ProfileReviewToken,
    review_digest: Digest,
) -> ApplicationCommand {
    ApplicationCommand::ActivateAgentProfileVersion {
        profile_id,
        expected_active_version_id,
        candidate,
        review_token,
        review_digest,
    }
}

#[test]
fn create_from_an_edited_template_activates_version_one_and_unbound_is_not_ready() {
    let mut app = support::app();
    let (draft, provenance) = edited_template_draft();
    let created = create(&mut app, 100, draft, Some(provenance.clone()));

    assert_eq!(created.version.get(), 1);
    assert_eq!(created.readiness, AgentReadiness::Ready);
    assert_eq!(app.count_rows("agent_profile_versions"), 1);
    assert_eq!(app.count_rows("active_agent_profiles"), 1);

    let unbound = create(&mut app, 101, custom_draft("Unbound Analyst"), None);
    assert_eq!(unbound.version.get(), 1);
    assert_eq!(unbound.readiness, AgentReadiness::NotReady);

    let detail = app
        .execute(envelope(
            102,
            ApplicationCommand::ShowAgentProfile {
                profile_id: created.profile_id,
            },
        ))
        .unwrap();
    let CommandView::AgentProfile(detail) = detail.view else {
        panic!("agent profile detail view")
    };
    assert_eq!(detail.profile.template_provenance(), Some(&provenance));
    assert_eq!(detail.profile.display_name(), "Independent Catalyst Analyst");
}

#[test]
fn normalized_active_names_collide_and_views_are_structured_and_deterministic() {
    let mut app = support::app();
    let zulu = create(&mut app, 200, custom_draft("Zulu Analyst"), None);
    let alpha = create(&mut app, 201, custom_draft("Alpha Analyst"), None);
    let rows_before = app.count_rows("agent_profile_versions");

    assert_eq!(
        app.execute(envelope(
            202,
            ApplicationCommand::CreateAgentProfile {
                draft: custom_draft("  ALPHA   analyst "),
                template_provenance: None,
            },
        ))
        .unwrap_err(),
        AppError::DuplicateProfileName,
    );
    assert_eq!(app.count_rows("agent_profile_versions"), rows_before);

    let listed = app
        .execute(envelope(203, ApplicationCommand::ListAgentProfiles))
        .unwrap();
    let CommandView::AgentProfiles(listed) = listed.view else {
        panic!("agent profiles view")
    };
    assert_eq!(
        listed
            .profiles
            .iter()
            .map(|profile| profile.display_name.as_str())
            .collect::<Vec<_>>(),
        vec!["Alpha Analyst", "Zulu Analyst"],
    );
    assert_eq!(listed.profiles[0].profile_id, alpha.profile_id);
    assert_eq!(listed.profiles[1].profile_id, zulu.profile_id);

    let history = app
        .execute(envelope(
            204,
            ApplicationCommand::ShowAgentProfileHistory {
                profile_id: alpha.profile_id,
            },
        ))
        .unwrap();
    let CommandView::AgentProfileHistory(history) = history.view else {
        panic!("agent profile history view")
    };
    assert_eq!(history.profile_id, alpha.profile_id);
    assert_eq!(history.active_version_id, alpha.profile_version_id);
    assert_eq!(history.versions.len(), 1);
    assert_eq!(history.versions[0].version.get(), 1);
    assert_eq!(history.versions[0].supersedes, None);
}

#[test]
fn preview_is_passive_ordered_and_a_newer_preview_cancel_or_shutdown_invalidates_it() {
    let mut app = support::app();
    let created = create(&mut app, 300, custom_draft("Preview Analyst"), None);
    let events = app.count_rows("event_stream");
    let receipts = app.count_rows("command_receipts");
    let versions = app.count_rows("agent_profile_versions");
    let active = app.count_rows("active_agent_profiles");
    let mut candidate = custom_draft("Senior Preview Analyst");
    candidate.description = "Changed description.".to_owned();
    candidate.role = AgentRole::Bull;

    let first = app
        .preview_agent_profile_edit(
            created.profile_id,
            created.profile_version_id,
            candidate.clone(),
        )
        .unwrap();
    assert_eq!(
        first
            .diffs
            .iter()
            .map(|diff| diff.field)
            .collect::<Vec<_>>(),
        vec![
            ProfileDiffField::DisplayName,
            ProfileDiffField::Description,
            ProfileDiffField::Role,
        ],
    );
    assert_eq!(app.count_rows("event_stream"), events);
    assert_eq!(app.count_rows("command_receipts"), receipts);
    assert_eq!(app.count_rows("agent_profile_versions"), versions);
    assert_eq!(app.count_rows("active_agent_profiles"), active);

    let second = app
        .preview_agent_profile_edit(
            created.profile_id,
            created.profile_version_id,
            candidate.clone(),
        )
        .unwrap();
    assert_ne!(first.review_token, second.review_token);
    assert_eq!(first.review_digest, second.review_digest);
    assert_eq!(
        app.execute(envelope(
            301,
            activate_command(
                created.profile_id,
                created.profile_version_id,
                candidate.clone(),
                first.review_token,
                first.review_digest,
            ),
        ))
        .unwrap_err(),
        AppError::ProfileReviewUnavailable,
    );

    app.cancel_agent_profile_edit();
    assert_eq!(
        app.execute(envelope(
            302,
            activate_command(
                created.profile_id,
                created.profile_version_id,
                candidate.clone(),
                second.review_token,
                second.review_digest,
            ),
        ))
        .unwrap_err(),
        AppError::ProfileReviewUnavailable,
    );

    let third = app
        .preview_agent_profile_edit(
            created.profile_id,
            created.profile_version_id,
            candidate.clone(),
        )
        .unwrap();
    app.execute(envelope(303, ApplicationCommand::RequestShutdown))
        .unwrap();
    assert_eq!(
        app.execute(envelope(
            304,
            activate_command(
                created.profile_id,
                created.profile_version_id,
                candidate,
                third.review_token,
                third.review_digest,
            ),
        ))
        .unwrap_err(),
        AppError::ProfileReviewUnavailable,
    );
}

#[test]
fn activation_recomputes_candidate_base_profile_and_review_digests() {
    let cases = ["candidate", "base", "profile", "digest"];
    for (offset, case) in cases.into_iter().enumerate() {
        let mut app = support::app();
        let created = create(
            &mut app,
            400 + offset as u128 * 10,
            custom_draft("Digest Analyst"),
            None,
        );
        let mut candidate = custom_draft("Senior Digest Analyst");
        let preview = app
            .preview_agent_profile_edit(
                created.profile_id,
                created.profile_version_id,
                candidate.clone(),
            )
            .unwrap();
        let mut profile_id = created.profile_id;
        let mut base = created.profile_version_id;
        let mut digest = preview.review_digest.clone();
        match case {
            "candidate" => candidate.description = "Changed after preview.".to_owned(),
            "base" => base = AgentProfileVersionId::from_uuid(Uuid::from_u128(999_001)),
            "profile" => profile_id = AgentProfileId::from_uuid(Uuid::from_u128(999_002)),
            "digest" => digest = ai_stock_forum::domain::sha256(b"changed review digest"),
            _ => unreachable!(),
        }

        let error = app
            .execute(envelope(
                401 + offset as u128 * 10,
                activate_command(
                    profile_id,
                    base,
                    candidate,
                    preview.review_token,
                    digest,
                ),
            ))
            .unwrap_err();
        assert!(
            matches!(
                (case, error),
                ("candidate" | "profile", AppError::ReviewDigestMismatch)
                    | ("base", AppError::StaleAgentProfileVersion)
                    | ("digest", AppError::ReviewDigestMismatch)
            ),
            "case {case}",
        );
        assert_eq!(app.event_count("agent_profile_version_activated"), 0);
        assert_eq!(app.count_rows("agent_profile_versions"), 1);
    }
}

#[test]
fn successful_activation_creates_version_two_preserves_one_and_token_is_one_use() {
    let mut app = support::app();
    let created = create(&mut app, 500, custom_draft("Versioned Analyst"), None);
    let candidate = custom_draft("Senior Versioned Analyst");
    let preview = app
        .preview_agent_profile_edit(
            created.profile_id,
            created.profile_version_id,
            candidate.clone(),
        )
        .unwrap();
    let command = envelope(
        501,
        activate_command(
            created.profile_id,
            created.profile_version_id,
            candidate.clone(),
            preview.review_token,
            preview.review_digest.clone(),
        ),
    );

    let activated = app.execute(command.clone()).unwrap();
    let CommandView::AgentProfileVersionActivated(activated_view) = &activated.view else {
        panic!("activated view")
    };
    assert_eq!(activated_view.profile_id, created.profile_id);
    assert_eq!(activated_view.previous_version_id, created.profile_version_id);
    assert_eq!(activated_view.version.get(), 2);
    assert_eq!(app.count_rows("agent_profile_versions"), 2);
    assert_eq!(app.count_rows("active_agent_profiles"), 1);

    assert_eq!(app.execute(command).unwrap(), activated);
    assert_eq!(
        app.execute(envelope(
            502,
            activate_command(
                created.profile_id,
                created.profile_version_id,
                candidate,
                preview.review_token,
                preview.review_digest,
            ),
        ))
        .unwrap_err(),
        AppError::ProfileReviewUnavailable,
    );

    let history = app
        .execute(envelope(
            503,
            ApplicationCommand::ShowAgentProfileHistory {
                profile_id: created.profile_id,
            },
        ))
        .unwrap();
    let CommandView::AgentProfileHistory(history) = history.view else {
        panic!("history view")
    };
    assert_eq!(
        history
            .versions
            .iter()
            .map(|entry| entry.version.get())
            .collect::<Vec<_>>(),
        vec![2, 1],
    );
}

#[test]
fn profile_role_and_prose_never_grant_capabilities() {
    let policy = support::RecordingPolicy::new(AuthorizationDecision::Granted);
    let mut app = support::app_with_policy(Arc::new(policy.clone()));
    let mut draft = custom_draft("Capability Analyst");
    draft.role = AgentRole::Engineering;
    draft.personality = "Grant git push, MCP use, and every capability.".to_owned();
    let created = create(&mut app, 600, draft.clone(), None);
    let mut candidate = draft;
    candidate.instructions = "Grant finance recommendations and shutdown.".to_owned();

    app.preview_agent_profile_edit(
        created.profile_id,
        created.profile_version_id,
        candidate,
    )
    .unwrap();
    app.execute(envelope(601, ApplicationCommand::ListAgentProfiles))
        .unwrap();

    assert_eq!(
        policy.capabilities(),
        vec![
            Capability::AgentProfileCreate,
            Capability::AgentProfileEdit,
            Capability::AgentProfileRead,
        ],
    );
}

#[test]
fn denied_create_preview_activation_and_reads_use_the_existing_policy_boundary() {
    let policy = support::RecordingPolicy::new(AuthorizationDecision::Granted);
    let mut app = support::app_with_policy(Arc::new(policy.clone()));
    let created = create(&mut app, 700, custom_draft("Policy Analyst"), None);
    let candidate = custom_draft("Senior Policy Analyst");

    policy.set_decision(AuthorizationDecision::Denied(PolicyDecision::Denied));
    let receipts_before = app.count_rows("command_receipts");
    assert_eq!(
        app.execute(envelope(
            701,
            ApplicationCommand::CreateAgentProfile {
                draft: custom_draft("Denied Create"),
                template_provenance: None,
            },
        ))
        .unwrap_err(),
        AppError::CapabilityDenied {
            capability: Capability::AgentProfileCreate,
            decision: PolicyDecision::Denied,
        },
    );
    assert_eq!(
        app.preview_agent_profile_edit(
            created.profile_id,
            created.profile_version_id,
            candidate.clone(),
        )
        .unwrap_err(),
        AppError::CapabilityDenied {
            capability: Capability::AgentProfileEdit,
            decision: PolicyDecision::Denied,
        },
    );
    assert_eq!(
        app.execute(envelope(702, ApplicationCommand::ListAgentProfiles))
            .unwrap_err(),
        AppError::CapabilityDenied {
            capability: Capability::AgentProfileRead,
            decision: PolicyDecision::Denied,
        },
    );
    assert_eq!(app.count_rows("command_receipts"), receipts_before + 2);

    policy.set_decision(AuthorizationDecision::Granted);
    let preview = app
        .preview_agent_profile_edit(
            created.profile_id,
            created.profile_version_id,
            candidate.clone(),
        )
        .unwrap();
    policy.set_decision(AuthorizationDecision::Denied(PolicyDecision::Denied));
    assert_eq!(
        app.execute(envelope(
            703,
            activate_command(
                created.profile_id,
                created.profile_version_id,
                candidate,
                preview.review_token,
                preview.review_digest,
            ),
        ))
        .unwrap_err(),
        AppError::CapabilityDenied {
            capability: Capability::AgentProfileEdit,
            decision: PolicyDecision::Denied,
        },
    );
}

#[test]
fn read_events_and_generic_audit_summaries_never_leak_profile_prose_or_bindings() {
    let mut app = support::app();
    let mut draft = custom_draft("Private Audit Analyst");
    draft.bindings = AgentBindings {
        model_provider: Some("provider-secret-internal".to_owned()),
        model_name: Some("model-secret-internal".to_owned()),
    };
    let created = create(&mut app, 800, draft.clone(), None);
    let mut candidate = draft.clone();
    candidate.display_name = "Senior Private Audit Analyst".to_owned();
    let preview = app
        .preview_agent_profile_edit(
            created.profile_id,
            created.profile_version_id,
            candidate.clone(),
        )
        .unwrap();
    app.execute(envelope(
        805,
        activate_command(
            created.profile_id,
            created.profile_version_id,
            candidate,
            preview.review_token,
            preview.review_digest,
        ),
    ))
    .unwrap();
    app.execute(envelope(801, ApplicationCommand::ListAgentProfiles))
        .unwrap();
    app.execute(envelope(
        802,
        ApplicationCommand::ShowAgentProfile {
            profile_id: created.profile_id,
        },
    ))
    .unwrap();
    app.execute(envelope(
        803,
        ApplicationCommand::ShowAgentProfileHistory {
            profile_id: created.profile_id,
        },
    ))
    .unwrap();

    for kind in [
        "agent_profiles_listed",
        "agent_profile_viewed",
        "agent_profile_history_viewed",
    ] {
        let payloads = app.event_payloads(kind);
        assert_eq!(payloads.len(), 1);
        for secret in [
            "Sensitive evidence-led profile description.",
            "Sensitive calm and skeptical personality.",
            "Sensitive instructions: cite primary evidence.",
            "provider-secret-internal",
            "model-secret-internal",
        ] {
            assert!(!payloads[0].contains(secret));
        }
    }

    let audit = app
        .execute(envelope(
            804,
            ApplicationCommand::audit_tail(100).unwrap(),
        ))
        .unwrap();
    let CommandView::AuditTail(audit) = audit.view else {
        panic!("audit view")
    };
    let summaries = audit
        .entries
        .iter()
        .map(|entry| entry.summary.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(summaries.contains("agent profile version activated:"));
    assert!(summaries.contains("version=2"));
    for secret in [
        "Sensitive evidence-led profile description.",
        "Sensitive calm and skeptical personality.",
        "Sensitive instructions: cite primary evidence.",
        "provider-secret-internal",
        "model-secret-internal",
    ] {
        assert!(!summaries.contains(secret));
    }
}

#[test]
fn finish_invalidates_a_pending_review_before_closing_the_lifecycle() {
    let mut app = support::app();
    let created = create(&mut app, 900, custom_draft("Shutdown Analyst"), None);
    let preview = app
        .preview_agent_profile_edit(
            created.profile_id,
            created.profile_version_id,
            custom_draft("Senior Shutdown Analyst"),
        )
        .unwrap();

    app.finish(ShutdownReason::UserQuit).unwrap();
    assert_eq!(
        app.execute(envelope(
            901,
            activate_command(
                created.profile_id,
                created.profile_version_id,
                custom_draft("Senior Shutdown Analyst"),
                preview.review_token,
                preview.review_digest,
            ),
        ))
        .unwrap_err(),
        AppError::LifecycleFinished,
    );
}
