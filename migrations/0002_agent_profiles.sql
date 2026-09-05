DROP TRIGGER command_event_refs_no_update;
DROP TRIGGER command_event_refs_no_delete;
DROP INDEX command_event_refs_event_idx;
ALTER TABLE command_event_refs RENAME TO command_event_refs_v1;

DROP TRIGGER command_receipts_no_update;
DROP TRIGGER command_receipts_no_delete;
ALTER TABLE command_receipts RENAME TO command_receipts_v1;

CREATE TABLE command_receipts (
    command_id TEXT PRIMARY KEY,
    command_fingerprint TEXT NOT NULL CHECK (
        typeof(command_fingerprint) = 'text'
        AND length(CAST(command_fingerprint AS BLOB)) = 64
        AND instr(command_fingerprint, char(0)) = 0
        AND command_fingerprint NOT GLOB '*[^0-9a-f]*'
    ),
    request_json TEXT NOT NULL CHECK (json_valid(request_json)),
    capability TEXT NOT NULL CHECK (capability IN (
        'help_read', 'status_read', 'setup_status_read', 'audit_read',
        'agent_profile_read', 'agent_profile_create', 'agent_profile_edit', 'shutdown',
        'discussion_run', 'mcp_use', 'engineering_job_run', 'git_merge', 'git_push',
        'finance_recommendation'
    )),
    policy_decision TEXT NOT NULL CHECK (policy_decision IN (
        'granted', 'denied', 'denied_by_default', 'approval_required'
    )),
    outcome_json TEXT NOT NULL CHECK (json_valid(outcome_json))
) STRICT;

INSERT INTO command_receipts (
    command_id, command_fingerprint, request_json, capability, policy_decision, outcome_json
)
SELECT command_id, command_fingerprint, request_json, capability, policy_decision, outcome_json
FROM command_receipts_v1;

CREATE TRIGGER command_receipts_no_update
BEFORE UPDATE ON command_receipts BEGIN
    SELECT RAISE(ABORT, 'command receipts are immutable');
END;

CREATE TRIGGER command_receipts_no_delete
BEFORE DELETE ON command_receipts BEGIN
    SELECT RAISE(ABORT, 'command receipts are immutable');
END;

CREATE TABLE command_event_refs (
    command_id TEXT NOT NULL REFERENCES command_receipts(command_id),
    event_ordinal INTEGER NOT NULL CHECK (event_ordinal >= 0),
    event_id TEXT NOT NULL REFERENCES event_stream(event_id),
    PRIMARY KEY (command_id, event_ordinal)
) STRICT;

INSERT INTO command_event_refs (command_id, event_ordinal, event_id)
SELECT command_id, event_ordinal, event_id FROM command_event_refs_v1;

CREATE UNIQUE INDEX command_event_refs_event_idx ON command_event_refs(event_id);

CREATE TRIGGER command_event_refs_no_update
BEFORE UPDATE ON command_event_refs BEGIN
    SELECT RAISE(ABORT, 'command event refs are immutable');
END;

CREATE TRIGGER command_event_refs_no_delete
BEFORE DELETE ON command_event_refs BEGIN
    SELECT RAISE(ABORT, 'command event refs are immutable');
END;

DROP TABLE command_event_refs_v1;
DROP TABLE command_receipts_v1;

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
