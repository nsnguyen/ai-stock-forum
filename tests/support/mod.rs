#![allow(dead_code)]

use ai_stock_forum::{
    agents::{
        AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentProfileVersionRef, AgentRole,
    },
    app::{
        AgentProfileSelector, AppError, ApplicationCommand, ApplicationEvent, ApplicationService,
        ApplicationWorker, AuditLimit, AuthorizationDecision, CommandEnvelope, CommandOutcome,
        CommandPolicy, CommandTransactionHook, CommandView, EVENT_SCHEMA_VERSION,
        MemoryEditPreview, MemoryProfileIdentityView, PendingEvent, PresentationSnapshot,
        ShutdownReason,
    },
    config::AppPaths,
    domain::{
        Actor, Clock, CommandId, CorrelationId, Digest, EpisodicSummaryId, EventId, IdGenerator,
        MemoryProposalId, ObjectRef,
    },
    memory::{
        EpisodicSourceRef, EpisodicSummary, ExpectedMemoryEntryState, MemoryEntryDraft,
        MemoryProposalOperation, MemoryProposalStatus, MemoryResolutionAction,
    },
    persistence::{
        Database, EventRepository, PersistenceError, ProjectionRepository, RecoveryError,
    },
    policy::Capability,
    runtime::{ApplicationRuntime, CommandExecutor, RuntimeClient},
    ui::{
        command::FallbackRunner,
        memory_editor::MemoryEditorStep,
        tui::{
            EventSource, Screen, TuiError, TuiEvent,
            model::{
                AgentDetailAction, AgentsPane, MemoryPane, MemoryProposalDetailAction, TuiModel,
                View,
            },
            render, run_tui_with_screen,
            theme::Theme,
        },
    },
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend, layout::Rect};
use rusqlite::{Connection, OptionalExtension, types::ValueRef};
use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    io::{self, BufRead, Read},
    ops::{Deref, DerefMut},
    sync::{
        Arc, Barrier, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use tempfile::TempDir;
use uuid::Uuid;

pub struct RuntimeFixture {
    _temporary_directory: TempDir,
    paths: AppPaths,
    session_id: ai_stock_forum::domain::SessionId,
    runtime: ApplicationRuntime,
}

impl RuntimeFixture {
    pub fn client(&self) -> RuntimeClient {
        self.runtime.client()
    }

    pub fn finish_and_join(&self, reason: ShutdownReason) {
        self.runtime.finish_and_join(reason).unwrap();
    }

    pub fn last_shutdown_reason(&self) -> Option<String> {
        Connection::open(self.paths.database_path())
            .unwrap()
            .query_row(
                "SELECT end_reason FROM process_session_projection WHERE session_id = ?1",
                [self.session_id.to_string()],
                |row| row.get(0),
            )
            .unwrap()
    }
}

pub fn runtime() -> RuntimeFixture {
    let temporary_directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temporary_directory.path());
    let service =
        ApplicationService::bootstrap(&paths, Arc::new(TestClock::new()), Arc::new(TestIds::new()))
            .unwrap();
    let session_id = service.session_id();
    let runtime = ApplicationRuntime::spawn_application(service, 32).unwrap();
    RuntimeFixture {
        _temporary_directory: temporary_directory,
        paths,
        session_id,
        runtime,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedEvent {
    pub sequence: u64,
    pub event_id: EventId,
    pub kind: String,
    pub payload_json: String,
}

pub struct PersistentFixture {
    _temporary_directory: TempDir,
    paths: AppPaths,
    clock: Arc<TestClock>,
    ids: Arc<TestIds>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawDatabaseSnapshot {
    pub tables: BTreeMap<String, Vec<Vec<String>>>,
}

fn raw_database_snapshot(paths: &AppPaths) -> RawDatabaseSnapshot {
    let connection = Connection::open(paths.database_path()).unwrap();
    let table_names = {
        let mut statement = connection
            .prepare(
                "SELECT name FROM sqlite_schema
                 WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
                 ORDER BY name",
            )
            .unwrap();
        statement
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>()
    };
    let tables = table_names
        .into_iter()
        .map(|table_name| {
            let quoted_name = table_name.replace('"', "\"\"");
            let mut statement = connection
                .prepare(&format!("SELECT * FROM \"{quoted_name}\""))
                .unwrap();
            let column_count = statement.column_count();
            let mut rows = statement
                .query_map([], |row| {
                    (0..column_count)
                        .map(|column| {
                            Ok(match row.get_ref(column)? {
                                ValueRef::Null => "null".to_owned(),
                                ValueRef::Integer(value) => format!("integer:{value}"),
                                ValueRef::Real(value) => {
                                    format!("real:{:016x}", value.to_bits())
                                }
                                ValueRef::Text(value) => {
                                    format!("text:{}", encode_hex(value))
                                }
                                ValueRef::Blob(value) => {
                                    format!("blob:{}", encode_hex(value))
                                }
                            })
                        })
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .unwrap()
                .map(Result::unwrap)
                .collect::<Vec<_>>();
            rows.sort();
            (table_name, rows)
        })
        .collect();
    RawDatabaseSnapshot { tables }
}

fn encode_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    bytes.iter().fold(
        String::with_capacity(bytes.len().saturating_mul(2)),
        |mut encoded, byte| {
            write!(encoded, "{byte:02x}").unwrap();
            encoded
        },
    )
}

impl PersistentFixture {
    pub fn raw_database_snapshot(&self) -> RawDatabaseSnapshot {
        raw_database_snapshot(&self.paths)
    }
    pub fn service(&self) -> ApplicationService {
        ApplicationService::bootstrap(&self.paths, self.clock.clone(), self.ids.clone()).unwrap()
    }

    pub fn service_with_policy(&self, policy: Arc<dyn CommandPolicy>) -> ApplicationService {
        ApplicationService::bootstrap_with_policy(
            &self.paths,
            self.clock.clone(),
            self.ids.clone(),
            policy,
        )
        .unwrap()
    }

    pub fn open_database(&self) -> Database {
        Database::open(&self.paths).unwrap()
    }

    pub fn side_effect_calls(&self) -> (usize, usize) {
        (self.ids.calls(), self.clock.calls())
    }

    pub fn active_profile(
        &self,
        profile_id: ai_stock_forum::domain::AgentProfileId,
    ) -> ai_stock_forum::agents::AgentProfileVersion {
        ProjectionRepository::load(self.open_database().connection())
            .unwrap()
            .agent_profiles
            .active_profiles()
            .into_iter()
            .find(|profile| profile.profile_id() == profile_id)
            .unwrap()
    }

    pub fn runtime(&self) -> ApplicationRuntime {
        let service =
            ApplicationService::bootstrap(&self.paths, self.clock.clone(), self.ids.clone())
                .unwrap();
        ApplicationRuntime::spawn_application(service, 32).unwrap()
    }

    pub fn installation_id(&self) -> ai_stock_forum::domain::InstallationId {
        let database = Database::open(&self.paths).unwrap();
        ProjectionRepository::load(database.connection())
            .unwrap()
            .installation
            .unwrap()
            .installation_id
    }

    pub fn events(&self) -> Vec<PersistedEvent> {
        let database = Database::open(&self.paths).unwrap();
        let mut statement = database
            .connection()
            .prepare(
                "SELECT sequence, event_id, event_type, payload_json
                 FROM event_stream ORDER BY sequence",
            )
            .unwrap();
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .unwrap()
            .map(|row| {
                let (sequence, event_id, kind, payload_json) = row.unwrap();
                PersistedEvent {
                    sequence: u64::try_from(sequence).unwrap(),
                    event_id: event_id.parse().unwrap(),
                    kind,
                    payload_json,
                }
            })
            .collect()
    }

    pub fn event_count(&self, kind: &str) -> i64 {
        Database::open(&self.paths)
            .unwrap()
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM event_stream WHERE event_type = ?1",
                [kind],
                |row| row.get(0),
            )
            .unwrap()
    }

    pub fn count_rows(&self, table: &str) -> i64 {
        assert!(matches!(
            table,
            "event_stream"
                | "installation_projection"
                | "process_session_projection"
                | "projection_metadata"
                | "setup_drafts"
                | "installation_configuration_versions"
                | "active_installation_configuration"
                | "setup_step_outcomes"
                | "capability_readiness"
                | "approval_records"
                | "command_receipts"
                | "command_event_refs"
                | "memory_entry_versions"
                | "current_memory_entries"
                | "memory_proposals"
                | "memory_proposal_resolutions"
                | "current_memory_proposal_status"
                | "episodic_summaries"
                | "episodic_summary_sources"
        ));
        Database::open(&self.paths)
            .unwrap()
            .connection()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
    }

    pub fn table_exists(&self, table: &str) -> bool {
        Database::open(&self.paths)
            .unwrap()
            .connection()
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1
                )",
                [table],
                |row| row.get(0),
            )
            .unwrap()
    }

    pub fn remove_recoverable_projection_state(&self) {
        let connection = Connection::open(self.paths.database_path()).unwrap();
        let transaction = connection.unchecked_transaction().unwrap();
        transaction
            .execute_batch(
                "DELETE FROM projection_metadata;
                 DELETE FROM process_session_projection;
                 DELETE FROM installation_projection;",
            )
            .unwrap();
        transaction.commit().unwrap();
    }

    pub fn verify_event_stream(&self) -> Result<(), RecoveryError> {
        let database = Database::open(&self.paths).unwrap();
        EventRepository::verify(database.connection())
    }

    pub fn assert_projection_rows_match_event_stream(&self) {
        let database = Database::open(&self.paths).unwrap();
        let connection = database.connection();
        let (last_sequence, last_digest): (i64, String) = connection
            .query_row(
                "SELECT sequence, event_digest FROM event_stream
                 ORDER BY sequence DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let (projected_sequence, projected_digest): (i64, Option<String>) = connection
            .query_row(
                "SELECT last_event_sequence, last_event_digest
                 FROM projection_metadata WHERE singleton = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(projected_sequence, last_sequence);
        assert_eq!(projected_digest.as_deref(), Some(last_digest.as_str()));

        let (installation_id, created_event_id, created_at_ms, event_kind, payload_json, event_at):
            (String, String, i64, String, String, i64) = connection
                .query_row(
                    "SELECT p.installation_id, p.created_event_id, p.created_at_ms,
                            e.event_type, e.payload_json, e.occurred_at_ms
                     FROM installation_projection p
                     JOIN event_stream e ON e.event_id = p.created_event_id
                     WHERE p.singleton = 1",
                    [],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                        ))
                    },
                )
                .unwrap();
        assert_eq!(event_kind, "installation_initialized");
        assert_eq!(created_at_ms, event_at);
        assert_eq!(
            payload_field(&payload_json, "installation_id"),
            installation_id
        );
        assert!(self.events().iter().any(|event| {
            event.event_id.to_string() == created_event_id
                && event.kind == "installation_initialized"
        }));

        let started_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM event_stream WHERE event_type = 'process_session_started'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let mut statement = connection
            .prepare(
                "SELECT session_id, started_event_id, started_at_ms,
                        ended_event_id, ended_at_ms, end_reason
                 FROM process_session_projection ORDER BY started_at_ms",
            )
            .unwrap();
        let sessions = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        assert_eq!(i64::try_from(sessions.len()).unwrap(), started_count);
        for (session_id, started_event_id, started_at_ms, ended_event_id, ended_at_ms, reason) in
            sessions
        {
            let (started_kind, started_payload, event_started_at): (String, String, i64) =
                connection
                    .query_row(
                        "SELECT event_type, payload_json, occurred_at_ms
                         FROM event_stream WHERE event_id = ?1",
                        [&started_event_id],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )
                    .unwrap();
            assert_eq!(started_kind, "process_session_started");
            assert_eq!(payload_field(&started_payload, "session_id"), session_id);
            assert_eq!(started_at_ms, event_started_at);

            let ended_event_id = ended_event_id.expect("finished acceptance session");
            let ended_at_ms = ended_at_ms.expect("finished acceptance session");
            let reason = reason.expect("finished acceptance session");
            let (ended_kind, ended_payload, event_ended_at): (String, String, i64) = connection
                .query_row(
                    "SELECT event_type, payload_json, occurred_at_ms
                     FROM event_stream WHERE event_id = ?1",
                    [&ended_event_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
            assert_eq!(payload_field(&ended_payload, "session_id"), session_id);
            assert_eq!(ended_at_ms, event_ended_at);
            match ended_kind.as_str() {
                "process_session_ended" => {
                    assert_eq!(payload_field(&ended_payload, "reason"), reason)
                }
                "previous_session_interrupted" => assert_eq!(reason, "interrupted"),
                _ => panic!("unexpected session terminal event {ended_kind}"),
            }
        }
    }
}

fn payload_field(payload_json: &str, field: &str) -> String {
    serde_json::from_str::<serde_json::Value>(payload_json).unwrap()[field]
        .as_str()
        .unwrap()
        .to_owned()
}

pub fn persistent_fixture() -> PersistentFixture {
    let temporary_directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temporary_directory.path());
    PersistentFixture {
        _temporary_directory: temporary_directory,
        paths,
        clock: Arc::new(TestClock::new()),
        ids: Arc::new(TestIds::new()),
    }
}

pub struct DatabaseFixture {
    _temporary_directory: TempDir,
    pub database: Database,
    next_id: u128,
}

impl DatabaseFixture {
    pub fn append(&mut self, event: ApplicationEvent) -> ai_stock_forum::app::EventEnvelope {
        let pending = self.pending(event);
        let transaction = self.database.immediate_transaction().unwrap();
        let envelope = EventRepository::append(&transaction, pending).unwrap();
        transaction.commit().unwrap();
        envelope
    }

    pub fn pending(&mut self, event: ApplicationEvent) -> PendingEvent {
        let event_id = EventId::from_uuid(self.next_uuid());
        let correlation_id = CorrelationId::from_uuid(self.next_uuid());
        PendingEvent {
            event_id,
            event_schema_version: EVENT_SCHEMA_VERSION,
            actor: Actor::Human,
            occurred_at_ms: 1_700_000_000_000,
            correlation_id,
            causation_id: None,
            object: None,
            event,
        }
    }

    fn next_uuid(&mut self) -> Uuid {
        let value = Uuid::from_u128(self.next_id);
        self.next_id += 1;
        value
    }
}

pub fn database() -> DatabaseFixture {
    let temporary_directory = tempfile::tempdir().unwrap();
    let database = Database::open(&ai_stock_forum::config::AppPaths::for_test(
        temporary_directory.path(),
    ))
    .unwrap();
    DatabaseFixture {
        _temporary_directory: temporary_directory,
        database,
        next_id: 1,
    }
}

pub fn rejected_event(input: &[u8]) -> ai_stock_forum::app::EventEnvelope {
    let mut fixture = database();
    fixture.append(ApplicationEvent::CommandRejected {
        rejection: ai_stock_forum::app::InputRejection::from_input(
            ai_stock_forum::app::InputRejectionCategory::Malformed,
            Some(ai_stock_forum::app::SafeToken::new("/secret").unwrap()),
            input,
        ),
    })
}

pub struct TestClock {
    calls: AtomicUsize,
}

impl TestClock {
    pub fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Clock for TestClock {
    fn now_millis(&self) -> i64 {
        1_700_000_000_000 + self.calls.fetch_add(1, Ordering::SeqCst) as i64
    }
}

pub struct TestIds {
    first: u128,
    calls: AtomicUsize,
}

impl TestIds {
    pub fn new() -> Self {
        Self::starting_at(10_000)
    }

    pub fn starting_at(first: u128) -> Self {
        Self {
            first,
            calls: AtomicUsize::new(0),
        }
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl IdGenerator for TestIds {
    fn next_uuid(&self) -> Uuid {
        Uuid::from_u128(self.first + self.calls.fetch_add(1, Ordering::SeqCst) as u128)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualMemoryAcceptanceSeed {
    pub profile: AgentProfileVersionRef,
    pub tui_approval_proposal_id: MemoryProposalId,
    pub tui_rejection_proposal_id: MemoryProposalId,
    pub fallback_approval_proposal_id: MemoryProposalId,
    pub fallback_rejection_proposal_id: MemoryProposalId,
    pub restart_pending_proposal_id: MemoryProposalId,
    pub summary_id: EpisodicSummaryId,
}

fn manual_acceptance_support_error() -> AppError {
    AppError::Persistence(PersistenceError::QueryFailed)
}

fn manual_acceptance_envelope(
    id: u128,
    actor: Actor,
    command: ApplicationCommand,
) -> CommandEnvelope {
    CommandEnvelope {
        command_id: CommandId::from_uuid(Uuid::from_u128(id)),
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(id + 1_000_000)),
        actor,
        command,
    }
}

fn create_manual_acceptance_proposal(
    service: &mut ApplicationService,
    profile: &ai_stock_forum::agents::AgentProfileVersion,
    command_id: u128,
    display_key: &str,
    value: &str,
    rationale: &str,
) -> Result<(MemoryProposalId, EventId), AppError> {
    let outcome = service.execute(manual_acceptance_envelope(
        command_id,
        Actor::Agent(profile.profile_id()),
        ApplicationCommand::ProposeMemoryMutation {
            proposer: profile.reference(),
            expected: ExpectedMemoryEntryState::Absent,
            operation: MemoryProposalOperation::Set {
                candidate: MemoryEntryDraft::new(
                    display_key.to_owned(),
                    value.to_owned(),
                    vec!["acceptance".to_owned()],
                )?,
            },
            rationale: rationale.to_owned(),
        },
    ))?;
    let CommandView::MemoryProposalCreated(view) = outcome.view else {
        return Err(manual_acceptance_support_error());
    };
    if view.status != MemoryProposalStatus::Pending || outcome.committed_events.len() != 1 {
        return Err(manual_acceptance_support_error());
    }
    Ok((
        view.proposal.proposal_id(),
        outcome.committed_events[0].event_id,
    ))
}

pub fn seed_manual_memory_acceptance(
    paths: &AppPaths,
) -> Result<ManualMemoryAcceptanceSeed, AppError> {
    fs::create_dir(paths.state_dir()).map_err(|_| manual_acceptance_support_error())?;

    let clock = Arc::new(TestClock::new());
    let ids = Arc::new(TestIds::new());
    let mut service = ApplicationService::bootstrap(paths, clock.clone(), ids.clone())
        .map_err(|_| manual_acceptance_support_error())?;

    let seeded = (|| {
        let profile_creation = service.execute(manual_acceptance_envelope(
            7_000_000,
            Actor::Human,
            ApplicationCommand::CreateAgentProfile {
                draft: AgentProfileDraft::new(
                    "Manual Memory Acceptance".to_owned(),
                    "Synthetic local acceptance profile.".to_owned(),
                    AgentRole::Custom,
                    "memory acceptance".to_owned(),
                    vec!["acceptance".to_owned()],
                    "Careful and deterministic.".to_owned(),
                    "Exercise only local Hybrid Memory.".to_owned(),
                    AgentBindings::default(),
                    Vec::new(),
                    Vec::new(),
                )?,
                template_provenance: None,
            },
        ))?;
        let CommandView::AgentProfileCreated(created) = profile_creation.view else {
            return Err(manual_acceptance_support_error());
        };
        let profile_detail = service.execute(manual_acceptance_envelope(
            7_000_001,
            Actor::Human,
            ApplicationCommand::ShowAgentProfile {
                selector: AgentProfileSelector::from(created.profile_id),
            },
        ))?;
        let CommandView::AgentProfile(profile_detail) = profile_detail.view else {
            return Err(manual_acceptance_support_error());
        };
        let profile = profile_detail.profile;

        let (tui_approval_proposal_id, summary_source_event_id) =
            create_manual_acceptance_proposal(
                &mut service,
                &profile,
                7_000_100,
                "TUI approval candidate",
                "Synthetic TUI approval value.",
                "Synthetic TUI approval rationale.",
            )?;
        let (tui_rejection_proposal_id, _) = create_manual_acceptance_proposal(
            &mut service,
            &profile,
            7_000_101,
            "TUI rejection candidate",
            "Synthetic TUI rejection value.",
            "Synthetic TUI rejection rationale.",
        )?;
        let (fallback_approval_proposal_id, _) = create_manual_acceptance_proposal(
            &mut service,
            &profile,
            7_000_102,
            "Fallback approval candidate",
            "Synthetic fallback approval value.",
            "Synthetic fallback approval rationale.",
        )?;
        let (fallback_rejection_proposal_id, _) = create_manual_acceptance_proposal(
            &mut service,
            &profile,
            7_000_103,
            "Fallback rejection candidate",
            "Synthetic fallback rejection value.",
            "Synthetic fallback rejection rationale.",
        )?;
        let (restart_pending_proposal_id, _) = create_manual_acceptance_proposal(
            &mut service,
            &profile,
            7_000_104,
            "Restart preservation candidate",
            "Synthetic restart preservation value.",
            "Synthetic restart preservation rationale.",
        )?;
        let summary = record_test_episodic_summary_at(
            paths,
            ids.as_ref(),
            clock.as_ref(),
            profile.reference(),
            "Manual source-linked summary".to_owned(),
            "Synthetic summary retained for local manual acceptance.".to_owned(),
            vec!["acceptance".to_owned()],
            vec![summary_source_event_id],
        )?;

        Ok(ManualMemoryAcceptanceSeed {
            profile: profile.reference(),
            tui_approval_proposal_id,
            tui_rejection_proposal_id,
            fallback_approval_proposal_id,
            fallback_rejection_proposal_id,
            restart_pending_proposal_id,
            summary_id: summary.summary_id(),
        })
    })();

    let finish_reason = if seeded.is_ok() {
        ShutdownReason::InputClosed
    } else {
        ShutdownReason::ApplicationError
    };
    let finish = service.finish(finish_reason);
    drop(service);
    let seed = seeded?;
    finish?;

    let mut reopened = ApplicationService::bootstrap(paths, clock, ids)
        .map_err(|_| manual_acceptance_support_error())?;
    let finish = reopened.finish(ShutdownReason::InputClosed);
    drop(reopened);
    finish?;

    Ok(seed)
}

const HOST_PARITY_NAVIGATION_LABELS: [&str; 9] = [
    "1 Home",
    "2 Chat",
    "3 Agents",
    "4 Skills",
    "5 Connections",
    "6 Activity",
    "7 Setup",
    "8 Audit",
    "9 Help",
];
const HOST_PARITY_KEY: &str = "HK701X Thesis";
const HOST_PARITY_INITIAL_VALUE: &str = "HI702X Initial aligned value.";
const HOST_PARITY_SET_VALUE: &str = "HV703X Aligned host value.";
const HOST_PARITY_TAG: &str = "HT704X";
const HOST_PARITY_PROPOSAL_KEY: &str = "HP705X Resolution candidate";
const HOST_PARITY_PROPOSAL_VALUE: &str = "HPV706X Aligned proposal value.";
const HOST_PARITY_PROPOSAL_RATIONALE: &str = "HPR707X Aligned proposal rationale.";
const HOST_PARITY_NAVIGATION_SENTINELS: [&str; 7] = [
    "HK701X", "HI702X", "HV703X", "HT704X", "HP705X", "HPV706X", "HPR707X",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostParityResult {
    pub command: ApplicationCommand,
    pub event_payload: ApplicationEvent,
    pub view: CommandView,
    pub navigation_labels: [&'static str; 9],
}

#[derive(Clone)]
struct RecordedHostMutation {
    command: ApplicationCommand,
    event_payload: ApplicationEvent,
    view: CommandView,
}

struct AlignedMemoryHostFixture {
    _temporary_directory: TempDir,
    service: ApplicationService,
    snapshot: PresentationSnapshot,
    profile: AgentProfileVersion,
    proposal_id: MemoryProposalId,
}

fn aligned_memory_host_fixture() -> Result<AlignedMemoryHostFixture, AppError> {
    let temporary_directory = tempfile::tempdir().map_err(|_| manual_acceptance_support_error())?;
    let paths = AppPaths::for_test(temporary_directory.path());
    let clock = Arc::new(TestClock::new());
    let ids = Arc::new(TestIds::starting_at(9_000_000));
    let mut service = ApplicationService::bootstrap(&paths, clock, ids)
        .map_err(|_| manual_acceptance_support_error())?;

    let created = service.execute(manual_acceptance_envelope(
        9_100_000,
        Actor::Human,
        ApplicationCommand::CreateAgentProfile {
            draft: AgentProfileDraft::new(
                "Host parity profile".to_owned(),
                "Synthetic profile for cross-host parity.".to_owned(),
                AgentRole::Custom,
                "memory parity".to_owned(),
                vec![HOST_PARITY_TAG.to_owned()],
                "Deterministic.".to_owned(),
                "Exercise equivalent host paths.".to_owned(),
                AgentBindings::default(),
                Vec::new(),
                Vec::new(),
            )?,
            template_provenance: None,
        },
    ))?;
    let profile = created
        .committed_events
        .iter()
        .find_map(|event| match &event.event {
            ApplicationEvent::AgentProfileCreated { profile } => Some(profile.clone()),
            _ => None,
        })
        .ok_or_else(manual_acceptance_support_error)?;

    let seed_candidate = MemoryEntryDraft::new(
        HOST_PARITY_KEY.to_owned(),
        HOST_PARITY_INITIAL_VALUE.to_owned(),
        vec![HOST_PARITY_TAG.to_owned()],
    )?;
    let MemoryEditPreview::Review(seed_review) = service.preview_memory_set(
        AgentProfileSelector::from(profile.profile_id()),
        seed_candidate,
    )?
    else {
        return Err(manual_acceptance_support_error());
    };
    let seed_candidate = seed_review
        .candidate
        .clone()
        .ok_or_else(manual_acceptance_support_error)?;
    let seeded = service.execute(manual_acceptance_envelope(
        9_100_001,
        Actor::Human,
        ApplicationCommand::SetMemoryEntry {
            profile: seed_review.profile,
            expected: seed_review.expected,
            candidate: seed_candidate,
            review_token: seed_review.review_token,
            review_digest: seed_review.review_digest,
        },
    ))?;
    if !matches!(seeded.view, CommandView::MemoryEntryMutation(_)) {
        return Err(manual_acceptance_support_error());
    }

    let (proposal_id, _) = create_manual_acceptance_proposal(
        &mut service,
        &profile,
        9_100_002,
        HOST_PARITY_PROPOSAL_KEY,
        HOST_PARITY_PROPOSAL_VALUE,
        HOST_PARITY_PROPOSAL_RATIONALE,
    )?;

    let audit_limit = AuditLimit::new(100).map_err(|_| manual_acceptance_support_error())?;
    let snapshot = service.presentation_snapshot(audit_limit)?;
    Ok(AlignedMemoryHostFixture {
        _temporary_directory: temporary_directory,
        service,
        snapshot,
        profile,
        proposal_id,
    })
}

fn execute_aligned_memory_prerequisite_reads(
    service: &mut ApplicationService,
    profile: &AgentProfileVersion,
) -> Result<(), AppError> {
    let commands = [
        ApplicationCommand::ListSkills,
        ApplicationCommand::ListAgentProfiles,
        ApplicationCommand::ShowAgentProfile {
            selector: AgentProfileSelector::from(profile.profile_id()),
        },
        ApplicationCommand::ListMemoryEntries {
            selector: AgentProfileSelector::from(profile.profile_id()),
        },
    ];
    for command in commands {
        service.execute_user(command)?;
    }
    Ok(())
}

struct RecordingMemoryExecutor {
    service: ApplicationService,
    recorded: Arc<Mutex<Option<RecordedHostMutation>>>,
    confirmation: Arc<Mutex<Option<String>>>,
    target: RecordedHostTarget,
}

#[derive(Clone, Copy)]
enum RecordedHostTarget {
    Set,
    Resolution(MemoryResolutionAction),
}

impl RecordedHostTarget {
    fn matches(self, command: &ApplicationCommand) -> bool {
        matches!(
            (self, command),
            (Self::Set, ApplicationCommand::SetMemoryEntry { .. })
                | (
                    Self::Resolution(MemoryResolutionAction::Approve),
                    ApplicationCommand::ApproveMemoryProposal { .. }
                )
                | (
                    Self::Resolution(MemoryResolutionAction::Reject),
                    ApplicationCommand::RejectMemoryProposal { .. }
                )
        )
    }
}

impl CommandExecutor for RecordingMemoryExecutor {
    fn execute_user(&mut self, command: ApplicationCommand) -> Result<CommandOutcome, AppError> {
        let should_record = self.target.matches(&command);
        let submitted = command.clone();
        let outcome = self.service.execute_user(command)?;
        if should_record {
            let event_payload = outcome
                .committed_events
                .first()
                .map(|event| event.event.clone());
            if outcome.committed_events.len() == 1
                && let Some(event_payload) = event_payload
            {
                *self
                    .recorded
                    .lock()
                    .map_err(|_| AppError::LifecycleFinished)? = Some(RecordedHostMutation {
                    command: submitted,
                    event_payload,
                    view: outcome.view.clone(),
                });
            }
        }
        Ok(outcome)
    }

    fn preview_memory_set(
        &mut self,
        selector: AgentProfileSelector,
        candidate: MemoryEntryDraft,
    ) -> Result<MemoryEditPreview, AppError> {
        let preview = self.service.preview_memory_set(selector, candidate)?;
        if let MemoryEditPreview::Review(review) = &preview {
            *self
                .confirmation
                .lock()
                .map_err(|_| AppError::LifecycleFinished)? =
                Some(format!("set {}", review.review_digest));
        }
        Ok(preview)
    }

    fn preview_memory_delete(
        &mut self,
        selector: AgentProfileSelector,
        display_key: String,
    ) -> Result<MemoryEditPreview, AppError> {
        self.service.preview_memory_delete(selector, display_key)
    }

    fn preview_memory_proposal_approval(
        &mut self,
        proposal: ai_stock_forum::memory::MemoryProposalRef,
    ) -> Result<ai_stock_forum::app::MemoryProposalResolutionReview, AppError> {
        let review = self.service.preview_memory_proposal_approval(proposal)?;
        *self
            .confirmation
            .lock()
            .map_err(|_| AppError::LifecycleFinished)? =
            Some(format!("approve {}", review.review_digest));
        Ok(review)
    }

    fn preview_memory_proposal_rejection(
        &mut self,
        proposal: ai_stock_forum::memory::MemoryProposalRef,
    ) -> Result<ai_stock_forum::app::MemoryProposalResolutionReview, AppError> {
        let review = self.service.preview_memory_proposal_rejection(proposal)?;
        *self
            .confirmation
            .lock()
            .map_err(|_| AppError::LifecycleFinished)? =
            Some(format!("reject {}", review.review_digest));
        Ok(review)
    }

    fn cancel_memory_review(&mut self) -> Result<(), AppError> {
        self.service.cancel_memory_review()
    }

    fn finish(&mut self, reason: ShutdownReason) -> Result<(), AppError> {
        self.service.finish(reason)
    }
}

struct DynamicFallbackInput {
    initial: VecDeque<Vec<u8>>,
    confirmation: Arc<Mutex<Option<String>>>,
    confirmation_emitted: bool,
    current: Vec<u8>,
    position: usize,
}

impl DynamicFallbackInput {
    fn memory_set(profile: &AgentProfileVersion, confirmation: Arc<Mutex<Option<String>>>) -> Self {
        Self {
            initial: VecDeque::from([
                format!(
                    "/memory set {} \"{HOST_PARITY_KEY}\"\n",
                    profile.profile_id()
                )
                .into_bytes(),
                format!("{HOST_PARITY_SET_VALUE}\n").into_bytes(),
                format!("{HOST_PARITY_TAG}\n").into_bytes(),
            ]),
            confirmation,
            confirmation_emitted: false,
            current: Vec::new(),
            position: 0,
        }
    }

    fn memory_resolution(
        proposal_id: MemoryProposalId,
        action: MemoryResolutionAction,
        confirmation: Arc<Mutex<Option<String>>>,
    ) -> Self {
        let action = match action {
            MemoryResolutionAction::Approve => "approve",
            MemoryResolutionAction::Reject => "reject",
        };
        Self {
            initial: VecDeque::from([format!("/memory {action} {proposal_id}\n").into_bytes()]),
            confirmation,
            confirmation_emitted: false,
            current: Vec::new(),
            position: 0,
        }
    }

    fn load_next(&mut self) -> io::Result<bool> {
        if self.position < self.current.len() {
            return Ok(true);
        }
        self.current.clear();
        self.position = 0;
        if let Some(next) = self.initial.pop_front() {
            self.current = next;
            return Ok(true);
        }
        if self.confirmation_emitted {
            return Ok(false);
        }
        let confirmation = self
            .confirmation
            .lock()
            .map_err(|_| io::Error::other("fallback confirmation unavailable"))?
            .clone()
            .ok_or_else(|| io::Error::other("fallback confirmation unavailable"))?;
        self.current = format!("{confirmation}\n").into_bytes();
        self.confirmation_emitted = true;
        Ok(true)
    }
}

impl Read for DynamicFallbackInput {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let available = self.fill_buf()?;
        let count = available.len().min(output.len());
        output[..count].copy_from_slice(&available[..count]);
        self.consume(count);
        Ok(count)
    }
}

impl BufRead for DynamicFallbackInput {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.load_next()?;
        Ok(&self.current[self.position..])
    }

    fn consume(&mut self, amount: usize) {
        self.position = self.position.saturating_add(amount).min(self.current.len());
    }
}

fn render_memory_navigation_labels(model: &TuiModel) -> Result<[&'static str; 9], TuiError> {
    let width = model.terminal_width.max(140);
    let height = model.terminal_height.max(40);
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).map_err(|_| TuiError::TerminalOutput)?;
    terminal
        .draw(|frame| render::render(frame, model, &Theme::from_no_color(true)))
        .map_err(|_| TuiError::TerminalOutput)?;
    let text = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    let header = text
        .chars()
        .take(usize::from(width) * 3)
        .collect::<String>();
    if HOST_PARITY_NAVIGATION_SENTINELS
        .iter()
        .any(|sentinel| header.contains(sentinel))
    {
        return Err(TuiError::TerminalOutput);
    }
    if HOST_PARITY_NAVIGATION_LABELS
        .iter()
        .any(|label| !header.contains(label))
    {
        return Err(TuiError::TerminalOutput);
    }
    Ok(HOST_PARITY_NAVIGATION_LABELS)
}

fn fallback_memory_navigation_labels(
    snapshot: PresentationSnapshot,
    profile: &AgentProfileVersion,
) -> Result<[&'static str; 9], TuiError> {
    let mut model = TuiModel::new(snapshot, false);
    model.select_view(View::Agents);
    model
        .agents
        .memory
        .bind_profile(
            MemoryProfileIdentityView {
                profile: profile.reference(),
                display_name: profile.display_name().to_owned(),
            },
            profile.memory_namespace_id(),
        )
        .map_err(TuiError::MemoryState)?;
    model.agents.pane = AgentsPane::Memory;
    model.set_terminal_size(140, 40);
    render_memory_navigation_labels(&model)
}

fn host_parity_result(
    recorded: Arc<Mutex<Option<RecordedHostMutation>>>,
    navigation_labels: [&'static str; 9],
) -> HostParityResult {
    let recorded = recorded
        .lock()
        .ok()
        .and_then(|recorded| recorded.clone())
        .unwrap_or_else(|| panic!("host parity mutation was not recorded"));
    HostParityResult {
        command: recorded.command,
        event_payload: recorded.event_payload,
        view: recorded.view,
        navigation_labels,
    }
}

pub fn run_fallback_memory_set_scenario() -> HostParityResult {
    let mut fixture = aligned_memory_host_fixture()
        .unwrap_or_else(|error| panic!("fallback parity fixture failed: {}", error.code()));
    execute_aligned_memory_prerequisite_reads(&mut fixture.service, &fixture.profile)
        .unwrap_or_else(|error| panic!("fallback prerequisite read failed: {}", error.code()));
    let navigation_labels =
        fallback_memory_navigation_labels(fixture.snapshot.clone(), &fixture.profile)
            .unwrap_or_else(|_| panic!("fallback parity navigation was not rendered"));
    let recorded = Arc::new(Mutex::new(None));
    let confirmation = Arc::new(Mutex::new(None));
    let input = DynamicFallbackInput::memory_set(&fixture.profile, Arc::clone(&confirmation));
    let runtime = ApplicationRuntime::spawn(
        RecordingMemoryExecutor {
            service: fixture.service,
            recorded: Arc::clone(&recorded),
            confirmation,
            target: RecordedHostTarget::Set,
        },
        32,
    )
    .unwrap_or_else(|_| panic!("fallback parity runtime did not start"));
    let runner = FallbackRunner::new(runtime.client(), false);
    let mut output = Vec::new();
    let run = runner.run(input, &mut output);
    let reason = run
        .as_ref()
        .copied()
        .unwrap_or(ShutdownReason::ApplicationError);
    let finish = runtime.finish_and_join(reason);
    if run.is_err() || finish.is_err() {
        panic!("fallback parity host did not close cleanly");
    }
    host_parity_result(recorded, navigation_labels)
}

struct ParityScreen {
    latest: Arc<Mutex<Option<TuiModel>>>,
    navigation_labels: Arc<Mutex<Option<[&'static str; 9]>>>,
    restore_calls: Arc<AtomicUsize>,
}

impl Screen for ParityScreen {
    fn size(&self) -> Result<Rect, TuiError> {
        Ok(Rect::new(0, 0, 140, 40))
    }

    fn draw(&mut self, model: &TuiModel, _theme: &Theme) -> Result<(), TuiError> {
        if model.active_view == View::Agents && model.agents.pane == AgentsPane::Memory {
            let labels = render_memory_navigation_labels(model)?;
            *self
                .navigation_labels
                .lock()
                .map_err(|_| TuiError::TerminalOutput)? = Some(labels);
        }
        *self.latest.lock().map_err(|_| TuiError::TerminalOutput)? = Some(model.clone());
        Ok(())
    }

    fn restore(&mut self) -> Result<(), TuiError> {
        self.restore_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

struct MemorySetParityEvents {
    stage: u8,
    latest: Arc<Mutex<Option<TuiModel>>>,
    idle_polls: usize,
    total_polls: usize,
    deadline: Instant,
}

impl MemorySetParityEvents {
    fn key(code: KeyCode) -> TuiEvent {
        TuiEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn latest(&self) -> Option<TuiModel> {
        self.latest.lock().ok().and_then(|latest| latest.clone())
    }

    fn idle(&mut self, timeout: Duration) -> Result<Option<TuiEvent>, TuiError> {
        self.idle_polls = self.idle_polls.saturating_add(1);
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if self.idle_polls >= 100_000 || remaining.is_zero() {
            return Err(TuiError::TerminalInput);
        }
        thread::sleep(timeout.min(Duration::from_millis(1)).min(remaining));
        Ok(None)
    }

    fn advance(&mut self, event: TuiEvent) -> Result<Option<TuiEvent>, TuiError> {
        self.stage = self.stage.saturating_add(1);
        self.idle_polls = 0;
        Ok(Some(event))
    }
}

impl EventSource for MemorySetParityEvents {
    fn next_event(&mut self, timeout: Duration) -> Result<Option<TuiEvent>, TuiError> {
        self.total_polls = self.total_polls.saturating_add(1);
        if self.total_polls >= 100_000 || Instant::now() >= self.deadline {
            return Err(TuiError::TerminalInput);
        }
        let model = self.latest();
        match self.stage {
            0 => self.advance(Self::key(KeyCode::Char('3'))),
            1 if model.as_ref().is_some_and(|model| {
                model.active_view == View::Agents
                    && model.agents.pane == AgentsPane::List
                    && !model.command_in_flight
                    && !model.agents.profiles.profiles.is_empty()
            }) =>
            {
                self.advance(Self::key(KeyCode::Enter))
            }
            2 if model.as_ref().is_some_and(|model| {
                model.agents.pane == AgentsPane::Detail
                    && model.agents.detail.is_some()
                    && !model.command_in_flight
            }) =>
            {
                self.advance(Self::key(KeyCode::Right))
            }
            3 if model.as_ref().is_some_and(|model| {
                model.agents.selected_detail_action == AgentDetailAction::Memory
            }) =>
            {
                self.advance(Self::key(KeyCode::Enter))
            }
            4 if model.as_ref().is_some_and(|model| {
                model.agents.pane == AgentsPane::Memory
                    && model.agents.memory.pane == MemoryPane::EntryList
                    && model.agents.memory.entries.is_some()
                    && !model.command_in_flight
            }) =>
            {
                self.advance(Self::key(KeyCode::Enter))
            }
            5 if model.as_ref().is_some_and(|model| {
                model.agents.memory.pane == MemoryPane::EntryDetail
                    && model.agents.memory.entry_detail.is_some()
                    && !model.command_in_flight
            }) =>
            {
                self.advance(Self::key(KeyCode::Enter))
            }
            6 if model.as_ref().is_some_and(|model| {
                model.agents.memory.pane == MemoryPane::Editor
                    && model
                        .agents
                        .memory
                        .editor
                        .as_ref()
                        .is_some_and(|editor| editor.step() == MemoryEditorStep::Value)
            }) =>
            {
                if model
                    .as_ref()
                    .is_some_and(|model| model.command.text().is_empty())
                {
                    self.advance(TuiEvent::Paste(HOST_PARITY_SET_VALUE.to_owned()))
                } else {
                    self.idle_polls = 0;
                    Ok(Some(Self::key(KeyCode::Backspace)))
                }
            }
            7 if model.as_ref().is_some_and(|model| {
                model.agents.memory.pane == MemoryPane::Editor
                    && model.command.text() == HOST_PARITY_SET_VALUE
            }) =>
            {
                self.advance(Self::key(KeyCode::Enter))
            }
            8 if model.as_ref().is_some_and(|model| {
                model
                    .agents
                    .memory
                    .editor
                    .as_ref()
                    .is_some_and(|editor| editor.step() == MemoryEditorStep::PurposeTags)
                    && model.command.text() == HOST_PARITY_TAG
            }) =>
            {
                self.advance(Self::key(KeyCode::Enter))
            }
            9 if model.as_ref().is_some_and(|model| {
                model.agents.memory.pane == MemoryPane::MutationReview
                    && model.agents.memory.review_registered
                    && !model.command_in_flight
            }) =>
            {
                self.advance(Self::key(KeyCode::Enter))
            }
            10 if model.as_ref().is_some_and(|model| {
                model.agents.memory.pane == MemoryPane::Confirmation
                    && model.agents.memory.confirmation.is_some()
            }) =>
            {
                self.advance(Self::key(KeyCode::Enter))
            }
            11 if model.as_ref().is_some_and(|model| {
                model.agents.memory.pane == MemoryPane::Result && !model.command_in_flight
            }) =>
            {
                self.advance(TuiEvent::Interrupt)
            }
            _ => self.idle(timeout),
        }
    }
}

pub fn run_tui_memory_set_scenario() -> HostParityResult {
    let fixture = aligned_memory_host_fixture()
        .unwrap_or_else(|error| panic!("TUI parity fixture failed: {}", error.code()));
    let recorded = Arc::new(Mutex::new(None));
    let confirmation = Arc::new(Mutex::new(None));
    let runtime = ApplicationRuntime::spawn(
        RecordingMemoryExecutor {
            service: fixture.service,
            recorded: Arc::clone(&recorded),
            confirmation,
            target: RecordedHostTarget::Set,
        },
        32,
    )
    .unwrap_or_else(|_| panic!("TUI parity runtime did not start"));
    let latest = Arc::new(Mutex::new(None));
    let navigation_labels = Arc::new(Mutex::new(None));
    let restore_calls = Arc::new(AtomicUsize::new(0));
    let mut screen = ParityScreen {
        latest: Arc::clone(&latest),
        navigation_labels: Arc::clone(&navigation_labels),
        restore_calls: Arc::clone(&restore_calls),
    };
    let mut events = MemorySetParityEvents {
        stage: 0,
        latest,
        idle_polls: 0,
        total_polls: 0,
        deadline: Instant::now() + Duration::from_secs(10),
    };
    run_tui_with_screen(
        runtime,
        fixture.snapshot,
        false,
        &mut screen,
        &mut events,
        &Theme::from_no_color(true),
    )
    .unwrap_or_else(|_| panic!("TUI parity host did not close cleanly"));
    if restore_calls.load(Ordering::SeqCst) != 1 {
        panic!("TUI parity screen was not restored exactly once");
    }
    let labels = navigation_labels
        .lock()
        .ok()
        .and_then(|labels| *labels)
        .unwrap_or_else(|| panic!("TUI parity navigation was not rendered"));
    host_parity_result(recorded, labels)
}

pub fn run_fallback_memory_resolution_scenario(action: MemoryResolutionAction) -> HostParityResult {
    let mut fixture = aligned_memory_host_fixture()
        .unwrap_or_else(|error| panic!("fallback resolution fixture failed: {}", error.code()));
    execute_aligned_memory_prerequisite_reads(&mut fixture.service, &fixture.profile)
        .unwrap_or_else(|error| panic!("fallback prerequisite read failed: {}", error.code()));
    fixture
        .service
        .execute_user(ApplicationCommand::ListMemoryProposals {
            selector: AgentProfileSelector::from(fixture.profile.profile_id()),
            filter: ai_stock_forum::memory::MemoryProposalFilter::Pending,
        })
        .unwrap_or_else(|error| panic!("fallback proposal read failed: {}", error.code()));
    let navigation_labels =
        fallback_memory_navigation_labels(fixture.snapshot.clone(), &fixture.profile)
            .unwrap_or_else(|_| panic!("fallback resolution navigation was not rendered"));
    let recorded = Arc::new(Mutex::new(None));
    let confirmation = Arc::new(Mutex::new(None));
    let input = DynamicFallbackInput::memory_resolution(
        fixture.proposal_id,
        action,
        Arc::clone(&confirmation),
    );
    let runtime = ApplicationRuntime::spawn(
        RecordingMemoryExecutor {
            service: fixture.service,
            recorded: Arc::clone(&recorded),
            confirmation,
            target: RecordedHostTarget::Resolution(action),
        },
        32,
    )
    .unwrap_or_else(|_| panic!("fallback resolution runtime did not start"));
    let runner = FallbackRunner::new(runtime.client(), false);
    let mut output = Vec::new();
    let run = runner.run(input, &mut output);
    let reason = run
        .as_ref()
        .copied()
        .unwrap_or(ShutdownReason::ApplicationError);
    let finish = runtime.finish_and_join(reason);
    if run.is_err() || finish.is_err() {
        panic!("fallback resolution host did not close cleanly");
    }
    host_parity_result(recorded, navigation_labels)
}

struct MemoryResolutionParityEvents {
    action: MemoryResolutionAction,
    stage: u8,
    latest: Arc<Mutex<Option<TuiModel>>>,
    idle_polls: usize,
    total_polls: usize,
    deadline: Instant,
}

impl MemoryResolutionParityEvents {
    fn key(code: KeyCode) -> TuiEvent {
        TuiEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn latest(&self) -> Option<TuiModel> {
        self.latest.lock().ok().and_then(|latest| latest.clone())
    }

    fn idle(&mut self, timeout: Duration) -> Result<Option<TuiEvent>, TuiError> {
        self.idle_polls = self.idle_polls.saturating_add(1);
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if self.idle_polls >= 100_000 || remaining.is_zero() {
            return Err(TuiError::TerminalInput);
        }
        thread::sleep(timeout.min(Duration::from_millis(1)).min(remaining));
        Ok(None)
    }

    fn advance(&mut self, event: TuiEvent) -> Result<Option<TuiEvent>, TuiError> {
        self.stage = self.stage.saturating_add(1);
        self.idle_polls = 0;
        Ok(Some(event))
    }
}

impl EventSource for MemoryResolutionParityEvents {
    fn next_event(&mut self, timeout: Duration) -> Result<Option<TuiEvent>, TuiError> {
        self.total_polls = self.total_polls.saturating_add(1);
        if self.total_polls >= 100_000 || Instant::now() >= self.deadline {
            return Err(TuiError::TerminalInput);
        }
        let model = self.latest();
        match self.stage {
            0 => self.advance(Self::key(KeyCode::Char('3'))),
            1 if model.as_ref().is_some_and(|model| {
                model.active_view == View::Agents
                    && model.agents.pane == AgentsPane::List
                    && !model.command_in_flight
                    && !model.agents.profiles.profiles.is_empty()
            }) =>
            {
                self.advance(Self::key(KeyCode::Enter))
            }
            2 if model.as_ref().is_some_and(|model| {
                model.agents.pane == AgentsPane::Detail
                    && model.agents.detail.is_some()
                    && !model.command_in_flight
            }) =>
            {
                self.advance(Self::key(KeyCode::Right))
            }
            3 if model.as_ref().is_some_and(|model| {
                model.agents.selected_detail_action == AgentDetailAction::Memory
            }) =>
            {
                self.advance(Self::key(KeyCode::Enter))
            }
            4 if model.as_ref().is_some_and(|model| {
                model.agents.pane == AgentsPane::Memory
                    && model.agents.memory.pane == MemoryPane::EntryList
                    && model.agents.memory.entries.is_some()
                    && !model.command_in_flight
            }) =>
            {
                self.advance(Self::key(KeyCode::Char('p')))
            }
            5 if model.as_ref().is_some_and(|model| {
                model.agents.memory.pane == MemoryPane::Proposals
                    && model.agents.memory.proposals.is_some()
                    && !model.command_in_flight
            }) =>
            {
                self.advance(Self::key(KeyCode::Enter))
            }
            6 if model.as_ref().is_some_and(|model| {
                model.agents.memory.pane == MemoryPane::ProposalDetail
                    && model.agents.memory.proposal_detail.is_some()
                    && !model.command_in_flight
            }) =>
            {
                let selected_action = model
                    .as_ref()
                    .map(|model| model.agents.memory.selected_proposal_detail_action);
                let desired_action = match self.action {
                    MemoryResolutionAction::Approve => MemoryProposalDetailAction::Approve,
                    MemoryResolutionAction::Reject => MemoryProposalDetailAction::Reject,
                };
                if selected_action == Some(desired_action) {
                    self.advance(Self::key(KeyCode::Enter))
                } else {
                    self.idle_polls = 0;
                    Ok(Some(Self::key(KeyCode::Right)))
                }
            }
            7 if model.as_ref().is_some_and(|model| {
                model.agents.memory.pane == MemoryPane::ProposalResolutionReview
                    && model.agents.memory.review_registered
                    && !model.command_in_flight
            }) =>
            {
                self.advance(Self::key(KeyCode::Enter))
            }
            8 if model.as_ref().is_some_and(|model| {
                model.agents.memory.pane == MemoryPane::Confirmation
                    && model.agents.memory.confirmation.is_some()
            }) =>
            {
                self.advance(Self::key(KeyCode::Enter))
            }
            9 if model.as_ref().is_some_and(|model| {
                model.agents.memory.pane == MemoryPane::Result && !model.command_in_flight
            }) =>
            {
                self.advance(TuiEvent::Interrupt)
            }
            _ => self.idle(timeout),
        }
    }
}

pub fn run_tui_memory_resolution_scenario(action: MemoryResolutionAction) -> HostParityResult {
    let fixture = aligned_memory_host_fixture()
        .unwrap_or_else(|error| panic!("TUI resolution fixture failed: {}", error.code()));
    let recorded = Arc::new(Mutex::new(None));
    let confirmation = Arc::new(Mutex::new(None));
    let runtime = ApplicationRuntime::spawn(
        RecordingMemoryExecutor {
            service: fixture.service,
            recorded: Arc::clone(&recorded),
            confirmation,
            target: RecordedHostTarget::Resolution(action),
        },
        32,
    )
    .unwrap_or_else(|_| panic!("TUI resolution runtime did not start"));
    let latest = Arc::new(Mutex::new(None));
    let navigation_labels = Arc::new(Mutex::new(None));
    let restore_calls = Arc::new(AtomicUsize::new(0));
    let mut screen = ParityScreen {
        latest: Arc::clone(&latest),
        navigation_labels: Arc::clone(&navigation_labels),
        restore_calls: Arc::clone(&restore_calls),
    };
    let mut events = MemoryResolutionParityEvents {
        action,
        stage: 0,
        latest,
        idle_polls: 0,
        total_polls: 0,
        deadline: Instant::now() + Duration::from_secs(10),
    };
    run_tui_with_screen(
        runtime,
        fixture.snapshot,
        false,
        &mut screen,
        &mut events,
        &Theme::from_no_color(true),
    )
    .unwrap_or_else(|_| panic!("TUI resolution host did not close cleanly"));
    if restore_calls.load(Ordering::SeqCst) != 1 {
        panic!("TUI resolution screen was not restored exactly once");
    }
    let labels = navigation_labels
        .lock()
        .ok()
        .and_then(|labels| *labels)
        .unwrap_or_else(|| panic!("TUI resolution navigation was not rendered"));
    host_parity_result(recorded, labels)
}

#[derive(Clone)]
pub struct RecordingPolicy {
    decision: Arc<Mutex<AuthorizationDecision>>,
    calls: Arc<AtomicUsize>,
    capabilities: Arc<Mutex<Vec<Capability>>>,
}

impl RecordingPolicy {
    pub fn new(decision: AuthorizationDecision) -> Self {
        Self {
            decision: Arc::new(Mutex::new(decision)),
            calls: Arc::new(AtomicUsize::new(0)),
            capabilities: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    pub fn capabilities(&self) -> Vec<Capability> {
        self.capabilities.lock().unwrap().clone()
    }

    pub fn set_decision(&self, decision: AuthorizationDecision) {
        *self.decision.lock().unwrap() = decision;
    }
}

impl CommandPolicy for RecordingPolicy {
    fn authorize(&self, capability: Capability) -> AuthorizationDecision {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.capabilities.lock().unwrap().push(capability);
        *self.decision.lock().unwrap()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookFailure {
    OutcomeMaterialization,
    ReceiptWrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptTamper {
    NoncanonicalRequest,
    NoncanonicalOutcome,
    TypedInvalidRequest,
    TypedInvalidOutcome,
    FingerprintMismatch,
    CapabilityMismatch,
    PolicyDecisionMismatch,
    EventRefOutcomeMismatch,
    OrdinalGap,
    MalformedReference,
}

pub struct TestCommandHook {
    failure: Option<HookFailure>,
}

impl TestCommandHook {
    pub fn passing() -> Self {
        Self { failure: None }
    }

    pub fn failing(failure: HookFailure) -> Self {
        Self {
            failure: Some(failure),
        }
    }
}

impl CommandTransactionHook for TestCommandHook {
    fn before_outcome_materialization(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        if self.failure == Some(HookFailure::OutcomeMaterialization) {
            Err(PersistenceError::QueryFailed)
        } else {
            Ok(())
        }
    }

    fn before_receipt_write(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        if self.failure == Some(HookFailure::ReceiptWrite) {
            Err(PersistenceError::QueryFailed)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleRacePoint {
    BeforeUserRead,
    AfterUserRead,
}

pub struct TestLifecycleHook {
    point: LifecycleRacePoint,
    command_entered: Arc<Barrier>,
    command_release: Arc<Barrier>,
    finish_attempted: Option<Arc<Barrier>>,
}

impl TestLifecycleHook {
    pub fn new(
        point: LifecycleRacePoint,
        command_entered: Arc<Barrier>,
        command_release: Arc<Barrier>,
        finish_attempted: Option<Arc<Barrier>>,
    ) -> Self {
        Self {
            point,
            command_entered,
            command_release,
            finish_attempted,
        }
    }

    fn block_command(&self) {
        self.command_entered.wait();
        self.command_release.wait();
    }
}

impl CommandTransactionHook for TestLifecycleHook {
    fn before_user_lifecycle_read(&self) {
        if self.point == LifecycleRacePoint::BeforeUserRead {
            self.block_command();
        }
    }

    fn after_user_lifecycle_read(&self) {
        if self.point == LifecycleRacePoint::AfterUserRead {
            self.block_command();
        }
    }

    fn before_finish_lifecycle_write(&self) {
        if let Some(barrier) = &self.finish_attempted {
            barrier.wait();
        }
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

pub struct TestApp {
    _temporary_directory: TempDir,
    paths: AppPaths,
    pub clock: Arc<TestClock>,
    pub ids: Arc<TestIds>,
    service: ApplicationService,
    policy: Option<Arc<dyn CommandPolicy>>,
    hook: Arc<dyn CommandTransactionHook>,
}

impl Deref for TestApp {
    type Target = ApplicationService;

    fn deref(&self) -> &Self::Target {
        &self.service
    }
}

impl DerefMut for TestApp {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.service
    }
}

impl TestApp {
    pub fn raw_database_snapshot(&self) -> RawDatabaseSnapshot {
        raw_database_snapshot(&self.paths)
    }

    pub fn record_test_episodic_summary(
        &self,
        profile: ai_stock_forum::agents::AgentProfileVersionRef,
        label: String,
        body: String,
        purpose_tags: Vec<String>,
        source_event_ids: Vec<EventId>,
    ) -> Result<ai_stock_forum::memory::EpisodicSummaryRef, ai_stock_forum::app::AppError> {
        record_test_episodic_summary_at(
            &self.paths,
            self.ids.as_ref(),
            self.clock.as_ref(),
            profile,
            label,
            body,
            purpose_tags,
            source_event_ids,
        )
    }

    pub fn into_runtime(self) -> RuntimeFixture {
        let TestApp {
            _temporary_directory,
            paths,
            service,
            ..
        } = self;
        let session_id = service.session_id();
        let runtime = ApplicationRuntime::spawn_application(service, 32).unwrap();
        RuntimeFixture {
            _temporary_directory,
            paths,
            session_id,
            runtime,
        }
    }

    pub fn open_database(&self) -> Database {
        Database::open(&self.paths).unwrap()
    }

    pub fn projection(&self) -> ai_stock_forum::recovery::ProjectionState {
        let database = Database::open(&self.paths).unwrap();
        ProjectionRepository::load(database.connection()).unwrap()
    }

    pub fn count_rows(&self, table: &str) -> i64 {
        assert!(matches!(
            table,
            "event_stream"
                | "setup_drafts"
                | "installation_configuration_versions"
                | "capability_readiness"
                | "approval_records"
                | "command_receipts"
                | "command_event_refs"
                | "agent_profile_versions"
                | "active_agent_profiles"
                | "memory_entry_versions"
                | "current_memory_entries"
                | "memory_proposals"
                | "memory_proposal_resolutions"
                | "current_memory_proposal_status"
                | "episodic_summaries"
                | "episodic_summary_sources"
        ));
        Connection::open(self.paths.database_path())
            .unwrap()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
    }

    pub fn active_profile_rows(&self) -> Vec<(String, String, i64, String, String)> {
        let connection = Connection::open(self.paths.database_path()).unwrap();
        let mut statement = connection
            .prepare(
                "SELECT profile_id, profile_version_id, version, normalized_name, content_digest
                 FROM active_agent_profiles ORDER BY profile_id",
            )
            .unwrap();
        statement
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    pub fn projection_metadata_row(&self) -> (i64, Option<String>, String) {
        Connection::open(self.paths.database_path())
            .unwrap()
            .query_row(
                "SELECT last_event_sequence, last_event_digest, projection_digest
                 FROM projection_metadata WHERE singleton = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap()
    }

    pub fn event_payloads(&self, kind: &str) -> Vec<String> {
        let connection = Connection::open(self.paths.database_path()).unwrap();
        let mut statement = connection
            .prepare(
                "SELECT payload_json FROM event_stream
                 WHERE event_type = ?1 ORDER BY sequence",
            )
            .unwrap();
        statement
            .query_map([kind], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    pub fn event_count(&self, kind: &str) -> i64 {
        Connection::open(self.paths.database_path())
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM event_stream WHERE event_type = ?1",
                [kind],
                |row| row.get(0),
            )
            .unwrap()
    }

    pub fn persisted_last_sequence(&self) -> u64 {
        Connection::open(self.paths.database_path())
            .unwrap()
            .query_row(
                "SELECT last_event_sequence FROM projection_metadata WHERE singleton = 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|value| value as u64)
            .unwrap()
    }

    pub fn max_event_sequence(&self) -> u64 {
        Connection::open(self.paths.database_path())
            .unwrap()
            .query_row("SELECT MAX(sequence) FROM event_stream", [], |row| {
                row.get::<_, i64>(0)
            })
            .map(|value| value as u64)
            .unwrap()
    }

    pub fn receipt_row(
        &self,
        command_id: ai_stock_forum::domain::CommandId,
    ) -> (String, String, String, String, String) {
        Connection::open(self.paths.database_path())
            .unwrap()
            .query_row(
                "SELECT command_fingerprint, request_json, capability, policy_decision, outcome_json FROM command_receipts WHERE command_id = ?1",
                [command_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .unwrap()
    }

    pub fn event_ref_ordinals(&self, command_id: ai_stock_forum::domain::CommandId) -> Vec<i64> {
        let connection = Connection::open(self.paths.database_path()).unwrap();
        let mut statement = connection
            .prepare("SELECT event_ordinal FROM command_event_refs WHERE command_id = ?1 ORDER BY event_ordinal")
            .unwrap();
        statement
            .query_map([command_id.to_string()], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    pub fn event_ref_rows(
        &self,
        command_id: ai_stock_forum::domain::CommandId,
    ) -> Vec<(i64, String)> {
        let connection = Connection::open(self.paths.database_path()).unwrap();
        let mut statement = connection
            .prepare(
                "SELECT event_ordinal, event_id FROM command_event_refs
                 WHERE command_id = ?1 ORDER BY event_ordinal",
            )
            .unwrap();
        statement
            .query_map([command_id.to_string()], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    pub fn install_projection_failure(&self) {
        Connection::open(self.paths.database_path())
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER fail_projection_metadata_update
                 BEFORE UPDATE ON projection_metadata BEGIN
                     SELECT RAISE(ABORT, 'injected projection failure');
                 END;",
            )
            .unwrap();
    }

    pub fn last_payload_json(&self) -> String {
        Connection::open(self.paths.database_path())
            .unwrap()
            .query_row(
                "SELECT payload_json FROM event_stream ORDER BY sequence DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap()
    }

    pub fn append_authoritative_help_event_without_projection(&self) {
        let mut database = Database::open(&self.paths).unwrap();
        let transaction = database.immediate_transaction().unwrap();
        EventRepository::append(
            &transaction,
            PendingEvent {
                event_id: EventId::from_uuid(Uuid::from_u128(u128::MAX - 1)),
                event_schema_version: EVENT_SCHEMA_VERSION,
                actor: Actor::System,
                occurred_at_ms: 1_800_000_000_000,
                correlation_id: CorrelationId::from_uuid(Uuid::from_u128(u128::MAX - 2)),
                causation_id: None,
                object: None,
                event: ApplicationEvent::HelpViewed,
            },
        )
        .unwrap();
        transaction.commit().unwrap();
    }

    pub fn shift_event_ref_to_ordinal_one(&self, command_id: ai_stock_forum::domain::CommandId) {
        Connection::open(self.paths.database_path())
            .unwrap()
            .execute_batch(&format!(
                "DROP TRIGGER command_event_refs_no_update;
                 UPDATE command_event_refs SET event_ordinal = 1 WHERE command_id = '{}';",
                command_id
            ))
            .unwrap();
    }

    pub fn mark_current_session_finished_in_database(&self) {
        Connection::open(self.paths.database_path())
            .unwrap()
            .execute(
                "UPDATE process_session_projection
                 SET ended_event_id = started_event_id, ended_at_ms = started_at_ms + 1,
                     end_reason = 'application_error'
                 WHERE session_id = ?1",
                [self.session_id().to_string()],
            )
            .unwrap();
    }

    pub fn tamper_receipt(
        &self,
        command_id: ai_stock_forum::domain::CommandId,
        tamper: ReceiptTamper,
    ) {
        let connection = Connection::open(self.paths.database_path()).unwrap();
        connection
            .execute_batch(
                "DROP TRIGGER IF EXISTS command_receipts_no_update;
                 DROP TRIGGER IF EXISTS command_event_refs_no_update;",
            )
            .unwrap();
        let command_id = command_id.to_string();

        match tamper {
            ReceiptTamper::NoncanonicalRequest => {
                let value: serde_json::Value = serde_json::from_str(
                    &connection
                        .query_row(
                            "SELECT request_json FROM command_receipts WHERE command_id = ?1",
                            [&command_id],
                            |row| row.get::<_, String>(0),
                        )
                        .unwrap(),
                )
                .unwrap();
                let json = serde_json::to_string_pretty(&value).unwrap();
                let fingerprint = ai_stock_forum::domain::sha256(json.as_bytes()).to_string();
                connection
                    .execute(
                        "UPDATE command_receipts SET request_json = ?1, command_fingerprint = ?2 WHERE command_id = ?3",
                        rusqlite::params![json, fingerprint, command_id],
                    )
                    .unwrap();
            }
            ReceiptTamper::NoncanonicalOutcome => {
                let value: serde_json::Value = serde_json::from_str(
                    &connection
                        .query_row(
                            "SELECT outcome_json FROM command_receipts WHERE command_id = ?1",
                            [&command_id],
                            |row| row.get::<_, String>(0),
                        )
                        .unwrap(),
                )
                .unwrap();
                let json = serde_json::to_string_pretty(&value).unwrap();
                connection
                    .execute(
                        "UPDATE command_receipts SET outcome_json = ?1 WHERE command_id = ?2",
                        rusqlite::params![json, command_id],
                    )
                    .unwrap();
            }
            ReceiptTamper::TypedInvalidRequest => {
                let mut value: serde_json::Value = serde_json::from_str(
                    &connection
                        .query_row(
                            "SELECT request_json FROM command_receipts WHERE command_id = ?1",
                            [&command_id],
                            |row| row.get::<_, String>(0),
                        )
                        .unwrap(),
                )
                .unwrap();
                value
                    .as_object_mut()
                    .unwrap()
                    .insert("unexpected".into(), serde_json::Value::Bool(true));
                let json = serde_json::to_string(&value).unwrap();
                let fingerprint = ai_stock_forum::domain::sha256(json.as_bytes()).to_string();
                connection
                    .execute(
                        "UPDATE command_receipts SET request_json = ?1, command_fingerprint = ?2 WHERE command_id = ?3",
                        rusqlite::params![json, fingerprint, command_id],
                    )
                    .unwrap();
            }
            ReceiptTamper::TypedInvalidOutcome => {
                let mut value: serde_json::Value = serde_json::from_str(
                    &connection
                        .query_row(
                            "SELECT outcome_json FROM command_receipts WHERE command_id = ?1",
                            [&command_id],
                            |row| row.get::<_, String>(0),
                        )
                        .unwrap(),
                )
                .unwrap();
                value
                    .as_object_mut()
                    .unwrap()
                    .insert("unexpected".into(), serde_json::Value::Bool(true));
                let json = serde_json::to_string(&value).unwrap();
                connection
                    .execute(
                        "UPDATE command_receipts SET outcome_json = ?1 WHERE command_id = ?2",
                        rusqlite::params![json, command_id],
                    )
                    .unwrap();
            }
            ReceiptTamper::FingerprintMismatch => {
                connection
                    .execute(
                        "UPDATE command_receipts SET command_fingerprint = ?1 WHERE command_id = ?2",
                        rusqlite::params!["b".repeat(64), command_id],
                    )
                    .unwrap();
            }
            ReceiptTamper::CapabilityMismatch => {
                connection
                    .execute(
                        "UPDATE command_receipts SET capability = 'help_read' WHERE command_id = ?1",
                        [&command_id],
                    )
                    .unwrap();
            }
            ReceiptTamper::PolicyDecisionMismatch => {
                connection
                    .execute(
                        "UPDATE command_receipts SET policy_decision = 'denied' WHERE command_id = ?1",
                        [&command_id],
                    )
                    .unwrap();
            }
            ReceiptTamper::EventRefOutcomeMismatch => {
                let replacement = connection
                    .query_row(
                        "SELECT event_id FROM event_stream
                         WHERE event_id NOT IN (SELECT event_id FROM command_event_refs)
                         ORDER BY sequence LIMIT 1",
                        [],
                        |row| row.get::<_, String>(0),
                    )
                    .unwrap();
                connection
                    .execute(
                        "UPDATE command_event_refs SET event_id = ?1 WHERE command_id = ?2",
                        rusqlite::params![replacement, command_id],
                    )
                    .unwrap();
            }
            ReceiptTamper::OrdinalGap => {
                connection
                    .execute(
                        "UPDATE command_event_refs SET event_ordinal = 1 WHERE command_id = ?1",
                        [&command_id],
                    )
                    .unwrap();
            }
            ReceiptTamper::MalformedReference => {
                connection
                    .pragma_update(None, "foreign_keys", "OFF")
                    .unwrap();
                connection
                    .execute(
                        "UPDATE command_event_refs SET event_id = 'not-an-event-id' WHERE command_id = ?1",
                        [&command_id],
                    )
                    .unwrap();
            }
        }
    }

    pub fn peer(&self) -> ApplicationWorker {
        self.service.worker().unwrap()
    }
}

pub fn record_test_episodic_summary(
    fixture: &mut PersistentFixture,
    profile: ai_stock_forum::agents::AgentProfileVersionRef,
    label: String,
    body: String,
    purpose_tags: Vec<String>,
    source_event_ids: Vec<EventId>,
) -> Result<ai_stock_forum::memory::EpisodicSummaryRef, ai_stock_forum::app::AppError> {
    record_test_episodic_summary_at(
        &fixture.paths,
        fixture.ids.as_ref(),
        fixture.clock.as_ref(),
        profile,
        label,
        body,
        purpose_tags,
        source_event_ids,
    )
}

#[allow(clippy::too_many_arguments)]
fn record_test_episodic_summary_at(
    paths: &AppPaths,
    ids: &TestIds,
    clock: &TestClock,
    profile: ai_stock_forum::agents::AgentProfileVersionRef,
    label: String,
    body: String,
    purpose_tags: Vec<String>,
    source_event_ids: Vec<EventId>,
) -> Result<ai_stock_forum::memory::EpisodicSummaryRef, ai_stock_forum::app::AppError> {
    use ai_stock_forum::{
        app::AppError,
        persistence::{MemoryRepository, load_exact_profile_version},
        recovery::reduce,
    };
    let connection =
        Connection::open(paths.database_path()).map_err(|_| PersistenceError::QueryFailed)?;
    let profile =
        load_exact_profile_version(&connection, &profile)?.ok_or(AppError::AgentProfileNotFound)?;

    let placeholder_event_id = EventId::from_uuid(Uuid::nil());
    let placeholder_source = EpisodicSourceRef::new(
        1,
        placeholder_event_id,
        "placeholder".into(),
        Digest::parse(&"0".repeat(64)).map_err(|_| RecoveryError::InvalidEventRecord)?,
    )?;
    let placeholder_sources = (0..source_event_ids.len())
        .map(|index| {
            EpisodicSourceRef::new(
                u64::try_from(index + 1).map_err(|_| RecoveryError::InvalidEventRecord)?,
                EventId::from_uuid(Uuid::from_u128(
                    u128::try_from(index + 1).map_err(|_| RecoveryError::InvalidEventRecord)?,
                )),
                placeholder_source.event_type().to_owned(),
                placeholder_source.event_digest().clone(),
            )
            .map_err(AppError::Domain)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let placeholder_creation_sequence = u64::try_from(source_event_ids.len())
        .map_err(|_| RecoveryError::InvalidEventRecord)?
        .checked_add(1)
        .ok_or(RecoveryError::InvalidEventRecord)?;
    EpisodicSummary::new(
        EpisodicSummaryId::from_uuid(Uuid::nil()),
        &profile,
        label.clone(),
        body.clone(),
        purpose_tags.clone(),
        placeholder_sources,
        0,
        placeholder_creation_sequence,
        placeholder_event_id,
    )?;

    drop(connection);
    let mut database = Database::open(paths).map_err(|_| PersistenceError::QueryFailed)?;
    let tx = database.immediate_transaction()?;
    let mut sources = Vec::with_capacity(source_event_ids.len());
    for event_id in source_event_ids {
        let row: Option<(i64, String, String)> = tx
            .transaction()
            .query_row(
                "SELECT sequence,event_type,event_digest FROM event_stream WHERE event_id=?1",
                [event_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|_| PersistenceError::QueryFailed)?;
        let (sequence, kind, digest) =
            row.ok_or(AppError::Recovery(RecoveryError::InvalidEventRecord))?;
        sources.push(EpisodicSourceRef::new(
            u64::try_from(sequence).map_err(|_| RecoveryError::InvalidEventRecord)?,
            event_id,
            kind,
            Digest::parse(&digest).map_err(|_| RecoveryError::InvalidEventRecord)?,
        )?);
    }
    let tail: i64 = tx
        .transaction()
        .query_row(
            "SELECT COALESCE(MAX(sequence),0) FROM event_stream",
            [],
            |row| row.get(0),
        )
        .map_err(|_| PersistenceError::QueryFailed)?;
    let sequence = u64::try_from(tail)
        .map_err(|_| RecoveryError::InvalidEventRecord)?
        .checked_add(1)
        .ok_or(RecoveryError::InvalidEventRecord)?;
    EpisodicSummary::new(
        EpisodicSummaryId::from_uuid(Uuid::nil()),
        &profile,
        label.clone(),
        body.clone(),
        purpose_tags.clone(),
        sources.clone(),
        0,
        sequence,
        placeholder_event_id,
    )?;
    let summary_id = EpisodicSummaryId::from_uuid(ids.next_uuid());
    let event_id = EventId::from_uuid(ids.next_uuid());
    let occurred_at_ms = clock.now_millis();
    let summary = EpisodicSummary::new(
        summary_id,
        &profile,
        label,
        body,
        purpose_tags,
        sources,
        occurred_at_ms,
        sequence,
        event_id,
    )?;
    let mut projection = ProjectionRepository::load_in(&tx)?;
    let committed = EventRepository::append(
        &tx,
        PendingEvent {
            event_id,
            event_schema_version: EVENT_SCHEMA_VERSION,
            actor: Actor::System,
            occurred_at_ms,
            correlation_id: CorrelationId::from_uuid(event_id.as_uuid()),
            causation_id: None,
            object: Some(ObjectRef::new(
                "episodic_summary",
                summary.reference().summary_id().to_string(),
                summary.reference().version(),
                summary.reference().content_digest().clone(),
            )?),
            event: ApplicationEvent::EpisodicSummaryRecorded {
                summary: summary.clone(),
            },
        },
    )?;
    if committed.sequence != sequence {
        return Err(AppError::Recovery(RecoveryError::InvalidEventRecord));
    }
    reduce(&mut projection, &committed)?;
    MemoryRepository::insert_episodic_summary(&tx, committed.sequence, &summary)?;
    ProjectionRepository::store(&tx, &projection)?;
    tx.commit()?;
    Ok(summary.reference())
}

pub fn app() -> TestApp {
    let temporary_directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temporary_directory.path());
    let clock = Arc::new(TestClock::new());
    let ids = Arc::new(TestIds::new());
    let service = ApplicationService::bootstrap(&paths, clock.clone(), ids.clone()).unwrap();
    let hook: Arc<dyn CommandTransactionHook> = Arc::new(TestCommandHook::passing());
    TestApp {
        _temporary_directory: temporary_directory,
        paths,
        clock,
        ids,
        service,
        policy: None,
        hook,
    }
}

pub fn app_with_policy(policy: Arc<dyn CommandPolicy>) -> TestApp {
    app_with_policy_and_hook(policy, Arc::new(TestCommandHook::passing()))
}

pub fn app_with_policy_and_hook(
    policy: Arc<dyn CommandPolicy>,
    hook: Arc<dyn CommandTransactionHook>,
) -> TestApp {
    let temporary_directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temporary_directory.path());
    let clock = Arc::new(TestClock::new());
    let ids = Arc::new(TestIds::new());
    let service = ApplicationService::bootstrap_with_dependencies(
        &paths,
        clock.clone(),
        ids.clone(),
        policy.clone(),
        hook.clone(),
    )
    .unwrap();
    TestApp {
        _temporary_directory: temporary_directory,
        paths,
        clock,
        ids,
        service,
        policy: Some(policy),
        hook,
    }
}
