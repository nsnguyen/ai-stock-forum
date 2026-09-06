DROP TRIGGER command_event_refs_no_update;
-- migration-boundary: drop_command_event_refs_no_update
DROP TRIGGER command_event_refs_no_delete;
-- migration-boundary: drop_command_event_refs_no_delete
DROP INDEX command_event_refs_event_idx;
-- migration-boundary: drop_command_event_refs_event_idx
ALTER TABLE command_event_refs RENAME TO command_event_refs_v1;
-- migration-boundary: rename_command_event_refs_v1

DROP TRIGGER command_receipts_no_update;
-- migration-boundary: drop_command_receipts_no_update
DROP TRIGGER command_receipts_no_delete;
-- migration-boundary: drop_command_receipts_no_delete
ALTER TABLE command_receipts RENAME TO command_receipts_v1;
-- migration-boundary: rename_command_receipts_v1

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
        'agent_profile_read', 'agent_profile_create', 'agent_profile_preview',
        'agent_profile_activate', 'shutdown', 'discussion_run', 'mcp_use',
        'engineering_job_run', 'git_merge', 'git_push', 'finance_recommendation'
    )),
    policy_decision TEXT NOT NULL CHECK (policy_decision IN (
        'granted', 'denied', 'denied_by_default', 'approval_required'
    )),
    outcome_json TEXT NOT NULL CHECK (json_valid(outcome_json))
) STRICT;
-- migration-boundary: create_command_receipts

INSERT INTO command_receipts (
    command_id, command_fingerprint, request_json, capability, policy_decision, outcome_json
)
SELECT command_id, command_fingerprint, request_json, capability, policy_decision, outcome_json
FROM command_receipts_v1;
-- migration-boundary: rebuild_command_receipts

CREATE TRIGGER command_receipts_no_update
BEFORE UPDATE ON command_receipts BEGIN
    SELECT RAISE(ABORT, 'command receipts are immutable');
END;
-- migration-boundary: create_command_receipts_no_update

CREATE TRIGGER command_receipts_no_delete
BEFORE DELETE ON command_receipts BEGIN
    SELECT RAISE(ABORT, 'command receipts are immutable');
END;
-- migration-boundary: create_command_receipts_no_delete

CREATE TABLE command_event_refs (
    command_id TEXT NOT NULL REFERENCES command_receipts(command_id),
    event_ordinal INTEGER NOT NULL CHECK (event_ordinal >= 0),
    event_id TEXT NOT NULL REFERENCES event_stream(event_id),
    PRIMARY KEY (command_id, event_ordinal)
) STRICT;
-- migration-boundary: create_command_event_refs

INSERT INTO command_event_refs (command_id, event_ordinal, event_id)
SELECT command_id, event_ordinal, event_id FROM command_event_refs_v1;
-- migration-boundary: rebuild_command_event_refs

CREATE UNIQUE INDEX command_event_refs_event_idx ON command_event_refs(event_id);
-- migration-boundary: create_command_event_refs_event_idx

CREATE TRIGGER command_event_refs_no_update
BEFORE UPDATE ON command_event_refs BEGIN
    SELECT RAISE(ABORT, 'command event refs are immutable');
END;
-- migration-boundary: create_command_event_refs_no_update

CREATE TRIGGER command_event_refs_no_delete
BEFORE DELETE ON command_event_refs BEGIN
    SELECT RAISE(ABORT, 'command event refs are immutable');
END;
-- migration-boundary: create_command_event_refs_no_delete

DROP TABLE command_event_refs_v1;
-- migration-boundary: drop_command_event_refs_v1
DROP TABLE command_receipts_v1;
-- migration-boundary: drop_command_receipts_v1

CREATE TABLE agent_profile_versions (
    profile_id TEXT NOT NULL,
    profile_version_id TEXT NOT NULL UNIQUE,
    version INTEGER NOT NULL CHECK (version >= 1),
    supersedes_version_id TEXT,
    template_id TEXT,
    template_version INTEGER,
    template_digest TEXT,
    role TEXT NOT NULL CHECK (role IN ('bull', 'bear', 'chief', 'engineering', 'custom')),
    display_name TEXT NOT NULL CHECK (
        length(CAST(display_name AS BLOB)) BETWEEN 1 AND 64
        AND instr(display_name, char(0)) = 0
    ),
    normalized_name TEXT NOT NULL CHECK (
        length(CAST(normalized_name AS BLOB)) BETWEEN 1 AND 256
        AND instr(normalized_name, char(0)) = 0
    ),
    memory_namespace_id TEXT NOT NULL CHECK (
        length(CAST(memory_namespace_id AS BLOB)) = 36
        AND instr(memory_namespace_id, char(0)) = 0
    ),
    policy_profile_ref TEXT NOT NULL CHECK (
        length(CAST(policy_profile_ref AS BLOB)) BETWEEN 1 AND 128
        AND instr(policy_profile_ref, char(0)) = 0
    ),
    content_digest TEXT NOT NULL CHECK (
        typeof(content_digest) = 'text'
        AND length(CAST(content_digest AS BLOB)) = 64
        AND instr(content_digest, char(0)) = 0
        AND content_digest NOT GLOB '*[^0-9a-f]*'
    ),
    payload_json BLOB NOT NULL CHECK (json_valid(CAST(payload_json AS TEXT))),
    source_event_sequence INTEGER NOT NULL UNIQUE CHECK (source_event_sequence >= 1),
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY (profile_id, version),
    UNIQUE (profile_id, profile_version_id),
    UNIQUE (profile_id, profile_version_id, content_digest),
    CHECK (
        (version = 1 AND supersedes_version_id IS NULL)
        OR (version > 1 AND supersedes_version_id IS NOT NULL)
    ),
    CHECK (
        (template_id IS NULL AND template_version IS NULL AND template_digest IS NULL)
        OR (
            template_id IS NOT NULL
            AND length(CAST(template_id AS BLOB)) BETWEEN 1 AND 128
            AND template_version >= 1
            AND template_digest IS NOT NULL
            AND length(CAST(template_digest AS BLOB)) = 64
            AND template_digest NOT GLOB '*[^0-9a-f]*'
        )
    ),
    FOREIGN KEY (profile_id, supersedes_version_id)
        REFERENCES agent_profile_versions(profile_id, profile_version_id)
        DEFERRABLE INITIALLY DEFERRED
) STRICT;
-- migration-boundary: create_agent_profile_versions

CREATE INDEX agent_profile_versions_history_idx
ON agent_profile_versions(profile_id, version DESC);
-- migration-boundary: create_agent_profile_versions_history_idx

CREATE TRIGGER agent_profile_namespace_insert_guard
BEFORE INSERT ON agent_profile_versions
WHEN EXISTS (
    SELECT 1 FROM agent_profile_versions existing
    WHERE existing.memory_namespace_id = NEW.memory_namespace_id
      AND existing.profile_id <> NEW.profile_id
) OR EXISTS (
    SELECT 1 FROM agent_profile_versions existing
    WHERE existing.profile_id = NEW.profile_id
      AND existing.memory_namespace_id <> NEW.memory_namespace_id
)
BEGIN
    SELECT RAISE(ABORT, 'agent_profile_namespace_conflict');
END;
-- migration-boundary: create_agent_profile_namespace_insert_guard

CREATE TABLE active_agent_profiles (
    profile_id TEXT PRIMARY KEY,
    profile_version_id TEXT NOT NULL UNIQUE,
    version INTEGER NOT NULL CHECK (version >= 1),
    normalized_name TEXT NOT NULL UNIQUE CHECK (
        length(CAST(normalized_name AS BLOB)) BETWEEN 1 AND 256
        AND instr(normalized_name, char(0)) = 0
    ),
    content_digest TEXT NOT NULL CHECK (
        length(CAST(content_digest AS BLOB)) = 64
        AND content_digest NOT GLOB '*[^0-9a-f]*'
    ),
    FOREIGN KEY (profile_id, profile_version_id, content_digest)
        REFERENCES agent_profile_versions(profile_id, profile_version_id, content_digest)
        DEFERRABLE INITIALLY DEFERRED
) STRICT;
-- migration-boundary: create_active_agent_profiles

CREATE INDEX active_agent_profiles_normalized_name_idx
ON active_agent_profiles(normalized_name);
-- migration-boundary: create_active_agent_profiles_normalized_name_idx

CREATE TRIGGER agent_profile_versions_no_update
BEFORE UPDATE ON agent_profile_versions BEGIN
    SELECT RAISE(ABORT, 'agent_profile_versions_immutable');
END;
-- migration-boundary: create_agent_profile_versions_no_update

CREATE TRIGGER agent_profile_versions_no_delete
BEFORE DELETE ON agent_profile_versions BEGIN
    SELECT RAISE(ABORT, 'agent_profile_versions_immutable');
END;
-- migration-boundary: create_agent_profile_versions_no_delete
