mod support;

use ai_stock_forum::{
    agents::{AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentRole},
    app::{
        AgentProfileSelector, ApplicationCommand, CommandEnvelope, CommandView, MemoryEditPreview,
    },
    audit::AuditEntry,
    domain::{Actor, CommandId, CorrelationId},
    memory::MemoryEntryDraft,
};
use uuid::Uuid;

fn envelope(id: u128, command: ApplicationCommand) -> CommandEnvelope {
    CommandEnvelope {
        command_id: CommandId::from_uuid(Uuid::from_u128(id)),
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(id + 200_000)),
        actor: Actor::Human,
        command,
    }
}

fn create_profile(app: &mut support::TestApp, id: u128) -> AgentProfileVersion {
    app.execute(envelope(
        id,
        ApplicationCommand::CreateAgentProfile {
            draft: AgentProfileDraft::new(
                format!("Audit Editor {id}"),
                "Audit contract profile.".into(),
                AgentRole::Custom,
                "research".into(),
                vec![],
                "Careful.".into(),
                "Use evidence.".into(),
                AgentBindings::default(),
                vec![],
                vec![],
            )
            .unwrap(),
            template_provenance: None,
        },
    ))
    .unwrap();
    app.projection()
        .agent_profiles
        .active_profiles()
        .into_iter()
        .find(|profile| profile.display_name() == format!("Audit Editor {id}"))
        .unwrap()
}

#[test]
fn direct_memory_audit_contains_only_identifiers_states_and_counts() {
    let mut app = support::app();
    let profile = create_profile(&mut app, 72_000);
    let draft = MemoryEntryDraft::new(
        "sensitive-key-label".into(),
        "sensitive synthetic memory prose".into(),
        vec!["sensitive-purpose-tag".into()],
    )
    .unwrap();
    let review = match app
        .preview_memory_set(AgentProfileSelector::from(profile.profile_id()), draft)
        .unwrap()
    {
        MemoryEditPreview::Review(review) => review,
        other => panic!("expected review, got {other:?}"),
    };
    let outcome = app
        .execute(envelope(
            72_500,
            ApplicationCommand::SetMemoryEntry {
                profile: review.profile,
                expected: review.expected,
                candidate: review.candidate.unwrap(),
                review_token: review.review_token,
                review_digest: review.review_digest,
            },
        ))
        .unwrap();

    let current = match &outcome.view {
        CommandView::MemoryEntryMutation(view) => view.entry.clone(),
        other => panic!("unexpected view {other:?}"),
    };
    let set_audit = AuditEntry::from_event(&outcome.committed_events[0]);
    assert_eq!(set_audit.kind, "memory_entry_set");
    assert!(set_audit.summary.contains("entry_version="));
    assert!(set_audit.summary.contains("version=1"));
    assert!(set_audit.summary.contains("expired_count=0"));

    let delete = match app
        .preview_memory_delete(
            AgentProfileSelector::from(profile.profile_id()),
            "sensitive-key-label".into(),
        )
        .unwrap()
    {
        MemoryEditPreview::Review(review) => review,
        other => panic!("expected review, got {other:?}"),
    };
    assert_eq!(
        delete.expected,
        ai_stock_forum::memory::ExpectedMemoryEntryState::Present(current.clone())
    );
    let deleted = app
        .execute(envelope(
            72_600,
            ApplicationCommand::DeleteMemoryEntry {
                profile: delete.profile,
                expected: current,
                review_token: delete.review_token,
                review_digest: delete.review_digest,
            },
        ))
        .unwrap();
    let delete_audit = AuditEntry::from_event(&deleted.committed_events[0]);
    assert_eq!(delete_audit.kind, "memory_entry_deleted");
    assert!(delete_audit.summary.contains("entry_version="));
    assert!(delete_audit.summary.contains("version=2"));
    assert!(delete_audit.summary.contains("expired_count=0"));

    for audit in [&set_audit, &delete_audit] {
        for secret in [
            "sensitive-key-label",
            "sensitive synthetic memory prose",
            "sensitive-purpose-tag",
        ] {
            assert!(!audit.summary.contains(secret));
        }
    }
}
