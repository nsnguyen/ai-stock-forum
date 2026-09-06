use crate::domain::{Sha256Digest, sha256};

pub const LATEST_SCHEMA_VERSION: u32 = 3;

pub(crate) const APPLICATION_ID: i64 = 0x4149_4653;
pub(crate) const MIGRATION_BOUNDARY_PREFIX: &str = "-- migration-boundary:";
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

pub(crate) fn ordered() -> [Migration; 3] {
    [
        Migration {
            version: 1,
            sql: include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/migrations/0001_phase0.sql"
            )),
        },
        Migration {
            version: 2,
            sql: include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/migrations/0002_agent_profiles.sql"
            )),
        },
        Migration {
            version: 3,
            sql: include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/migrations/0003_declarative_skills.sql"
            )),
        },
    ]
}

pub(crate) fn migration_boundary_names(sql: &'static str) -> Vec<&'static str> {
    sql.lines()
        .filter_map(|line| line.trim().strip_prefix(MIGRATION_BOUNDARY_PREFIX))
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .collect()
}
