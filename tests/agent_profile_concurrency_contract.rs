mod support;

use std::{
    sync::{
        Arc, Barrier,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, RecvTimeoutError},
    },
    thread,
    time::Duration,
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

#[test]
fn folded_name_create_race_has_one_winner_one_conflict_and_no_loser_writes() {
    let mut app = support::app();
    let baseline_events = app.count_rows("event_stream");
    let baseline_receipts = app.count_rows("command_receipts");
    let start = Arc::new(Barrier::new(3));
    let mut first_worker = app.peer();
    let mut second_worker = app.peer();
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

    let first_start = start.clone();
    let first_command = first.clone();
    let first_thread = thread::spawn(move || {
        first_start.wait();
        first_worker.execute(first_command)
    });
    let second_start = start.clone();
    let second_command = second.clone();
    let second_thread = thread::spawn(move || {
        second_start.wait();
        second_worker.execute(second_command)
    });
    start.wait();

    let results = [
        (first.clone(), first_thread.join().unwrap()),
        (second.clone(), second_thread.join().unwrap()),
    ];
    assert_eq!(results.iter().filter(|(_, result)| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|(_, result)| matches!(result, Err(AppError::DuplicateProfileName)))
            .count(),
        1
    );

    let (winning_command, winning_outcome) = results
        .iter()
        .find_map(|(command, result)| result.as_ref().ok().map(|outcome| (command, outcome)))
        .unwrap();
    let losing_command = results
        .iter()
        .find_map(|(command, result)| result.as_ref().err().map(|_| command))
        .unwrap();
    assert_eq!(
        app.execute(winning_command.clone()).unwrap(),
        *winning_outcome
    );

    let mut changed = winning_command.clone();
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
    assert!(app.event_ref_rows(losing_command.command_id).is_empty());
    assert_eq!(app.persisted_last_sequence(), app.max_event_sequence());
}

struct BlockingOutcomeHook {
    armed: AtomicBool,
    entered: Barrier,
    release: Barrier,
}

impl BlockingOutcomeHook {
    fn new() -> Self {
        Self {
            armed: AtomicBool::new(false),
            entered: Barrier::new(2),
            release: Barrier::new(2),
        }
    }

    fn arm(&self) {
        self.armed.store(true, Ordering::SeqCst);
    }
}

impl CommandTransactionHook for BlockingOutcomeHook {
    fn before_outcome_materialization(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        if self.armed.swap(false, Ordering::SeqCst) {
            self.entered.wait();
            self.release.wait();
        }
        Ok(())
    }

    fn before_receipt_write(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }
}

#[test]
fn two_edits_from_one_base_linearize_to_version_two_then_stale_preview() {
    let hook = Arc::new(BlockingOutcomeHook::new());
    let mut app = support::app_with_policy_and_hook(
        Arc::new(support::RecordingPolicy::new(
            AuthorizationDecision::Granted,
        )),
        hook.clone(),
    );
    let (profile_id, base_version_id) = created_profile(&mut app, 20_000);
    let winning_candidate = draft("Winning Version Two Analyst");
    let preview = app
        .preview_agent_profile_edit(profile_id, base_version_id, winning_candidate.clone())
        .unwrap();
    let mut worker = app.peer();

    hook.arm();
    let activation_thread = thread::spawn(move || {
        worker.execute(envelope(
            20_001,
            activation(
                profile_id,
                base_version_id,
                winning_candidate,
                preview.review_token,
                preview.review_digest,
            ),
        ))
    });
    hook.entered.wait();

    let preview_started = Arc::new(Barrier::new(2));
    let preview_started_in_thread = preview_started.clone();
    let (preview_sender, preview_receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        preview_started_in_thread.wait();
        let result = app.preview_agent_profile_edit(
            profile_id,
            base_version_id,
            draft("Losing Stale Analyst"),
        );
        preview_sender.send((app, result)).unwrap();
    });
    preview_started.wait();

    assert!(matches!(
        preview_receiver.recv_timeout(Duration::from_millis(250)),
        Err(RecvTimeoutError::Timeout)
    ));
    hook.release.wait();
    let activated = activation_thread.join().unwrap().unwrap();
    let (app, stale) = preview_receiver.recv().unwrap();

    let CommandView::AgentProfileVersionActivated(view) = activated.view else {
        panic!("agent profile activation view")
    };
    assert_eq!(view.version.get(), 2);
    assert_eq!(stale.unwrap_err(), AppError::StaleAgentProfileVersion);
    assert_eq!(app.event_count("agent_profile_version_activated"), 1);
    assert_eq!(app.count_rows("agent_profile_versions"), 2);
    assert_eq!(app.count_rows("active_agent_profiles"), 1);
}

#[test]
fn one_review_token_has_one_activation_winner_and_receipt_first_replay() {
    let mut app = support::app();
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
    let start = Arc::new(Barrier::new(3));
    let first_start = start.clone();
    let first_command = first.clone();
    let first_thread = thread::spawn(move || {
        first_start.wait();
        first_worker.execute(first_command)
    });
    let second_start = start.clone();
    let second_command = second.clone();
    let second_thread = thread::spawn(move || {
        second_start.wait();
        second_worker.execute(second_command)
    });
    start.wait();

    let results = [
        (first, first_thread.join().unwrap()),
        (second, second_thread.join().unwrap()),
    ];
    assert_eq!(results.iter().filter(|(_, result)| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|(_, result)| matches!(result, Err(AppError::ProfileReviewUnavailable)))
            .count(),
        1
    );

    let (winning_command, winning_outcome) = results
        .iter()
        .find_map(|(command, result)| result.as_ref().ok().map(|outcome| (command, outcome)))
        .unwrap();
    let losing_command = results
        .iter()
        .find_map(|(command, result)| result.as_ref().err().map(|_| command))
        .unwrap();
    assert_eq!(
        app.execute(winning_command.clone()).unwrap(),
        *winning_outcome
    );

    let mut changed = winning_command.clone();
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
    assert!(app.event_ref_rows(losing_command.command_id).is_empty());
    assert_eq!(app.persisted_last_sequence(), app.max_event_sequence());
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
                     SELECT profile_id, profile_version_id, version, readiness
                     FROM active_agent_profiles LIMIT 1;
                 CREATE TRIGGER inject_active_name_conflict
                 BEFORE INSERT ON active_agent_profiles
                 WHEN NEW.normalized_name = 'alpha sqlite candidate'
                 BEGIN
                     INSERT INTO active_agent_profiles (
                         profile_id, profile_version_id, version, normalized_name, readiness
                     )
                     SELECT profile_id, profile_version_id, version,
                            'alpha sqlite candidate', readiness
                     FROM injected_active_name_source;
                 END;",
            )
            .map_err(|_| PersistenceError::QueryFailed)
    }
}

#[test]
fn sqlite_normalized_name_conflict_matches_reducer_precheck_and_rolls_back() {
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
        app.count_rows("active_agent_profiles"),
        app.count_rows("command_receipts"),
        app.count_rows("command_event_refs"),
        app.persisted_last_sequence(),
    );

    hook.arm();
    assert_eq!(
        app.execute(command.clone()).unwrap_err(),
        AppError::DuplicateProfileName
    );
    assert_eq!(
        (
            app.count_rows("event_stream"),
            app.count_rows("agent_profile_versions"),
            app.count_rows("active_agent_profiles"),
            app.count_rows("command_receipts"),
            app.count_rows("command_event_refs"),
            app.persisted_last_sequence(),
        ),
        before
    );
    assert!(app.event_ref_rows(command.command_id).is_empty());

    let retried = app.execute(command).unwrap();
    assert!(matches!(retried.view, CommandView::AgentProfileCreated(_)));
    assert_eq!(app.count_rows("agent_profile_versions"), before.1 + 1);
    assert_eq!(app.count_rows("active_agent_profiles"), before.2 + 1);
}
