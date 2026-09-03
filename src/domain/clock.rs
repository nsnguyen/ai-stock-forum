use std::time::{SystemTime, UNIX_EPOCH};

use uuid::Uuid;

pub trait Clock: Send + Sync {
    fn now_millis(&self) -> i64;
}

pub trait IdGenerator: Send + Sync {
    fn next_uuid(&self) -> Uuid;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_millis(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .try_into()
            .unwrap_or(i64::MAX)
    }
}

pub struct UuidGenerator;

impl IdGenerator for UuidGenerator {
    fn next_uuid(&self) -> Uuid {
        Uuid::new_v4()
    }
}
