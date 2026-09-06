DROP TRIGGER command_event_refs_no_update;
-- migration-boundary: drop_command_event_refs_no_update
DROP TRIGGER command_event_refs_no_delete;
-- migration-boundary: drop_command_event_refs_no_delete
DROP INDEX command_event_refs_event_idx;
-- migration-boundary: drop_command_event_refs_event_idx
ALTER TABLE command_event_refs RENAME TO command_event_refs_v2;
-- migration-boundary: rename_command_event_refs_v2

DROP TRIGGER command_receipts_no_update;
-- migration-boundary: drop_command_receipts_no_update
DROP TRIGGER command_receipts_no_delete;
-- migration-boundary: drop_command_receipts_no_delete
ALTER TABLE command_receipts RENAME TO command_receipts_v2;
-- migration-boundary: rename_command_receipts_v2

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
        'agent_profile_activate', 'skill_read', 'skill_create', 'skill_version',
        'skill_assign', 'skill_unassign', 'shutdown', 'discussion_run', 'mcp_use',
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
FROM command_receipts_v2;
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
SELECT command_id, event_ordinal, event_id FROM command_event_refs_v2;
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

DROP TABLE command_event_refs_v2;
-- migration-boundary: drop_command_event_refs_v2
DROP TABLE command_receipts_v2;
-- migration-boundary: drop_command_receipts_v2

CREATE TABLE skill_versions (
    skill_id TEXT NOT NULL CHECK (
        length(CAST(skill_id AS BLOB)) = 36 AND instr(skill_id, char(0)) = 0
    ),
    skill_version_id TEXT NOT NULL UNIQUE CHECK (
        length(CAST(skill_version_id AS BLOB)) = 36
        AND instr(skill_version_id, char(0)) = 0
    ),
    version INTEGER NOT NULL CHECK (version >= 1),
    predecessor_version_id TEXT,
    display_name TEXT NOT NULL CHECK (
        length(CAST(display_name AS BLOB)) BETWEEN 1 AND 64
        AND instr(display_name, char(0)) = 0
    ),
    normalized_name TEXT NOT NULL CHECK (
        length(CAST(normalized_name AS BLOB)) BETWEEN 1 AND 256
        AND instr(normalized_name, char(0)) = 0
    ),
    content_digest TEXT NOT NULL CHECK (
        typeof(content_digest) = 'text'
        AND length(CAST(content_digest AS BLOB)) = 64
        AND instr(content_digest, char(0)) = 0
        AND content_digest NOT GLOB '*[^0-9a-f]*'
    ),
    content_json BLOB NOT NULL CHECK (
        typeof(content_json) = 'blob' AND json_valid(CAST(content_json AS TEXT))
    ),
    provenance_json BLOB NOT NULL CHECK (
        typeof(provenance_json) = 'blob' AND json_valid(CAST(provenance_json AS TEXT))
    ),
    created_at_ms INTEGER NOT NULL,
    record_digest TEXT NOT NULL CHECK (
        typeof(record_digest) = 'text'
        AND length(CAST(record_digest AS BLOB)) = 64
        AND instr(record_digest, char(0)) = 0
        AND record_digest NOT GLOB '*[^0-9a-f]*'
    ),
    record_json BLOB NOT NULL CHECK (
        typeof(record_json) = 'blob' AND json_valid(CAST(record_json AS TEXT))
    ),
    PRIMARY KEY (skill_id, version),
    UNIQUE (skill_id, skill_version_id),
    UNIQUE (
        skill_id, skill_version_id, version, normalized_name, content_digest, record_digest
    ),
    CHECK (
        (version = 1 AND predecessor_version_id IS NULL)
        OR (version > 1 AND predecessor_version_id IS NOT NULL)
    ),
    FOREIGN KEY (skill_id, predecessor_version_id)
        REFERENCES skill_versions(skill_id, skill_version_id)
        DEFERRABLE INITIALLY DEFERRED
) STRICT;
-- migration-boundary: create_skill_versions

CREATE INDEX skill_versions_history_idx ON skill_versions(skill_id, version ASC);
-- migration-boundary: create_skill_versions_history_idx

CREATE TRIGGER skill_versions_predecessor_guard
BEFORE INSERT ON skill_versions
WHEN NEW.version > 1 AND NOT EXISTS (
    SELECT 1 FROM skill_versions predecessor
    WHERE predecessor.skill_id = NEW.skill_id
      AND predecessor.version = NEW.version - 1
      AND predecessor.skill_version_id = NEW.predecessor_version_id
)
BEGIN
    SELECT RAISE(ABORT, 'skill_version_predecessor_mismatch');
END;
-- migration-boundary: create_skill_versions_predecessor_guard

CREATE TABLE active_skills (
    skill_id TEXT PRIMARY KEY,
    skill_version_id TEXT NOT NULL UNIQUE,
    version INTEGER NOT NULL CHECK (version >= 1),
    normalized_name TEXT NOT NULL UNIQUE CHECK (
        length(CAST(normalized_name AS BLOB)) BETWEEN 1 AND 256
        AND instr(normalized_name, char(0)) = 0
    ),
    content_digest TEXT NOT NULL CHECK (
        length(CAST(content_digest AS BLOB)) = 64
        AND content_digest NOT GLOB '*[^0-9a-f]*'
    ),
    record_digest TEXT NOT NULL CHECK (
        length(CAST(record_digest AS BLOB)) = 64
        AND record_digest NOT GLOB '*[^0-9a-f]*'
    ),
    FOREIGN KEY (
        skill_id, skill_version_id, version, normalized_name, content_digest, record_digest
    ) REFERENCES skill_versions (
        skill_id, skill_version_id, version, normalized_name, content_digest, record_digest
    ) DEFERRABLE INITIALLY DEFERRED
) STRICT;
-- migration-boundary: create_active_skills

CREATE INDEX active_skills_normalized_name_idx ON active_skills(normalized_name);
-- migration-boundary: create_active_skills_normalized_name_idx

CREATE TRIGGER skill_versions_no_update
BEFORE UPDATE ON skill_versions BEGIN
    SELECT RAISE(ABORT, 'skill_versions_immutable');
END;
-- migration-boundary: create_skill_versions_no_update

CREATE TRIGGER skill_versions_no_delete
BEFORE DELETE ON skill_versions BEGIN
    SELECT RAISE(ABORT, 'skill_versions_immutable');
END;
-- migration-boundary: create_skill_versions_no_delete
