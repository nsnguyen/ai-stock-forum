mod support;

use std::sync::Arc;

use ai_stock_forum::{
    app::{ApplicationService, AuditLimit},
    config::AppPaths,
    persistence::{Database, EventRepository},
    setup::SetupStatus,
};
use tempfile::TempDir;

struct SnapshotHarness {
    _temporary_directory: TempDir,
    paths: AppPaths,
    service: ApplicationService,
}

impl SnapshotHarness {
    fn new() -> Self {
        let temporary_directory = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temporary_directory.path());
        let service = ApplicationService::bootstrap(
            &paths,
            Arc::new(support::TestClock::new()),
            Arc::new(support::TestIds::new()),
        )
        .unwrap();

        Self {
            _temporary_directory: temporary_directory,
            paths,
            service,
        }
    }

    fn persisted_events(&self) -> Vec<ai_stock_forum::app::EventEnvelope> {
        let database = Database::open(&self.paths).unwrap();
        EventRepository::load_all(database.connection()).unwrap()
    }
}

#[test]
fn presentation_snapshot_is_typed_bounded_and_does_not_append_events() {
    let harness = SnapshotHarness::new();
    let before = harness.persisted_events();
    let limit = AuditLimit::new(3).unwrap();

    let snapshot = harness.service.presentation_snapshot(limit).unwrap();
    let after = harness.persisted_events();

    assert_eq!(snapshot.installation_id, harness.service.installation_id());
    assert_eq!(snapshot.session_id, harness.service.session_id());
    assert_eq!(snapshot.setup_status, SetupStatus::NotStarted);
    assert!(snapshot.recent_audit.len() <= usize::from(limit.get()));
    assert_eq!(after, before);
}
