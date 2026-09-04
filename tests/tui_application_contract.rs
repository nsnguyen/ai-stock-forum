mod support;

use std::sync::Arc;

use ai_stock_forum::{
    app::{
        ApplicationCommand, ApplicationEvent, ApplicationService, AuditLimit, EVENT_SCHEMA_VERSION,
        EventEnvelope, PendingEvent, ShutdownReason,
    },
    config::AppPaths,
    domain::{Actor, CorrelationId, EventId},
    persistence::{Database, EventRepository},
    setup::SetupStatus,
};
use tempfile::TempDir;
use uuid::Uuid;

struct SnapshotHarness {
    _temporary_directory: TempDir,
    paths: AppPaths,
    service: ApplicationService,
}

impl SnapshotHarness {
    fn new() -> Self {
        let temporary_directory = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temporary_directory.path());
        let clock = Arc::new(support::TestClock::new());
        let ids = Arc::new(support::TestIds::new());
        {
            let mut seed_service =
                ApplicationService::bootstrap(&paths, clock.clone(), ids.clone()).unwrap();
            for command in [
                ApplicationCommand::ShowHelp,
                ApplicationCommand::ShowStatus,
                ApplicationCommand::ShowSetupStatus,
                ApplicationCommand::ShowHelp,
            ] {
                seed_service.execute_user(command).unwrap();
            }
            seed_service.finish(ShutdownReason::UserQuit).unwrap();
        }
        let service = ApplicationService::bootstrap(&paths, clock, ids).unwrap();

        Self {
            _temporary_directory: temporary_directory,
            paths,
            service,
        }
    }

    fn persisted_events(&self) -> Vec<EventEnvelope> {
        let database = Database::open(&self.paths).unwrap();
        EventRepository::load_all(database.connection()).unwrap()
    }

    fn append_authoritative_help_event_without_projection(&self) {
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
}

#[test]
fn presentation_snapshot_is_typed_bounded_and_does_not_append_events() {
    let harness = SnapshotHarness::new();
    let limit = AuditLimit::new(3).unwrap();

    harness.append_authoritative_help_event_without_projection();
    let before = harness.persisted_events();

    let snapshot = harness.service.presentation_snapshot(limit).unwrap();
    let after = harness.persisted_events();

    assert_eq!(snapshot.installation_id, harness.service.installation_id());
    assert_eq!(snapshot.session_id, harness.service.session_id());
    assert_eq!(snapshot.setup_status, SetupStatus::NotStarted);
    assert_eq!(
        snapshot
            .recent_audit
            .iter()
            .map(|entry| (entry.sequence, entry.kind.as_str()))
        .collect::<Vec<_>>(),
        vec![
            (6, "help_viewed"),
            (7, "process_session_ended"),
            (8, "process_session_started"),
        ],
    );
    assert_eq!(before.last().unwrap().sequence, 9);
    assert_eq!(after, before);
}
