mod support;

use ai_stock_forum::{
    agents::{AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentRole},
    app::{
        AgentProfileSelector, ApplicationCommand, AuthorizationDecision, CommandEnvelope,
        CommandTransactionHook, CommandView, MemoryEditPreview,
    },
    domain::{Actor, CommandId, CorrelationId},
    memory::{
        ExpectedMemoryEntryState, MemoryEditReview, MemoryEntryDraft, MemoryMutationKind,
        MemoryProposalOperation, MemoryProposalRef, MemoryProposalStatus,
    },
    persistence::{MemoryRepository, PersistenceError},
    policy::ApprovalStatus,
};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

struct MemoryHookSurface;

impl CommandTransactionHook for MemoryHookSurface {
    fn before_memory_review_operation(&self, _command_id: CommandId) {}

    fn after_memory_review_reservation(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
        _command_id: CommandId,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }

    fn after_memory_proposal_insert(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }

    fn after_memory_approval_write(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }

    fn after_memory_entry_insert(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }

    fn after_memory_resolution_insert(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }

    fn after_memory_current_update(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryFaultBoundary {
    ReviewReservation,
    EventAppend,
    ProposalInsert,
    ApprovalWrite,
    EntryInsert,
    ResolutionInsert,
    CurrentStateUpdate,
    ProjectionStore,
    AuditAppend,
    ReceiptStore,
    PreCommit,
    SqliteCapacity,
}

impl MemoryFaultBoundary {
    pub const ALL: &'static [Self] = &[
        Self::ReviewReservation,
        Self::EventAppend,
        Self::ProposalInsert,
        Self::ApprovalWrite,
        Self::EntryInsert,
        Self::ResolutionInsert,
        Self::CurrentStateUpdate,
        Self::ProjectionStore,
        Self::AuditAppend,
        Self::ReceiptStore,
        Self::PreCommit,
        Self::SqliteCapacity,
    ];
}

struct MemoryFaultHook {
    next: Mutex<Option<(MemoryFaultBoundary, usize)>>,
    restore_page_limit: Mutex<bool>,
}

impl MemoryFaultHook {
    fn new() -> Self {
        Self {
            next: Mutex::new(None),
            restore_page_limit: Mutex::new(false),
        }
    }

    fn fail_once_at(&self, boundary: MemoryFaultBoundary) {
        self.fail_at_occurrence(boundary, 1);
    }

    fn fail_at_occurrence(&self, boundary: MemoryFaultBoundary, occurrence: usize) {
        assert!(occurrence > 0);
        *self.next.lock().unwrap() = Some((boundary, occurrence));
    }

    fn inject(&self, boundary: MemoryFaultBoundary) -> Result<(), PersistenceError> {
        let mut next = self.next.lock().unwrap();
        if let Some((armed, occurrence)) = next.as_mut()
            && *armed == boundary
        {
            if *occurrence > 1 {
                *occurrence -= 1;
                return Ok(());
            }
            *next = None;
            Err(if boundary == MemoryFaultBoundary::SqliteCapacity {
                PersistenceError::Capacity
            } else {
                PersistenceError::QueryFailed
            })
        } else {
            Ok(())
        }
    }
}

impl CommandTransactionHook for MemoryFaultHook {
    fn after_memory_review_reservation(
        &self,
        transaction: &rusqlite::Transaction<'_>,
        _command_id: CommandId,
    ) -> Result<(), PersistenceError> {
        let mut restore = self.restore_page_limit.lock().unwrap();
        if *restore {
            transaction
                .pragma_update(None, "max_page_count", 1_073_741_823_i64)
                .map_err(|_| PersistenceError::QueryFailed)?;
            *restore = false;
        }
        self.inject(MemoryFaultBoundary::ReviewReservation)
    }

    fn after_memory_proposal_insert(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        self.inject(MemoryFaultBoundary::ProposalInsert)
    }

    fn after_memory_approval_write(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        self.inject(MemoryFaultBoundary::ApprovalWrite)
    }

    fn after_memory_entry_insert(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        self.inject(MemoryFaultBoundary::EntryInsert)
    }

    fn after_memory_resolution_insert(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        self.inject(MemoryFaultBoundary::ResolutionInsert)
    }

    fn after_memory_current_update(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        self.inject(MemoryFaultBoundary::CurrentStateUpdate)
    }

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
        transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        self.inject(MemoryFaultBoundary::EventAppend)?;
        let mut next = self.next.lock().unwrap();
        if matches!(*next, Some((MemoryFaultBoundary::SqliteCapacity, 1))) {
            let pages: i64 = transaction
                .pragma_query_value(None, "page_count", |row| row.get(0))
                .map_err(|_| PersistenceError::QueryFailed)?;
            transaction
                .pragma_update(None, "max_page_count", pages)
                .map_err(|_| PersistenceError::QueryFailed)?;
            *next = None;
            *self.restore_page_limit.lock().unwrap() = true;
        }
        Ok(())
    }

    fn after_projection_store(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        self.inject(MemoryFaultBoundary::ProjectionStore)
    }

    fn after_audit_append(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        self.inject(MemoryFaultBoundary::AuditAppend)
    }

    fn after_receipt_store(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        self.inject(MemoryFaultBoundary::ReceiptStore)
    }

    fn before_commit(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        self.inject(MemoryFaultBoundary::PreCommit)
    }
}

fn envelope(id: u128, command: ApplicationCommand) -> CommandEnvelope {
    envelope_as(id, Actor::Human, command)
}

fn envelope_as(id: u128, actor: Actor, command: ApplicationCommand) -> CommandEnvelope {
    CommandEnvelope {
        command_id: CommandId::from_uuid(Uuid::from_u128(id)),
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(id + 100_000)),
        actor,
        command,
    }
}

fn create_profile(app: &mut support::TestApp, id: u128) -> AgentProfileVersion {
    app.execute(envelope(
        id,
        ApplicationCommand::CreateAgentProfile {
            draft: AgentProfileDraft::new(
                format!("Atomic memory {id}"),
                "Atomicity fixture.".into(),
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
        .find(|profile| profile.display_name() == format!("Atomic memory {id}"))
        .unwrap()
}

fn reviewed_set_with_value(
    app: &support::TestApp,
    profile: &AgentProfileVersion,
    value: String,
) -> MemoryEditReview {
    match app
        .preview_memory_set(
            AgentProfileSelector::from(profile.profile_id()),
            MemoryEntryDraft::new("Thesis".into(), value, vec!["atomic".into()]).unwrap(),
        )
        .unwrap()
    {
        MemoryEditPreview::Review(review) => review,
        other => panic!("expected review, got {other:?}"),
    }
}

fn set_command(review: &MemoryEditReview) -> ApplicationCommand {
    assert_eq!(review.operation, MemoryMutationKind::Set);
    assert_eq!(review.expected, ExpectedMemoryEntryState::Absent);
    ApplicationCommand::SetMemoryEntry {
        profile: review.profile.clone(),
        expected: review.expected.clone(),
        candidate: review.candidate.clone().unwrap(),
        review_token: review.review_token,
        review_digest: review.review_digest.clone(),
    }
}

fn proposal_command(profile: &AgentProfileVersion, value: &str) -> ApplicationCommand {
    proposal_command_for(profile, "Thesis", value)
}

fn proposal_command_for(
    profile: &AgentProfileVersion,
    key: &str,
    value: &str,
) -> ApplicationCommand {
    ApplicationCommand::ProposeMemoryMutation {
        proposer: profile.reference(),
        expected: ExpectedMemoryEntryState::Absent,
        operation: MemoryProposalOperation::Set {
            candidate: MemoryEntryDraft::new(key.into(), value.into(), vec!["proposal".into()])
                .unwrap(),
        },
        rationale: "Fresh evidence.".into(),
    }
}

fn proposal_from(outcome: ai_stock_forum::app::CommandOutcome) -> MemoryProposalRef {
    match outcome.view {
        CommandView::MemoryProposalCreated(view) => view.proposal,
        other => panic!("expected proposal view, got {other:?}"),
    }
}

#[test]
fn memory_fault_hooks_and_raw_snapshot_extend_the_existing_transaction_seam() {
    let fixture = support::persistent_fixture();
    let before = fixture.raw_database_snapshot();
    let after = fixture.raw_database_snapshot();
    assert_eq!(before, after);

    let hook: &dyn CommandTransactionHook = &MemoryHookSurface;
    hook.before_memory_review_operation(CommandId::from_uuid(uuid::Uuid::nil()));
}

#[test]
fn reviewed_memory_edit_rolls_back_and_releases_at_every_direct_write_boundary() {
    for (index, boundary) in [
        MemoryFaultBoundary::ReviewReservation,
        MemoryFaultBoundary::EventAppend,
        MemoryFaultBoundary::EntryInsert,
        MemoryFaultBoundary::CurrentStateUpdate,
        MemoryFaultBoundary::ProjectionStore,
        MemoryFaultBoundary::AuditAppend,
        MemoryFaultBoundary::ReceiptStore,
        MemoryFaultBoundary::PreCommit,
        MemoryFaultBoundary::SqliteCapacity,
    ]
    .into_iter()
    .enumerate()
    {
        let policy = Arc::new(support::RecordingPolicy::new(
            AuthorizationDecision::Granted,
        ));
        let hook = Arc::new(MemoryFaultHook::new());
        let mut app = support::app_with_policy_and_hook(policy, hook.clone());
        let profile = create_profile(&mut app, 900_000 + index as u128 * 1_000);
        let review = reviewed_set_with_value(
            &app,
            &profile,
            if boundary == MemoryFaultBoundary::SqliteCapacity {
                "x".repeat(4_096)
            } else {
                "Durable value.".into()
            },
        );
        let command = set_command(&review);
        let before = app.raw_database_snapshot();

        hook.fail_once_at(boundary);
        assert_eq!(
            app.execute(envelope(900_100 + index as u128 * 1_000, command.clone())),
            Err(ai_stock_forum::app::AppError::Persistence(
                if boundary == MemoryFaultBoundary::SqliteCapacity {
                    PersistenceError::Capacity
                } else {
                    PersistenceError::QueryFailed
                }
            )),
            "boundary {boundary:?}",
        );
        assert_eq!(app.raw_database_snapshot(), before, "boundary {boundary:?}");
        assert!(
            app.execute(envelope(900_100 + index as u128 * 1_000, command))
                .is_ok(),
            "exact review was not released at {boundary:?}",
        );
    }
}

#[test]
fn proposal_creation_rolls_back_at_its_immutable_and_approval_boundaries() {
    for (index, boundary) in [
        MemoryFaultBoundary::ProposalInsert,
        MemoryFaultBoundary::ApprovalWrite,
    ]
    .into_iter()
    .enumerate()
    {
        let policy = Arc::new(support::RecordingPolicy::new(
            AuthorizationDecision::Granted,
        ));
        let hook = Arc::new(MemoryFaultHook::new());
        let mut app = support::app_with_policy_and_hook(policy, hook.clone());
        let profile = create_profile(&mut app, 920_000 + index as u128 * 1_000);
        let request = envelope_as(
            920_100 + index as u128 * 1_000,
            Actor::Agent(profile.profile_id()),
            proposal_command(&profile, "Proposed value."),
        );
        let before = app.raw_database_snapshot();

        hook.fail_once_at(boundary);
        assert_eq!(
            app.execute(request.clone()),
            Err(ai_stock_forum::app::AppError::Persistence(
                PersistenceError::QueryFailed
            )),
            "boundary {boundary:?}",
        );
        assert_eq!(app.raw_database_snapshot(), before, "boundary {boundary:?}");
        assert!(app.execute(request).is_ok(), "boundary {boundary:?}");
    }
}

#[test]
fn proposal_acceptance_rolls_back_and_releases_at_each_resolution_boundary() {
    for (index, boundary) in [
        MemoryFaultBoundary::ApprovalWrite,
        MemoryFaultBoundary::ResolutionInsert,
        MemoryFaultBoundary::CurrentStateUpdate,
    ]
    .into_iter()
    .enumerate()
    {
        let policy = Arc::new(support::RecordingPolicy::new(
            AuthorizationDecision::Granted,
        ));
        let hook = Arc::new(MemoryFaultHook::new());
        let mut app = support::app_with_policy_and_hook(policy, hook.clone());
        let profile = create_profile(&mut app, 940_000 + index as u128 * 1_000);
        let proposal = proposal_from(
            app.execute(envelope_as(
                940_100 + index as u128 * 1_000,
                Actor::Agent(profile.profile_id()),
                proposal_command(&profile, "Accepted value."),
            ))
            .unwrap(),
        );
        let review = app
            .preview_memory_proposal_approval(proposal.clone())
            .unwrap();
        let request = envelope(
            940_200 + index as u128 * 1_000,
            ApplicationCommand::ApproveMemoryProposal {
                proposal,
                approval_id: review.approval_id,
                expected_approval_status: review.expected_approval_status,
                expected_entry: review.expected_entry,
                review_token: review.review_token,
                review_digest: review.review_digest,
            },
        );
        let before = app.raw_database_snapshot();

        hook.fail_once_at(boundary);
        assert_eq!(
            app.execute(request.clone()),
            Err(ai_stock_forum::app::AppError::Persistence(
                PersistenceError::QueryFailed
            )),
            "boundary {boundary:?}",
        );
        assert_eq!(app.raw_database_snapshot(), before, "boundary {boundary:?}");
        assert!(
            app.execute(request).is_ok(),
            "exact resolution review was not released at {boundary:?}",
        );
    }
}

#[test]
fn failure_after_primary_and_sibling_resolution_rolls_back_and_preserves_unrelated_approval() {
    let policy = Arc::new(support::RecordingPolicy::new(
        AuthorizationDecision::Granted,
    ));
    let hook = Arc::new(MemoryFaultHook::new());
    let mut app = support::app_with_policy_and_hook(policy, hook.clone());
    let profile = create_profile(&mut app, 960_000);
    let mut create = |id, key, value| {
        proposal_from(
            app.execute(envelope_as(
                id,
                Actor::Agent(profile.profile_id()),
                proposal_command_for(&profile, key, value),
            ))
            .unwrap(),
        )
    };
    let selected = create(960_100, "Shared key", "Selected value.");
    let sibling_one = create(960_101, "Shared key", "Sibling one.");
    let sibling_two = create(960_102, "Shared key", "Sibling two.");
    let unrelated = create(960_103, "Unrelated key", "Unrelated value.");
    let review = app
        .preview_memory_proposal_approval(selected.clone())
        .unwrap();
    let request = envelope(
        960_200,
        ApplicationCommand::ApproveMemoryProposal {
            proposal: selected,
            approval_id: review.approval_id,
            expected_approval_status: review.expected_approval_status,
            expected_entry: review.expected_entry,
            review_token: review.review_token,
            review_digest: review.review_digest,
        },
    );
    let before = app.raw_database_snapshot();

    hook.fail_at_occurrence(MemoryFaultBoundary::ResolutionInsert, 3);
    assert_eq!(
        app.execute(request.clone()),
        Err(ai_stock_forum::app::AppError::Persistence(
            PersistenceError::QueryFailed
        ))
    );
    assert_eq!(app.raw_database_snapshot(), before);

    let outcome = app.execute(request).unwrap();
    let CommandView::MemoryProposalResolution(view) = outcome.view else {
        panic!("proposal resolution view")
    };
    let mut expected_expired = vec![sibling_one, sibling_two];
    expected_expired.sort_by_key(|proposal| proposal.proposal_id());
    assert_eq!(view.expired_proposals, expected_expired);

    let mut database = app.open_database();
    let tx = database.immediate_transaction().unwrap();
    let (proposal, status, resolution) =
        MemoryRepository::load_proposal(&tx, unrelated.proposal_id())
            .unwrap()
            .unwrap();
    assert_eq!(status, MemoryProposalStatus::Pending);
    assert_eq!(resolution, None);
    let approval = MemoryRepository::load_memory_approval(&tx, proposal.approval_id())
        .unwrap()
        .unwrap();
    assert_eq!(approval.status(), ApprovalStatus::Pending);
    tx.commit().unwrap();
}
