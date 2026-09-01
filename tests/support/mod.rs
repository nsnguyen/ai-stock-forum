use ai_stock_forum::{
    app::{ApplicationEvent, PendingEvent, EVENT_SCHEMA_VERSION},
    domain::{Actor, CorrelationId, EventId},
    persistence::{Database, EventRepository},
};
use tempfile::TempDir;
use uuid::Uuid;

pub struct DatabaseFixture {
    _temporary_directory: TempDir,
    pub database: Database,
    next_id: u128,
}

impl DatabaseFixture {
    pub fn append(&mut self, event: ApplicationEvent) -> ai_stock_forum::app::EventEnvelope {
        let pending = self.pending(event);
        let transaction = self.database.connection_mut().transaction().unwrap();
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
            Some("/secret".to_owned()),
            input,
        ),
    })
}
