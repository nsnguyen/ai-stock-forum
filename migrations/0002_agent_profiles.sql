CREATE TABLE agent_profile_versions (
    profile_id TEXT NOT NULL,
    profile_version_id TEXT NOT NULL UNIQUE,
    version INTEGER NOT NULL CHECK (version >= 1),
    normalized_name TEXT NOT NULL,
    content_digest TEXT NOT NULL,
    payload_json BLOB NOT NULL,
    source_event_sequence INTEGER NOT NULL UNIQUE,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY (profile_id, version),
    UNIQUE (profile_id, profile_version_id)
) STRICT;

CREATE INDEX agent_profile_versions_history_idx
ON agent_profile_versions(profile_id, version DESC);

CREATE TABLE active_agent_profiles (
    profile_id TEXT PRIMARY KEY,
    profile_version_id TEXT NOT NULL UNIQUE,
    version INTEGER NOT NULL CHECK (version >= 1),
    normalized_name TEXT NOT NULL UNIQUE,
    readiness TEXT NOT NULL CHECK (readiness IN ('ready', 'not_ready')),
    FOREIGN KEY (profile_id, profile_version_id)
        REFERENCES agent_profile_versions(profile_id, profile_version_id)
        DEFERRABLE INITIALLY DEFERRED
) STRICT;

CREATE INDEX active_agent_profiles_normalized_name_idx
ON active_agent_profiles(normalized_name);

CREATE TRIGGER agent_profile_versions_no_update
BEFORE UPDATE ON agent_profile_versions BEGIN
    SELECT RAISE(ABORT, 'agent_profile_versions_immutable');
END;

CREATE TRIGGER agent_profile_versions_no_delete
BEFORE DELETE ON agent_profile_versions BEGIN
    SELECT RAISE(ABORT, 'agent_profile_versions_immutable');
END;
