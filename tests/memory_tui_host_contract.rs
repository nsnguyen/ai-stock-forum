use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};

use ai_stock_forum::{
    agents::{AgentProfileVersion, AgentReadiness, builtin_profile_templates},
    app::{
        AgentProfileSelector, AgentProfileSummary, AgentProfileView, AgentProfilesView, AppError,
        ApplicationCommand, CommandOutcome, CommandView, DatabaseReadiness, MemoryEntriesView,
        MemoryEntrySummary, PresentationSnapshot, ProcessGuardOwnership, ShutdownDisposition,
        ShutdownReason, SkillsView,
    },
    domain::{
        Actor, AgentProfileId, AgentProfileVersionId, CommandId, CorrelationId, DomainError,
        EventId, InstallationId, MemoryEntryId, MemoryEntryVersionId, MemoryNamespaceId, SessionId,
    },
    memory::{MemoryEntryDraft, MemoryEntryVersion},
    runtime::{ApplicationRuntime, CommandExecutor},
    setup::SetupStatus,
    ui::tui::{
        EventSource, Screen, TuiError, TuiEvent,
        model::{AgentsPane, TuiModel, View},
        run_tui_with_screen,
        theme::Theme,
    },
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use uuid::Uuid;

#[test]
fn memory_host_errors_are_stable_and_content_free() {
    assert_eq!(
        TuiError::UnexpectedControllerEffect.to_string(),
        "unexpected controller effect",
    );
    assert_eq!(
        TuiError::MemoryState(DomainError::MemorySelectionUnavailable).to_string(),
        "memory state transition failed",
    );
}

fn profile(seed: u128) -> AgentProfileVersion {
    let template = &builtin_profile_templates()[0];
    AgentProfileVersion::create(
        AgentProfileId::from_uuid(Uuid::from_u128(seed)),
        AgentProfileVersionId::from_uuid(Uuid::from_u128(seed + 1)),
        MemoryNamespaceId::from_uuid(Uuid::from_u128(seed + 2)),
        1_800_000_000_000,
        template.copy_to_draft().expect("profile draft"),
        Some(template.provenance()),
    )
    .expect("profile")
}

fn entry(profile: &AgentProfileVersion, seed: u128) -> MemoryEntryVersion {
    MemoryEntryVersion::create_present(
        profile.memory_namespace_id(),
        MemoryEntryId::from_uuid(Uuid::from_u128(seed)),
        MemoryEntryVersionId::from_uuid(Uuid::from_u128(seed + 1)),
        MemoryEntryDraft::new(
            "Durable host key".to_owned(),
            "Durable host value".to_owned(),
            vec!["host".to_owned()],
        )
        .expect("entry draft"),
        Actor::Human,
        1_800_000_000_001,
        None,
        EventId::from_uuid(Uuid::from_u128(seed + 2)),
    )
    .expect("entry")
}

fn profile_summary(profile: &AgentProfileVersion) -> AgentProfileSummary {
    AgentProfileSummary {
        profile_id: profile.profile_id(),
        profile_version_id: profile.profile_version_id(),
        version: profile.version(),
        display_name: profile.display_name().to_owned(),
        role: profile.role(),
        primary_specialty: profile.primary_specialty().to_owned(),
        readiness: AgentReadiness::Unbound,
        content_digest: profile.content_digest().clone(),
    }
}

fn profiles_view(profile: &AgentProfileVersion) -> AgentProfilesView {
    AgentProfilesView {
        profiles: vec![profile_summary(profile)],
        total_count: 1,
        returned_count: 1,
        truncated: false,
    }
}

fn profile_view(profile: &AgentProfileVersion) -> AgentProfileView {
    AgentProfileView {
        profile: profile.clone(),
        readiness: AgentReadiness::Unbound,
    }
}

fn entries_view(profile: &AgentProfileVersion, entry: &MemoryEntryVersion) -> MemoryEntriesView {
    MemoryEntriesView {
        profile: profile.reference(),
        namespace_id: profile.memory_namespace_id(),
        entries: vec![MemoryEntrySummary {
            entry: entry.reference(),
            display_key: entry.display_key().to_owned(),
            purpose_tags: entry.purpose_tags().to_vec(),
            value_bytes: u64::try_from(entry.value().expect("present entry").len())
                .expect("value length"),
            created_at_ms: entry.created_at_ms(),
        }],
        total_count: 1,
        returned_count: 1,
        omitted_count: 0,
    }
}

fn snapshot(profile: &AgentProfileVersion) -> PresentationSnapshot {
    PresentationSnapshot {
        installation_id: InstallationId::from_uuid(Uuid::from_u128(1)),
        session_id: SessionId::from_uuid(Uuid::from_u128(2)),
        database_readiness: DatabaseReadiness::Ready,
        process_guard_ownership: ProcessGuardOwnership::Held,
        setup_status: SetupStatus::NotStarted,
        recent_audit: Vec::new(),
        agent_profiles: profiles_view(profile),
        selected_agent_profile: Some(profile_view(profile)),
        selected_agent_profile_history: None,
    }
}

fn outcome(view: CommandView) -> CommandOutcome {
    CommandOutcome {
        command_id: CommandId::from_uuid(Uuid::from_u128(10)),
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(11)),
        committed_events: Vec::new(),
        view,
        shutdown: ShutdownDisposition::Continue,
    }
}

struct BlockingMemoryExecutor {
    profile: AgentProfileVersion,
    entry: MemoryEntryVersion,
    calls: Arc<Mutex<Vec<ApplicationCommand>>>,
    memory_started: mpsc::SyncSender<()>,
    release_memory: mpsc::Receiver<()>,
    memory_returned: Arc<AtomicBool>,
    finishes: Arc<Mutex<Vec<ShutdownReason>>>,
}

impl CommandExecutor for BlockingMemoryExecutor {
    fn execute_user(&mut self, command: ApplicationCommand) -> Result<CommandOutcome, AppError> {
        self.calls.lock().unwrap().push(command.clone());
        match command {
            ApplicationCommand::ListSkills => Ok(outcome(CommandView::Skills(SkillsView {
                skills: Vec::new(),
                total_count: 0,
                returned_count: 0,
                truncated: false,
            }))),
            ApplicationCommand::ListAgentProfiles => Ok(outcome(CommandView::AgentProfiles(
                profiles_view(&self.profile),
            ))),
            ApplicationCommand::ShowAgentProfile { selector }
                if selector == AgentProfileSelector::Id(self.profile.profile_id()) =>
            {
                Ok(outcome(CommandView::AgentProfile(profile_view(
                    &self.profile,
                ))))
            }
            ApplicationCommand::ListMemoryEntries { selector }
                if selector == AgentProfileSelector::Id(self.profile.profile_id()) =>
            {
                self.memory_started.send(()).expect("announce memory read");
                self.release_memory.recv().expect("release memory read");
                self.memory_returned.store(true, Ordering::SeqCst);
                Ok(outcome(CommandView::MemoryEntries(entries_view(
                    &self.profile,
                    &self.entry,
                ))))
            }
            _ => Err(AppError::LifecycleFinished),
        }
    }

    fn finish(&mut self, reason: ShutdownReason) -> Result<(), AppError> {
        self.finishes.lock().unwrap().push(reason);
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HostObservation {
    active_view: View,
    agents_pane: AgentsPane,
    command_in_flight: bool,
    memory_hydrated: bool,
    profile_hydrated: bool,
}

struct ContractScreen {
    latest: Arc<Mutex<Option<HostObservation>>>,
    saw_blocked_memory: Arc<AtomicBool>,
    saw_hydrated_help: Arc<AtomicBool>,
    restore_calls: Arc<AtomicUsize>,
}

impl Screen for ContractScreen {
    fn size(&self) -> Result<Rect, TuiError> {
        Ok(Rect::new(0, 0, 140, 40))
    }

    fn draw(&mut self, model: &TuiModel, _theme: &Theme) -> Result<(), TuiError> {
        let memory_hydrated = model.agents.memory.entries.is_some();
        if model.agents.pane == AgentsPane::Memory && model.command_in_flight {
            self.saw_blocked_memory.store(true, Ordering::SeqCst);
        }
        if model.active_view == View::Help
            && model.agents.pane == AgentsPane::Memory
            && memory_hydrated
            && !model.command_in_flight
        {
            self.saw_hydrated_help.store(true, Ordering::SeqCst);
        }
        *self.latest.lock().unwrap() = Some(HostObservation {
            active_view: model.active_view,
            agents_pane: model.agents.pane,
            command_in_flight: model.command_in_flight,
            memory_hydrated,
            profile_hydrated: model.agents.matching_detail().is_some(),
        });
        Ok(())
    }

    fn restore(&mut self) -> Result<(), TuiError> {
        self.restore_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

struct ContractEvents {
    stage: u8,
    latest: Arc<Mutex<Option<HostObservation>>>,
    memory_started: mpsc::Receiver<()>,
    release_memory: Option<mpsc::SyncSender<()>>,
    saw_hydrated_help: Arc<AtomicBool>,
    idle_polls: usize,
}

impl ContractEvents {
    fn key(code: KeyCode) -> TuiEvent {
        TuiEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }
}

impl EventSource for ContractEvents {
    fn next_event(&mut self, _timeout: Duration) -> Result<Option<TuiEvent>, TuiError> {
        match self.stage {
            0 => {
                self.stage = 1;
                Ok(Some(Self::key(KeyCode::Char('3'))))
            }
            1 => {
                self.stage = 2;
                Ok(Some(Self::key(KeyCode::Enter)))
            }
            2 => {
                if self
                    .latest
                    .lock()
                    .unwrap()
                    .as_ref()
                    .is_some_and(|model| model.profile_hydrated && !model.command_in_flight)
                {
                    self.stage = 3;
                    Ok(Some(Self::key(KeyCode::Right)))
                } else {
                    self.idle_polls += 1;
                    assert!(self.idle_polls < 100_000, "profile outcome was not drawn");
                    std::thread::yield_now();
                    Ok(None)
                }
            }
            3 => {
                self.stage = 4;
                Ok(Some(Self::key(KeyCode::Enter)))
            }
            4 => {
                let blocked = self.latest.lock().unwrap().as_ref().is_some_and(|model| {
                    model.agents_pane == AgentsPane::Memory && model.command_in_flight
                });
                if blocked {
                    self.stage = 5;
                    Ok(Some(Self::key(KeyCode::Char('p'))))
                } else {
                    self.idle_polls += 1;
                    assert!(self.idle_polls < 100_000, "memory read was not started");
                    std::thread::yield_now();
                    Ok(None)
                }
            }
            5 => {
                self.stage = 6;
                Ok(Some(Self::key(KeyCode::Char('9'))))
            }
            6 => {
                self.memory_started
                    .recv_timeout(Duration::from_secs(1))
                    .expect("memory worker started");
                self.release_memory
                    .take()
                    .expect("one memory release")
                    .send(())
                    .expect("release memory worker");
                self.stage = 7;
                Ok(None)
            }
            7 if self.saw_hydrated_help.load(Ordering::SeqCst) => {
                self.stage = 8;
                Ok(Some(TuiEvent::Interrupt))
            }
            7 => {
                self.idle_polls = self.idle_polls.saturating_add(1);
                assert!(self.idle_polls < 100_000, "memory outcome was not drawn");
                std::thread::yield_now();
                Ok(None)
            }
            _ => Ok(None),
        }
    }
}

#[test]
fn blocked_memory_read_keeps_one_receiver_then_hydrates_without_navigation_theft() {
    let profile = profile(100);
    let entry = entry(&profile, 200);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let finishes = Arc::new(Mutex::new(Vec::new()));
    let memory_returned = Arc::new(AtomicBool::new(false));
    let (started_sender, started_receiver) = mpsc::sync_channel(1);
    let (release_sender, release_receiver) = mpsc::sync_channel(1);
    let runtime = ApplicationRuntime::spawn(
        BlockingMemoryExecutor {
            profile: profile.clone(),
            entry: entry.clone(),
            calls: Arc::clone(&calls),
            memory_started: started_sender,
            release_memory: release_receiver,
            memory_returned: Arc::clone(&memory_returned),
            finishes: Arc::clone(&finishes),
        },
        1,
    )
    .expect("runtime");
    let latest = Arc::new(Mutex::new(None));
    let saw_blocked_memory = Arc::new(AtomicBool::new(false));
    let saw_hydrated_help = Arc::new(AtomicBool::new(false));
    let restore_calls = Arc::new(AtomicUsize::new(0));
    let mut screen = ContractScreen {
        latest: Arc::clone(&latest),
        saw_blocked_memory: Arc::clone(&saw_blocked_memory),
        saw_hydrated_help: Arc::clone(&saw_hydrated_help),
        restore_calls: Arc::clone(&restore_calls),
    };
    let mut events = ContractEvents {
        stage: 0,
        latest,
        memory_started: started_receiver,
        release_memory: Some(release_sender),
        saw_hydrated_help: Arc::clone(&saw_hydrated_help),
        idle_polls: 0,
    };

    run_tui_with_screen(
        runtime,
        snapshot(&profile),
        false,
        &mut screen,
        &mut events,
        &Theme::from_no_color(true),
    )
    .expect("public TUI host run");

    assert!(saw_blocked_memory.load(Ordering::SeqCst));
    assert!(memory_returned.load(Ordering::SeqCst));
    assert!(saw_hydrated_help.load(Ordering::SeqCst));
    assert_eq!(restore_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        finishes.lock().unwrap().as_slice(),
        [ShutdownReason::Interrupted]
    );
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        [
            ApplicationCommand::ListSkills,
            ApplicationCommand::ListAgentProfiles,
            ApplicationCommand::ShowAgentProfile {
                selector: profile.profile_id().into(),
            },
            ApplicationCommand::ListMemoryEntries {
                selector: profile.profile_id().into(),
            },
        ],
        "the blocked `p` action must not install or submit a second Memory receiver",
    );
}
