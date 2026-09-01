use crate::domain::{Sha256Digest, sha256};

pub const LATEST_SCHEMA_VERSION: u32 = 1;

pub(crate) const APPLICATION_ID: i64 = 0x4149_4653;
pub(crate) const SCHEMA_MIGRATIONS_SQL: &str = "
CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY CHECK (version > 0),
    checksum TEXT NOT NULL
) STRICT;
";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedMigration {
    pub(crate) version: u32,
    pub(crate) checksum: Sha256Digest,
}

impl AppliedMigration {
    pub const fn version(&self) -> u32 {
        self.version
    }

    pub fn checksum(&self) -> &Sha256Digest {
        &self.checksum
    }
}

pub(crate) struct Migration {
    pub(crate) version: u32,
    pub(crate) sql: &'static str,
}

impl Migration {
    pub(crate) fn checksum(&self) -> Sha256Digest {
        sha256(self.sql.as_bytes())
    }
}

pub(crate) fn ordered() -> [Migration; 1] {
    [Migration {
        version: 1,
        sql: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/migrations/0001_phase0.sql"
        )),
    }]
}
