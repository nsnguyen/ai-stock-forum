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
    domain::{AgentProfileId, AgentProfileVersionId, CommandId, CorrelationId, Digest},
    persistence::PersistenceError,
};
use uuid::Uuid;

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

    fn wait_until_blocked(&self) {
        self.entered.wait();
    }

    fn unblock(&self) {
        self.release.wait();
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

fn envelope(id: u128, command: ApplicationCommand) -> CommandEnvelope {
    CommandEnvelope {
        command_id: CommandId::from_uuid(Uuid::from_u128(id)),
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(id + 10_000)),
        actor: ai_stock_forum::domain::Actor::Human,
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

fn create_profile(
    app: &mut support::TestApp,
    id: u128,
) -> (AgentProfileId, AgentProfileVersionId) {
    let outcome = app
        .execute(envelope(
            id,
            ApplicationCommand::CreateAgentProfile {
                draft: draft("Concurrency Analyst"),
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

fn race_app(hook: Arc<BlockingOutcomeHook>) -> support::TestApp {
    support::app_with_policy_and_hook(
        Arc::new(support::RecordingPolicy::new(
            AuthorizationDecision::Granted,
        )),
        hook,
    )
}

#[test]
fn cancel_waits_for_reserved_activation_and_cannot_claim_its_review() {
    let hook = Arc::new(BlockingOutcomeHook::new());
    let mut app = race_app(hook.clone());
    let (profile_id, base_version_id) = create_profile(&mut app, 1_000);
    let candidate = draft("Activated Analyst");
    let preview = app
        .preview_agent_profile_edit(profile_id, base_version_id, candidate.clone())
        .unwrap();
    let mut worker = app.peer();

    hook.arm();
    let activation_thread = thread::spawn(move || {
        worker.execute(envelope(
            1_001,
            activation(
                profile_id,
                base_version_id,
                candidate,
                preview.review_token,
                preview.review_digest,
            ),
        ))
    });
    hook.wait_until_blocked();

    let cancel_started = Arc::new(Barrier::new(2));
    let cancel_started_in_thread = cancel_started.clone();
    let (cancel_done_tx, cancel_done_rx) = mpsc::sync_channel(1);
    thread::spawn(move || {
        cancel_started_in_thread.wait();
        app.cancel_agent_profile_edit().unwrap();
        cancel_done_tx.send(app).unwrap();
    });
    cancel_started.wait();

    let canceled_before_commit = match cancel_done_rx.recv_timeout(Duration::from_millis(250)) {
        Ok(app) => Some(app),
        Err(RecvTimeoutError::Timeout) => None,
        Err(error) => panic!("cancel channel failed: {error}"),
    };
    let cancel_completed_before_commit = canceled_before_commit.is_some();
    hook.unblock();
    let activated = activation_thread.join().unwrap().unwrap();
    let app = match canceled_before_commit {
        Some(app) => app,
        None => cancel_done_rx.recv().unwrap(),
    };

    assert!(!cancel_completed_before_commit);
    assert!(matches!(
        activated.view,
        CommandView::AgentProfileVersionActivated(_)
    ));
    assert_eq!(app.count_rows("agent_profile_versions"), 2);
}

#[test]
fn old_base_preview_waits_for_activation_then_fails_without_installing_a_token() {
    let hook = Arc::new(BlockingOutcomeHook::new());
    let mut app = race_app(hook.clone());
    let (profile_id, base_version_id) = create_profile(&mut app, 1_100);
    let candidate = draft("Activated Before Preview Analyst");
    let preview = app
        .preview_agent_profile_edit(profile_id, base_version_id, candidate.clone())
        .unwrap();
    let mut worker = app.peer();

    hook.arm();
    let activation_thread = thread::spawn(move || {
        worker.execute(envelope(
            1_101,
            activation(
                profile_id,
                base_version_id,
                candidate,
                preview.review_token,
                preview.review_digest,
            ),
        ))
    });
    hook.wait_until_blocked();

    let preview_started = Arc::new(Barrier::new(2));
    let preview_started_in_thread = preview_started.clone();
    let (preview_done_tx, preview_done_rx) = mpsc::sync_channel(1);
    thread::spawn(move || {
        preview_started_in_thread.wait();
        let result = app.preview_agent_profile_edit(
            profile_id,
            base_version_id,
            draft("Old Base Preview Analyst"),
        );
        preview_done_tx.send((app, result)).unwrap();
    });
    preview_started.wait();

    let previewed_before_commit = match preview_done_rx.recv_timeout(Duration::from_millis(250)) {
        Ok(result) => Some(result),
        Err(RecvTimeoutError::Timeout) => None,
        Err(error) => panic!("preview channel failed: {error}"),
    };
    let preview_completed_before_commit = previewed_before_commit.is_some();
    hook.unblock();
    activation_thread.join().unwrap().unwrap();
    let (app, preview_result) = match previewed_before_commit {
        Some(result) => result,
        None => preview_done_rx.recv().unwrap(),
    };

    assert!(!preview_completed_before_commit);
    assert!(matches!(
        preview_result,
        Err(AppError::StaleAgentProfileVersion { .. })
    ));
    assert_eq!(app.count_rows("agent_profile_versions"), 2);
}
