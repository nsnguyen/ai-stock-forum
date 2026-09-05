mod support;

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use ai_stock_forum::{
    agents::{AgentBindings, AgentProfileDraft, AgentRole},
    app::{
        ApplicationCommand, AuthorizationDecision, CommandEnvelope, CommandTransactionHook,
        CommandView,
    },
    domain::{Actor, AgentProfileId, AgentProfileVersionId, CommandId, CorrelationId, Digest},
    persistence::PersistenceError,
};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureBoundary {
    AfterEventAppend,
    AfterProfileMirrorInsert,
    AfterActivePointerUpdate,
    AfterProjectionStore,
    AfterAuditAppend,
    AfterReceiptStore,
    BeforeCommit,
}

struct FailOnceHook {
    target: FailureBoundary,
    armed: AtomicBool,
    fired: AtomicBool,
}

impl FailOnceHook {
    fn new(target: FailureBoundary) -> Self {
        Self {
            target,
            armed: AtomicBool::new(false),
            fired: AtomicBool::new(false),
        }
    }

    fn arm(&self) {
        self.armed.store(true, Ordering::SeqCst);
    }

    fn inject(&self, boundary: FailureBoundary) -> Result<(), PersistenceError> {
        if self.target == boundary && self.armed.swap(false, Ordering::SeqCst) {
            self.fired.store(true, Ordering::SeqCst);
            Err(PersistenceError::QueryFailed)
        } else {
            Ok(())
        }
    }
}

impl CommandTransactionHook for FailOnceHook {
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

    fn after_event_append(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        self.inject(FailureBoundary::AfterEventAppend)
    }

    fn after_profile_mirror_insert(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        self.inject(FailureBoundary::AfterProfileMirrorInsert)
    }

    fn after_active_pointer_update(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        self.inject(FailureBoundary::AfterActivePointerUpdate)
    }

    fn after_projection_store(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        self.inject(FailureBoundary::AfterProjectionStore)
    }

    fn after_audit_append(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        self.inject(FailureBoundary::AfterAuditAppend)
    }

    fn after_receipt_store(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        self.inject(FailureBoundary::AfterReceiptStore)
    }

    fn before_commit(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        self.inject(FailureBoundary::BeforeCommit)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DurableSnapshot {
    events: i64,
    versions: i64,
    active: i64,
    receipts: i64,
    refs: i64,
    projected_sequence: u64,
    max_sequence: u64,
}

fn snapshot(app: &support::TestApp) -> DurableSnapshot {
    DurableSnapshot {
        events: app.count_rows("event_stream"),
        versions: app.count_rows("agent_profile_versions"),
        active: app.count_rows("active_agent_profiles"),
        receipts: app.count_rows("command_receipts"),
        refs: app.count_rows("command_event_refs"),
        projected_sequence: app.persisted_last_sequence(),
        max_sequence: app.max_event_sequence(),
    }
}

fn envelope(id: u128, command: ApplicationCommand) -> CommandEnvelope {
    CommandEnvelope {
        command_id: CommandId::from_uuid(Uuid::from_u128(id)),
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(id + 100_000)),
        actor: Actor::Human,
        command,
    }
}

fn draft(name: &str) -> AgentProfileDraft {
    AgentProfileDraft::new(
        name.to_owned(),
        "Atomic profile description.".to_owned(),
        AgentRole::Custom,
        "equity research".to_owned(),
        vec!["research".to_owned()],
        "Calm and skeptical personality.".to_owned(),
        "Cite evidence before making a claim.".to_owned(),
        AgentBindings::default(),
        Vec::new(),
        Vec::new(),
    )
    .unwrap()
}

fn create_profile(
    app: &mut support::TestApp,
    id: u128,
) -> (AgentProfileId, AgentProfileVersionId) {
    let outcome = app
        .execute(envelope(
            id,
            ApplicationCommand::CreateAgentProfile {
                draft: draft("Atomicity Base Analyst"),
                template_provenance: None,
            },
        ))
        .unwrap();
    let CommandView::AgentProfileCreated(view) = outcome.view else {
        panic!("agent profile created view")
    };
    (view.profile_id, view.profile_version_id)
}

fn activation(
    profile_id: AgentProfileId,
    base_version_id: AgentProfileVersionId,
    candidate: AgentProfileDraft,
    review_token: ai_stock_forum::domain::ProfileReviewToken,
    review_digest: Digest,
) -> ApplicationCommand {
    ApplicationCommand::ActivateAgentProfileVersion {
        profile_id,
        expected_active_version_id: base_version_id,
        candidate,
        review_token,
        review_digest,
    }
}

#[test]
fn every_injected_write_boundary_rolls_back_and_releases_the_review_for_retry() {
    for (offset, boundary) in [
        FailureBoundary::AfterEventAppend,
        FailureBoundary::AfterProfileMirrorInsert,
        FailureBoundary::AfterActivePointerUpdate,
        FailureBoundary::AfterProjectionStore,
        FailureBoundary::AfterAuditAppend,
        FailureBoundary::AfterReceiptStore,
        FailureBoundary::BeforeCommit,
    ]
    .into_iter()
    .enumerate()
    {
        let hook = Arc::new(FailOnceHook::new(boundary));
        let mut app = support::app_with_policy_and_hook(
            Arc::new(support::RecordingPolicy::new(
                AuthorizationDecision::Granted,
            )),
            hook.clone(),
        );
        let command_base = 40_000 + (offset as u128 * 100);
        let (profile_id, base_version_id) = create_profile(&mut app, command_base);
        let mut candidate = draft(&format!("Atomic Retry Analyst {offset}"));
        candidate.description = "credential=must-never-appear".to_owned();
        let preview = app
            .preview_agent_profile_edit(profile_id, base_version_id, candidate.clone())
            .unwrap();
        let command = envelope(
            command_base + 1,
            activation(
                profile_id,
                base_version_id,
                candidate,
                preview.review_token,
                preview.review_digest,
            ),
        );
        let before = snapshot(&app);

        hook.arm();
        let error = app.execute(command.clone()).unwrap_err();

        assert!(hook.fired.load(Ordering::SeqCst), "boundary {boundary:?}");
        assert_eq!(error.code(), "database_write_failed", "boundary {boundary:?}");
        assert!(
            !error.to_string().contains("credential=must-never-appear"),
            "boundary {boundary:?}"
        );
        assert_eq!(snapshot(&app), before, "boundary {boundary:?}");
        assert!(app.event_ref_rows(command.command_id).is_empty());

        let retried = app.execute(command).unwrap();
        let CommandView::AgentProfileVersionActivated(view) = retried.view else {
            panic!("agent profile activation view")
        };
        assert_eq!(view.version.get(), 2, "boundary {boundary:?}");
        let after = snapshot(&app);
        assert_eq!(after.events, before.events + 1, "boundary {boundary:?}");
        assert_eq!(after.versions, before.versions + 1, "boundary {boundary:?}");
        assert_eq!(after.active, before.active, "boundary {boundary:?}");
        assert_eq!(after.receipts, before.receipts + 1, "boundary {boundary:?}");
        assert_eq!(after.refs, before.refs + 1, "boundary {boundary:?}");
        assert_eq!(after.projected_sequence, before.projected_sequence + 1);
        assert_eq!(after.projected_sequence, after.max_sequence);
    }
}
