mod support;

use std::{
    sync::{
        Arc, Barrier,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use ai_stock_forum::{
    agents::{AgentBindings, AgentProfileDraft, AgentRole},
    app::{
        AppError, ApplicationCommand, AuthorizationDecision, CommandEnvelope,
        CommandTransactionHook, CommandView,
    },
    domain::{Actor, AgentProfileId, AgentProfileVersionId, CommandId, CorrelationId, Digest},
    persistence::PersistenceError,
};
use uuid::Uuid;

fn command_id(id: u128) -> CommandId {
    CommandId::from_uuid(Uuid::from_u128(id))
}

fn envelope(id: u128, command: ApplicationCommand) -> CommandEnvelope {
    CommandEnvelope {
        command_id: command_id(id),
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(id + 100_000)),
        actor: Actor::Human,
        command,
    }
}

fn draft(name: &str) -> AgentProfileDraft {
    AgentProfileDraft::new(
        name.to_owned(),
        "Evidence-led profile description.".to_owned(),
        AgentRole::Custom,
        "equity research".to_owned(),
        vec!["research".to_owned()],
        "Calm and skeptical personality.".to_owned(),
        "Cite primary evidence.".to_owned(),
        AgentBindings::default(),
        Vec::new(),
        Vec::new(),
    )
    .unwrap()
}

fn activation(
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

fn created_profile(
    app: &mut support::TestApp,
    id: u128,
) -> (AgentProfileId, AgentProfileVersionId) {
    let outcome = app
        .execute(envelope(
            id,
            ApplicationCommand::CreateAgentProfile {
                draft: draft("Concurrency Base Analyst"),
                template_provenance: None,
            },
        ))
        .unwrap();
    let CommandView::AgentProfileCreated(view) = outcome.view else {
        panic!("agent profile created view")
    };
    (view.profile_id, view.profile_version_id)
}

struct NameBoundaryHook {
    first: CommandId,
    second: CommandId,
    first_inside: AtomicBool,
    overlap_observed: AtomicBool,
    first_entered: Barrier,
    second_entered: Barrier,
    overlap_recorded: Barrier,
    release_first: Barrier,
    review_preflights: Barrier,
}

impl NameBoundaryHook {
    fn new(first: CommandId, second: CommandId) -> Self {
        Self {
            first,
            second,
            first_inside: AtomicBool::new(false),
            overlap_observed: AtomicBool::new(false),
            first_entered: Barrier::new(2),
            second_entered: Barrier::new(2),
            overlap_recorded: Barrier::new(2),
            release_first: Barrier::new(2),
            review_preflights: Barrier::new(2),
        }
    }

    fn wait_for_first_precheck(&self) {
        self.first_entered.wait();
    }

    fn wait_for_second_attempt(&self) {
        self.second_entered.wait();
        self.overlap_recorded.wait();
    }

    fn release_first(&self) {
        self.release_first.wait();
    }
}

impl CommandTransactionHook for NameBoundaryHook {
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

    fn before_profile_mutation_transaction(&self, contender: CommandId) {
        if contender == self.second {
            self.second_entered.wait();
            self.overlap_observed
                .store(self.first_inside.load(Ordering::SeqCst), Ordering::SeqCst);
            self.overlap_recorded.wait();
        }
    }

    fn before_profile_review_operation(&self, contender: CommandId) {
        if contender == self.first || contender == self.second {
            self.review_preflights.wait();
        }
    }

    fn after_profile_name_precheck(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
        contender: CommandId,
    ) -> Result<(), PersistenceError> {
        if contender == self.first {
            self.first_inside.store(true, Ordering::SeqCst);
            self.first_entered.wait();
            self.release_first.wait();
            self.first_inside.store(false, Ordering::SeqCst);
        }
        Ok(())
    }
}

#[test]
fn folded_name_create_race_overlaps_at_precheck_and_has_one_deterministic_winner() {
    let first_id = command_id(10_000);
    let second_id = command_id(10_001);
    let hook = Arc::new(NameBoundaryHook::new(first_id, second_id));
    let mut app = support::app_with_policy_and_hook(
        Arc::new(support::RecordingPolicy::new(
            AuthorizationDecision::Granted,
        )),
        hook.clone(),
    );
    let baseline_events = app.count_rows("event_stream");
    let baseline_receipts = app.count_rows("command_receipts");
    let first = envelope(
        10_000,
        ApplicationCommand::CreateAgentProfile {
            draft: draft("  Alpha   Analyst "),
            template_provenance: None,
        },
    );
    let second = envelope(
        10_001,
        ApplicationCommand::CreateAgentProfile {
            draft: draft("ALPHA analyst"),
            template_provenance: None,
        },
    );
    let mut first_worker = app.peer();
    let mut second_worker = app.peer();

    let first_command = first.clone();
    let first_thread = thread::spawn(move || first_worker.execute(first_command));
    hook.wait_for_first_precheck();
    let second_command = second.clone();
    let second_thread = thread::spawn(move || second_worker.execute(second_command));
    hook.wait_for_second_attempt();
    assert!(hook.overlap_observed.load(Ordering::SeqCst));
    hook.release_first();

    let winning_outcome = first_thread.join().unwrap().unwrap();
    let losing_error = second_thread.join().unwrap().unwrap_err();
    assert_eq!(losing_error, AppError::DuplicateProfileName);
    assert_eq!(losing_error.code(), "active_name_conflict");
    assert_eq!(app.execute(first.clone()).unwrap(), winning_outcome);

    let mut changed = first;
    changed.command = ApplicationCommand::CreateAgentProfile {
        draft: draft("Changed Payload Analyst"),
        template_provenance: None,
    };
    assert_eq!(app.execute(changed).unwrap_err(), AppError::CommandConflict);

    assert_eq!(app.event_count("agent_profile_created"), 1);
    assert_eq!(app.count_rows("event_stream"), baseline_events + 1);
    assert_eq!(app.count_rows("agent_profile_versions"), 1);
    assert_eq!(app.count_rows("active_agent_profiles"), 1);
    assert_eq!(app.count_rows("command_receipts"), baseline_receipts + 1);
    assert_eq!(app.count_rows("command_event_refs"), baseline_receipts + 1);
    assert!(app.event_ref_rows(second.command_id).is_empty());
    assert_eq!(app.persisted_last_sequence(), app.max_event_sequence());
}

#[test]
fn two_independent_services_edit_one_base_with_one_version_two_and_one_stale_loser() {
    let first_id = command_id(20_001);
    let second_id = command_id(20_002);
    let hook = Arc::new(NameBoundaryHook::new(first_id, second_id));
    let mut app = support::app_with_policy_and_hook(
        Arc::new(support::RecordingPolicy::new(
            AuthorizationDecision::Granted,
        )),
        hook.clone(),
    );
    let (profile_id, base_version_id) = created_profile(&mut app, 20_000);
    let mut first_service = app.independent_profile_instance().unwrap();
    let mut second_service = app.independent_profile_instance().unwrap();
    let first_candidate = draft("First Independent Edit");
    let second_candidate = draft("Second Independent Edit");
    let first_preview = first_service
        .preview_agent_profile_edit(profile_id, base_version_id, first_candidate.clone())
        .unwrap();
    let second_preview = second_service
        .preview_agent_profile_edit(profile_id, base_version_id, second_candidate.clone())
        .unwrap();
    let first = envelope(
        20_001,
        activation(
            profile_id,
            base_version_id,
            first_candidate,
            first_preview.review_token,
            first_preview.review_digest,
        ),
    );
    let second = envelope(
        20_002,
        activation(
            profile_id,
            base_version_id,
            second_candidate,
            second_preview.review_token,
            second_preview.review_digest,
        ),
    );
    let before_events = app.count_rows("event_stream");
    let before_receipts = app.count_rows("command_receipts");

    let second_for_thread = second.clone();
    let first_thread = thread::spawn(move || first_service.execute(first));
    let second_thread = thread::spawn(move || second_service.execute(second_for_thread));
    hook.wait_for_first_precheck();
    hook.wait_for_second_attempt();
    assert!(hook.overlap_observed.load(Ordering::SeqCst));
    hook.release_first();

    let first_outcome = first_thread.join().unwrap().unwrap();
    let second_error = second_thread.join().unwrap().unwrap_err();
    let CommandView::AgentProfileVersionActivated(view) = first_outcome.view else {
        panic!("agent profile activation view")
    };
    assert_eq!(view.version.get(), 2);
    assert_eq!(second_error, AppError::StaleAgentProfileVersion);
    assert_eq!(app.event_count("agent_profile_version_activated"), 1);
    assert_eq!(app.count_rows("event_stream"), before_events + 1);
    assert_eq!(app.count_rows("agent_profile_versions"), 2);
    assert_eq!(app.count_rows("active_agent_profiles"), 1);
    assert_eq!(app.count_rows("command_receipts"), before_receipts + 1);
    assert_eq!(app.count_rows("command_event_refs"), before_receipts + 1);
    assert!(app.event_ref_rows(second.command_id).is_empty());
    assert_eq!(app.persisted_last_sequence(), app.max_event_sequence());
}

struct ReviewReservationHook {
    first: CommandId,
    second: CommandId,
    reservation_inside: AtomicBool,
    overlap_observed: AtomicBool,
    first_before_operation: Barrier,
    second_before_operation: Barrier,
    allow_first_operation: Barrier,
    allow_second_operation: Barrier,
    first_reserved: Barrier,
    second_attempting: Barrier,
    release_first: Barrier,
}

impl ReviewReservationHook {
    fn new(first: CommandId, second: CommandId) -> Self {
        Self {
            first,
            second,
            reservation_inside: AtomicBool::new(false),
            overlap_observed: AtomicBool::new(false),
            first_before_operation: Barrier::new(2),
            second_before_operation: Barrier::new(2),
            allow_first_operation: Barrier::new(2),
            allow_second_operation: Barrier::new(2),
            first_reserved: Barrier::new(2),
            second_attempting: Barrier::new(2),
            release_first: Barrier::new(2),
        }
    }
}

impl CommandTransactionHook for ReviewReservationHook {
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

    fn before_profile_review_operation(&self, contender: CommandId) {
        if contender == self.first {
            self.first_before_operation.wait();
            self.allow_first_operation.wait();
        } else if contender == self.second {
            self.second_before_operation.wait();
            self.allow_second_operation.wait();
            self.overlap_observed.store(
                self.reservation_inside.load(Ordering::SeqCst),
                Ordering::SeqCst,
            );
            self.second_attempting.wait();
        }
    }

    fn after_profile_review_reservation(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
        contender: CommandId,
    ) -> Result<(), PersistenceError> {
        if contender == self.first {
            self.reservation_inside.store(true, Ordering::SeqCst);
            self.first_reserved.wait();
            self.release_first.wait();
            self.reservation_inside.store(false, Ordering::SeqCst);
        }
        Ok(())
    }
}

#[test]
fn one_review_token_overlaps_at_reservation_and_has_one_deterministic_winner() {
    let first_id = command_id(30_001);
    let second_id = command_id(30_002);
    let hook = Arc::new(ReviewReservationHook::new(first_id, second_id));
    let mut app = support::app_with_policy_and_hook(
        Arc::new(support::RecordingPolicy::new(
            AuthorizationDecision::Granted,
        )),
        hook.clone(),
    );
    let (profile_id, base_version_id) = created_profile(&mut app, 30_000);
    let candidate = draft("Single Token Winner Analyst");
    let preview = app
        .preview_agent_profile_edit(profile_id, base_version_id, candidate.clone())
        .unwrap();
    let first = envelope(
        30_001,
        activation(
            profile_id,
            base_version_id,
            candidate.clone(),
            preview.review_token,
            preview.review_digest.clone(),
        ),
    );
    let second = envelope(
        30_002,
        activation(
            profile_id,
            base_version_id,
            candidate,
            preview.review_token,
            preview.review_digest,
        ),
    );
    let mut first_worker = app.peer();
    let mut second_worker = app.peer();

    let first_command = first.clone();
    let first_thread = thread::spawn(move || first_worker.execute(first_command));
    hook.first_before_operation.wait();
    let second_command = second.clone();
    let second_thread = thread::spawn(move || second_worker.execute(second_command));
    hook.second_before_operation.wait();
    hook.allow_first_operation.wait();
    hook.first_reserved.wait();
    hook.allow_second_operation.wait();
    hook.second_attempting.wait();
    assert!(hook.overlap_observed.load(Ordering::SeqCst));
    hook.release_first.wait();

    let winning_outcome = first_thread.join().unwrap().unwrap();
    assert_eq!(
        second_thread.join().unwrap().unwrap_err(),
        AppError::ProfileReviewUnavailable
    );
    assert_eq!(app.execute(first.clone()).unwrap(), winning_outcome);

    let mut changed = first;
    let ApplicationCommand::ActivateAgentProfileVersion { candidate, .. } = &mut changed.command
    else {
        panic!("activation command")
    };
    candidate.description = "Changed payload after receipt commit.".to_owned();
    assert_eq!(app.execute(changed).unwrap_err(), AppError::CommandConflict);
    assert_eq!(app.event_count("agent_profile_version_activated"), 1);
    assert_eq!(app.count_rows("agent_profile_versions"), 2);
    assert_eq!(app.count_rows("active_agent_profiles"), 1);
    assert_eq!(app.count_rows("command_receipts"), 2);
    assert_eq!(app.count_rows("command_event_refs"), 2);
    assert!(app.event_ref_rows(second.command_id).is_empty());
}

struct SqliteNameConflictHook {
    armed: AtomicBool,
}

impl SqliteNameConflictHook {
    fn new() -> Self {
        Self {
            armed: AtomicBool::new(false),
        }
    }

    fn arm(&self) {
        self.armed.store(true, Ordering::SeqCst);
    }
}

impl CommandTransactionHook for SqliteNameConflictHook {
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

    fn after_profile_mirror_insert(
        &self,
        transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        if !self.armed.swap(false, Ordering::SeqCst) {
            return Ok(());
        }
        transaction
            .execute_batch(
                "CREATE TABLE injected_active_name_source AS
                     SELECT profile_id, profile_version_id, version, content_digest
                     FROM active_agent_profiles LIMIT 1;
                 CREATE TRIGGER inject_active_name_conflict
                 BEFORE INSERT ON active_agent_profiles
                 WHEN NEW.normalized_name = 'alpha sqlite candidate'
                 BEGIN
                     INSERT INTO active_agent_profiles (
                         profile_id, profile_version_id, version, normalized_name, content_digest
                     )
                     SELECT profile_id, profile_version_id, version,
                            'alpha sqlite candidate', content_digest
                     FROM injected_active_name_source;
                 END;",
            )
            .map_err(|_| PersistenceError::QueryFailed)
    }
}

#[test]
fn sqlite_normalized_name_conflict_has_the_same_stable_code_and_rolls_back() {
    let hook = Arc::new(SqliteNameConflictHook::new());
    let mut app = support::app_with_policy_and_hook(
        Arc::new(support::RecordingPolicy::new(
            AuthorizationDecision::Granted,
        )),
        hook.clone(),
    );
    created_profile(&mut app, 35_000);
    let command = envelope(
        35_001,
        ApplicationCommand::CreateAgentProfile {
            draft: draft("Alpha SQLite Candidate"),
            template_provenance: None,
        },
    );
    let before = (
        app.count_rows("event_stream"),
        app.count_rows("agent_profile_versions"),
        app.active_profile_rows(),
        app.count_rows("command_receipts"),
        app.count_rows("command_event_refs"),
        app.projection_metadata_row(),
    );

    hook.arm();
    let error = app.execute(command.clone()).unwrap_err();
    assert_eq!(error, AppError::DuplicateProfileName);
    assert_eq!(error.code(), "active_name_conflict");
    assert_eq!(
        (
            app.count_rows("event_stream"),
            app.count_rows("agent_profile_versions"),
            app.active_profile_rows(),
            app.count_rows("command_receipts"),
            app.count_rows("command_event_refs"),
            app.projection_metadata_row(),
        ),
        before
    );
    assert!(app.event_ref_rows(command.command_id).is_empty());

    let retried = app.execute(command).unwrap();
    assert!(matches!(retried.view, CommandView::AgentProfileCreated(_)));
    assert_eq!(app.count_rows("agent_profile_versions"), before.1 + 1);
    assert_eq!(app.active_profile_rows().len(), before.2.len() + 1);
}
