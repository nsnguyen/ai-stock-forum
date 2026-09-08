DROP INDEX approval_records_status_idx;
-- migration-boundary: drop_approval_records_status_idx

ALTER TABLE approval_records RENAME TO approval_records_v3;
-- migration-boundary: rename_approval_records_v3

CREATE TABLE approval_records (
    approval_id TEXT PRIMARY KEY,
    action_kind TEXT NOT NULL,
    object_kind TEXT NOT NULL,
    object_id TEXT NOT NULL,
    object_version INTEGER NOT NULL CHECK (object_version > 0),
    object_digest TEXT NOT NULL,
    actor_kind TEXT NOT NULL,
    actor_id TEXT,
    status TEXT NOT NULL CHECK (
        status IN ('pending', 'accepted', 'rejected', 'expired', 'cancelled')
    ),
    created_at_ms INTEGER NOT NULL,
    expires_at_ms INTEGER,
    resolved_at_ms INTEGER,
    resolution_kind TEXT,
    resolution_event_id TEXT REFERENCES event_stream(event_id)
        DEFERRABLE INITIALLY DEFERRED,
    resolution_actor_kind TEXT,
    resolution_actor_id TEXT,
    CHECK (expires_at_ms IS NULL OR expires_at_ms > created_at_ms),
    CHECK (
        (resolution_actor_kind IS NULL AND resolution_actor_id IS NULL)
        OR (
            resolution_actor_kind IN ('human', 'system')
            AND resolution_actor_id IS NULL
        )
        OR (
            resolution_actor_kind = 'agent'
            AND resolution_actor_id IS NOT NULL
            AND typeof(resolution_actor_id) = 'text'
            AND length(CAST(resolution_actor_id AS BLOB)) = 36
            AND resolution_actor_id = lower(resolution_actor_id)
            AND substr(resolution_actor_id, 9, 1) = '-'
            AND substr(resolution_actor_id, 14, 1) = '-'
            AND substr(resolution_actor_id, 19, 1) = '-'
            AND substr(resolution_actor_id, 24, 1) = '-'
            AND length(replace(resolution_actor_id, '-', '')) = 32
            AND replace(resolution_actor_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        )
    ),
    CHECK (
        (
            status = 'pending'
            AND resolved_at_ms IS NULL
            AND resolution_kind IS NULL
        )
        OR (
            status <> 'pending'
            AND resolved_at_ms IS NOT NULL
            AND resolution_kind IS NOT NULL
        )
    )
) STRICT;
-- migration-boundary: create_approval_records

INSERT INTO approval_records (
    approval_id,
    action_kind,
    object_kind,
    object_id,
    object_version,
    object_digest,
    actor_kind,
    actor_id,
    status,
    created_at_ms,
    expires_at_ms,
    resolved_at_ms,
    resolution_kind,
    resolution_event_id,
    resolution_actor_kind,
    resolution_actor_id
)
SELECT
    approval_id,
    action_kind,
    object_kind,
    object_id,
    object_version,
    object_digest,
    actor_kind,
    actor_id,
    status,
    created_at_ms,
    expires_at_ms,
    resolved_at_ms,
    resolution_kind,
    resolution_event_id,
    NULL,
    NULL
FROM approval_records_v3
ORDER BY rowid;
-- migration-boundary: copy_approval_records_v3

DROP TABLE approval_records_v3;
-- migration-boundary: drop_approval_records_v3

CREATE INDEX approval_records_status_idx
ON approval_records (status, created_at_ms);
-- migration-boundary: create_approval_records_status_idx

CREATE TRIGGER approval_records_identity_guard
BEFORE UPDATE ON approval_records
WHEN NEW.approval_id IS NOT OLD.approval_id
  OR NEW.action_kind IS NOT OLD.action_kind
  OR NEW.object_kind IS NOT OLD.object_kind
  OR NEW.object_id IS NOT OLD.object_id
  OR NEW.object_version IS NOT OLD.object_version
  OR NEW.object_digest IS NOT OLD.object_digest
  OR NEW.actor_kind IS NOT OLD.actor_kind
  OR NEW.actor_id IS NOT OLD.actor_id
  OR NEW.created_at_ms IS NOT OLD.created_at_ms
  OR NEW.expires_at_ms IS NOT OLD.expires_at_ms
BEGIN
    SELECT RAISE(ABORT, 'approval_identity_immutable');
END;
-- migration-boundary: create_approval_records_identity_guard

CREATE TRIGGER approval_records_requester_insert_guard
BEFORE INSERT ON approval_records
WHEN NOT (
    (NEW.actor_kind IN ('human', 'system') AND NEW.actor_id IS NULL)
    OR (
        NEW.actor_kind = 'agent'
        AND NEW.actor_id IS NOT NULL
        AND typeof(NEW.actor_id) = 'text'
        AND length(CAST(NEW.actor_id AS BLOB)) = 36
        AND NEW.actor_id = lower(NEW.actor_id)
        AND substr(NEW.actor_id, 9, 1) = '-'
        AND substr(NEW.actor_id, 14, 1) = '-'
        AND substr(NEW.actor_id, 19, 1) = '-'
        AND substr(NEW.actor_id, 24, 1) = '-'
        AND length(replace(NEW.actor_id, '-', '')) = 32
        AND replace(NEW.actor_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    )
)
BEGIN
    SELECT RAISE(ABORT, 'approval_requester_invalid');
END;
-- migration-boundary: create_approval_records_requester_insert_guard

CREATE TRIGGER approval_records_pending_insert_guard
BEFORE INSERT ON approval_records
WHEN NEW.status = 'pending'
 AND (
     NEW.resolution_event_id IS NOT NULL
     OR NEW.resolution_actor_kind IS NOT NULL
     OR NEW.resolution_actor_id IS NOT NULL
 )
BEGIN
    SELECT RAISE(ABORT, 'approval_pending_resolution_invalid');
END;
-- migration-boundary: create_approval_records_pending_insert_guard

CREATE TRIGGER approval_records_memory_insert_guard
BEFORE INSERT ON approval_records
WHEN NEW.action_kind = 'memory_mutation'
 AND NOT (
     NEW.actor_kind = 'agent'
     AND NEW.actor_id IS NOT NULL
     AND NEW.object_kind = 'memory_proposal'
     AND NEW.object_version = 1
     AND typeof(NEW.object_digest) = 'text'
     AND length(CAST(NEW.object_digest AS BLOB)) = 64
     AND NEW.object_digest = lower(NEW.object_digest)
     AND NEW.object_digest NOT GLOB '*[^0-9a-f]*'
     AND NEW.expires_at_ms IS NULL
     AND NEW.status IN ('pending', 'accepted', 'rejected', 'expired')
     AND (
         NEW.status = 'pending'
         OR (
             NEW.resolution_actor_kind = 'human'
             AND NEW.resolution_actor_id IS NULL
             AND NEW.resolution_event_id IS NOT NULL
             AND NEW.resolution_kind = NEW.status
         )
     )
 )
BEGIN
    SELECT RAISE(ABORT, 'memory_approval_invalid');
END;
-- migration-boundary: create_approval_records_memory_insert_guard

CREATE TRIGGER approval_records_transition_guard
BEFORE UPDATE ON approval_records
WHEN NOT (
    OLD.status = 'pending'
    AND OLD.resolved_at_ms IS NULL
    AND OLD.resolution_kind IS NULL
    AND OLD.resolution_event_id IS NULL
    AND OLD.resolution_actor_kind IS NULL
    AND OLD.resolution_actor_id IS NULL
    AND NEW.status IN ('accepted', 'rejected', 'expired', 'cancelled')
    AND NEW.resolved_at_ms IS NOT NULL
    AND NEW.resolution_kind IS NOT NULL
    AND NEW.resolution_actor_kind IS NOT NULL
    AND (
        (NEW.actor_kind IN ('human', 'system') AND NEW.actor_id IS NULL)
        OR (
            NEW.actor_kind = 'agent'
            AND NEW.actor_id IS NOT NULL
            AND typeof(NEW.actor_id) = 'text'
            AND length(CAST(NEW.actor_id AS BLOB)) = 36
            AND NEW.actor_id = lower(NEW.actor_id)
            AND substr(NEW.actor_id, 9, 1) = '-'
            AND substr(NEW.actor_id, 14, 1) = '-'
            AND substr(NEW.actor_id, 19, 1) = '-'
            AND substr(NEW.actor_id, 24, 1) = '-'
            AND length(replace(NEW.actor_id, '-', '')) = 32
            AND replace(NEW.actor_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        )
    )
    AND (
        NEW.action_kind <> 'memory_mutation'
        OR (
            NEW.actor_kind = 'agent'
            AND NEW.object_kind = 'memory_proposal'
            AND NEW.object_version = 1
            AND typeof(NEW.object_digest) = 'text'
            AND length(CAST(NEW.object_digest AS BLOB)) = 64
            AND NEW.object_digest = lower(NEW.object_digest)
            AND NEW.object_digest NOT GLOB '*[^0-9a-f]*'
            AND NEW.expires_at_ms IS NULL
            AND NEW.status IN ('accepted', 'rejected', 'expired')
            AND NEW.resolution_actor_kind = 'human'
            AND NEW.resolution_actor_id IS NULL
            AND NEW.resolution_event_id IS NOT NULL
            AND NEW.resolution_kind = NEW.status
        )
    )
)
BEGIN
    SELECT RAISE(ABORT, 'approval_transition_invalid');
END;
-- migration-boundary: create_approval_records_transition_guard

CREATE TRIGGER approval_records_terminal_insert_guard
BEFORE INSERT ON approval_records
WHEN NEW.status <> 'pending' AND NEW.resolution_actor_kind IS NULL
BEGIN
    SELECT RAISE(ABORT, 'approval_terminal_resolver_missing');
END;
-- migration-boundary: create_approval_records_terminal_insert_guard

CREATE TRIGGER approval_records_no_delete
BEFORE DELETE ON approval_records BEGIN
    SELECT RAISE(ABORT, 'approval_records_immutable');
END;
-- migration-boundary: create_approval_records_no_delete

DROP TRIGGER command_event_refs_no_update;
-- migration-boundary: drop_command_event_refs_no_update_v4

DROP TRIGGER command_event_refs_no_delete;
-- migration-boundary: drop_command_event_refs_no_delete_v4

DROP INDEX command_event_refs_event_idx;
-- migration-boundary: drop_command_event_refs_event_idx_v4

ALTER TABLE command_event_refs RENAME TO command_event_refs_v3;
-- migration-boundary: rename_command_event_refs_v3

DROP TRIGGER command_receipts_no_update;
-- migration-boundary: drop_command_receipts_no_update_v4

DROP TRIGGER command_receipts_no_delete;
-- migration-boundary: drop_command_receipts_no_delete_v4

ALTER TABLE command_receipts RENAME TO command_receipts_v3;
-- migration-boundary: rename_command_receipts_v3

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
        'skill_assign', 'skill_unassign', 'memory_read', 'memory_preview',
        'memory_mutate', 'memory_propose', 'memory_resolve', 'shutdown',
        'discussion_run', 'mcp_use', 'engineering_job_run', 'git_merge',
        'git_push', 'finance_recommendation'
    )),
    policy_decision TEXT NOT NULL CHECK (policy_decision IN (
        'granted', 'denied', 'denied_by_default', 'approval_required'
    )),
    outcome_json TEXT NOT NULL CHECK (json_valid(outcome_json))
) STRICT;
-- migration-boundary: create_command_receipts_v4

INSERT INTO command_receipts (
    command_id,
    command_fingerprint,
    request_json,
    capability,
    policy_decision,
    outcome_json
)
SELECT
    command_id,
    command_fingerprint,
    request_json,
    capability,
    policy_decision,
    outcome_json
FROM command_receipts_v3
ORDER BY rowid;
-- migration-boundary: copy_command_receipts_v3

CREATE TRIGGER command_receipts_no_update
BEFORE UPDATE ON command_receipts BEGIN
    SELECT RAISE(ABORT, 'command receipts are immutable');
END;
-- migration-boundary: create_command_receipts_no_update_v4

CREATE TRIGGER command_receipts_no_delete
BEFORE DELETE ON command_receipts BEGIN
    SELECT RAISE(ABORT, 'command receipts are immutable');
END;
-- migration-boundary: create_command_receipts_no_delete_v4

CREATE TABLE command_event_refs (
    command_id TEXT NOT NULL REFERENCES command_receipts(command_id),
    event_ordinal INTEGER NOT NULL CHECK (event_ordinal >= 0),
    event_id TEXT NOT NULL REFERENCES event_stream(event_id),
    PRIMARY KEY (command_id, event_ordinal)
) STRICT;
-- migration-boundary: create_command_event_refs_v4

INSERT INTO command_event_refs (command_id, event_ordinal, event_id)
SELECT command_id, event_ordinal, event_id
FROM command_event_refs_v3
ORDER BY command_id, event_ordinal;
-- migration-boundary: copy_command_event_refs_v3

CREATE UNIQUE INDEX command_event_refs_event_idx
ON command_event_refs (event_id);
-- migration-boundary: create_command_event_refs_event_idx_v4

CREATE TRIGGER command_event_refs_no_update
BEFORE UPDATE ON command_event_refs BEGIN
    SELECT RAISE(ABORT, 'command event refs are immutable');
END;
-- migration-boundary: create_command_event_refs_no_update_v4

CREATE TRIGGER command_event_refs_no_delete
BEFORE DELETE ON command_event_refs BEGIN
    SELECT RAISE(ABORT, 'command event refs are immutable');
END;
-- migration-boundary: create_command_event_refs_no_delete_v4

DROP TABLE command_event_refs_v3;
-- migration-boundary: drop_command_event_refs_v3

DROP TABLE command_receipts_v3;
-- migration-boundary: drop_command_receipts_v3
CREATE UNIQUE INDEX agent_profile_versions_memory_ref_idx
ON agent_profile_versions (
    profile_id,
    profile_version_id,
    version,
    content_digest,
    memory_namespace_id
);
-- migration-boundary: create_agent_profile_versions_memory_ref_idx

CREATE UNIQUE INDEX event_stream_memory_source_ref_idx
ON event_stream (sequence, event_id, event_type, event_digest);
-- migration-boundary: create_event_stream_memory_source_ref_idx
CREATE TABLE memory_entry_versions (
    memory_namespace_id TEXT NOT NULL CHECK (
        typeof(memory_namespace_id) = 'text'
        AND length(CAST(memory_namespace_id AS BLOB)) = 36
        AND memory_namespace_id = lower(memory_namespace_id)
        AND substr(memory_namespace_id, 9, 1) = '-'
        AND substr(memory_namespace_id, 14, 1) = '-'
        AND substr(memory_namespace_id, 19, 1) = '-'
        AND substr(memory_namespace_id, 24, 1) = '-'
        AND length(replace(memory_namespace_id, '-', '')) = 32
        AND replace(memory_namespace_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    entry_id TEXT NOT NULL CHECK (
        typeof(entry_id) = 'text'
        AND length(CAST(entry_id AS BLOB)) = 36
        AND entry_id = lower(entry_id)
        AND substr(entry_id, 9, 1) = '-'
        AND substr(entry_id, 14, 1) = '-'
        AND substr(entry_id, 19, 1) = '-'
        AND substr(entry_id, 24, 1) = '-'
        AND length(replace(entry_id, '-', '')) = 32
        AND replace(entry_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    entry_version_id TEXT NOT NULL UNIQUE CHECK (
        typeof(entry_version_id) = 'text'
        AND length(CAST(entry_version_id AS BLOB)) = 36
        AND entry_version_id = lower(entry_version_id)
        AND substr(entry_version_id, 9, 1) = '-'
        AND substr(entry_version_id, 14, 1) = '-'
        AND substr(entry_version_id, 19, 1) = '-'
        AND substr(entry_version_id, 24, 1) = '-'
        AND length(replace(entry_version_id, '-', '')) = 32
        AND replace(entry_version_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    version INTEGER NOT NULL CHECK (version >= 1),
    predecessor_version_id TEXT,
    display_key TEXT NOT NULL CHECK (
        length(CAST(display_key AS BLOB)) BETWEEN 1 AND 96
        AND instr(display_key, char(0)) = 0
    ),
    normalized_key TEXT NOT NULL CHECK (
        length(CAST(normalized_key AS BLOB)) >= 1
        AND instr(normalized_key, char(0)) = 0
    ),
    state TEXT NOT NULL CHECK (state IN ('present', 'deleted')),
    value_text TEXT,
    value_bytes INTEGER NOT NULL CHECK (value_bytes >= 0),
    purpose_tags_json BLOB NOT NULL CHECK (
        typeof(purpose_tags_json) = 'blob'
        AND json_valid(CAST(purpose_tags_json AS TEXT))
        AND json_type(CAST(purpose_tags_json AS TEXT)) = 'array'
        AND json_array_length(CAST(purpose_tags_json AS TEXT)) BETWEEN 0 AND 8
    ),
    created_by_kind TEXT NOT NULL,
    created_by_id TEXT,
    created_at_ms INTEGER NOT NULL,
    accepted_proposal_id TEXT,
    accepted_proposal_version INTEGER,
    accepted_proposal_digest TEXT,
    plaintext_validation_version INTEGER NOT NULL CHECK (
        plaintext_validation_version = 1
    ),
    creation_event_sequence INTEGER NOT NULL CHECK (creation_event_sequence >= 1),
    creation_event_id TEXT NOT NULL REFERENCES event_stream(event_id)
        DEFERRABLE INITIALLY DEFERRED,
    content_digest TEXT NOT NULL CHECK (
        typeof(content_digest) = 'text'
        AND length(CAST(content_digest AS BLOB)) = 64
        AND content_digest = lower(content_digest)
        AND content_digest NOT GLOB '*[^0-9a-f]*'
    ),
    record_digest TEXT NOT NULL CHECK (
        typeof(record_digest) = 'text'
        AND length(CAST(record_digest AS BLOB)) = 64
        AND record_digest = lower(record_digest)
        AND record_digest NOT GLOB '*[^0-9a-f]*'
    ),
    record_json BLOB NOT NULL CHECK (
        typeof(record_json) = 'blob'
        AND json_valid(CAST(record_json AS TEXT))
    ),
    PRIMARY KEY (entry_id, version),
    UNIQUE (entry_id, entry_version_id),
    UNIQUE (
        memory_namespace_id,
        normalized_key,
        entry_id,
        entry_version_id,
        version,
        state,
        content_digest
    ),
    CHECK (
        (version = 1 AND predecessor_version_id IS NULL)
        OR (version > 1 AND predecessor_version_id IS NOT NULL)
    ),
    CHECK (
        (state = 'present'
         AND value_text IS NOT NULL
         AND value_bytes = length(CAST(value_text AS BLOB))
         AND value_bytes BETWEEN 1 AND 4096)
        OR (state = 'deleted'
            AND value_text IS NULL
            AND value_bytes = 0
            AND json_array_length(CAST(purpose_tags_json AS TEXT)) = 0)
    ),
    CHECK (created_by_kind = 'human' AND created_by_id IS NULL),
    CHECK (
        (accepted_proposal_id IS NULL
         AND accepted_proposal_version IS NULL
         AND accepted_proposal_digest IS NULL)
        OR (accepted_proposal_id IS NOT NULL
            AND accepted_proposal_version = 1
            AND accepted_proposal_digest IS NOT NULL
            AND length(CAST(accepted_proposal_digest AS BLOB)) = 64
            AND accepted_proposal_digest = lower(accepted_proposal_digest)
            AND accepted_proposal_digest NOT GLOB '*[^0-9a-f]*')
    ),
    FOREIGN KEY (entry_id, predecessor_version_id)
        REFERENCES memory_entry_versions(entry_id, entry_version_id)
        DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY (
        memory_namespace_id,
        normalized_key,
        accepted_proposal_id,
        accepted_proposal_version,
        accepted_proposal_digest
    ) REFERENCES memory_proposals (
        memory_namespace_id,
        normalized_key,
        proposal_id,
        version,
        content_digest
    ) DEFERRABLE INITIALLY DEFERRED
) STRICT;
-- migration-boundary: create_memory_entry_versions

CREATE INDEX memory_entry_versions_history_idx
ON memory_entry_versions (memory_namespace_id, normalized_key, version DESC);
-- migration-boundary: create_memory_entry_versions_history_idx

CREATE TRIGGER memory_entry_versions_identity_guard
BEFORE INSERT ON memory_entry_versions
WHEN EXISTS (
    SELECT 1
    FROM memory_entry_versions existing
    WHERE (existing.memory_namespace_id = NEW.memory_namespace_id
           AND existing.normalized_key = NEW.normalized_key
           AND existing.entry_id <> NEW.entry_id)
       OR (existing.entry_id = NEW.entry_id
           AND (existing.memory_namespace_id <> NEW.memory_namespace_id
                OR existing.normalized_key <> NEW.normalized_key))
)
BEGIN
    SELECT RAISE(ABORT, 'memory_entry_identity_mismatch');
END;
-- migration-boundary: create_memory_entry_versions_identity_guard

CREATE TRIGGER memory_entry_versions_predecessor_guard
BEFORE INSERT ON memory_entry_versions
WHEN NEW.version > 1 AND NOT EXISTS (
    SELECT 1
    FROM memory_entry_versions predecessor
    WHERE predecessor.memory_namespace_id = NEW.memory_namespace_id
      AND predecessor.normalized_key = NEW.normalized_key
      AND predecessor.entry_id = NEW.entry_id
      AND predecessor.entry_version_id = NEW.predecessor_version_id
      AND predecessor.version = NEW.version - 1
)
BEGIN
    SELECT RAISE(ABORT, 'memory_entry_predecessor_mismatch');
END;
-- migration-boundary: create_memory_entry_versions_predecessor_guard

CREATE TRIGGER memory_entry_versions_no_update
BEFORE UPDATE ON memory_entry_versions BEGIN
    SELECT RAISE(ABORT, 'memory_entry_versions_immutable');
END;
-- migration-boundary: create_memory_entry_versions_no_update

CREATE TRIGGER memory_entry_versions_no_delete
BEFORE DELETE ON memory_entry_versions BEGIN
    SELECT RAISE(ABORT, 'memory_entry_versions_immutable');
END;
-- migration-boundary: create_memory_entry_versions_no_delete

CREATE TABLE current_memory_entries (
    memory_namespace_id TEXT NOT NULL,
    normalized_key TEXT NOT NULL,
    entry_id TEXT NOT NULL,
    entry_version_id TEXT NOT NULL UNIQUE,
    version INTEGER NOT NULL CHECK (version >= 1),
    state TEXT NOT NULL CHECK (state IN ('present', 'deleted')),
    content_digest TEXT NOT NULL,
    PRIMARY KEY (memory_namespace_id, normalized_key),
    FOREIGN KEY (
        memory_namespace_id,
        normalized_key,
        entry_id,
        entry_version_id,
        version,
        state,
        content_digest
    ) REFERENCES memory_entry_versions (
        memory_namespace_id,
        normalized_key,
        entry_id,
        entry_version_id,
        version,
        state,
        content_digest
    ) DEFERRABLE INITIALLY DEFERRED
) STRICT;
-- migration-boundary: create_current_memory_entries

CREATE INDEX current_memory_entries_list_idx
ON current_memory_entries (memory_namespace_id, state, normalized_key, entry_id);
-- migration-boundary: create_current_memory_entries_list_idx
CREATE TABLE memory_proposals (
    proposal_id TEXT PRIMARY KEY CHECK (
        typeof(proposal_id) = 'text'
        AND length(CAST(proposal_id AS BLOB)) = 36
        AND proposal_id = lower(proposal_id)
        AND substr(proposal_id, 9, 1) = '-'
        AND substr(proposal_id, 14, 1) = '-'
        AND substr(proposal_id, 19, 1) = '-'
        AND substr(proposal_id, 24, 1) = '-'
        AND length(replace(proposal_id, '-', '')) = 32
        AND replace(proposal_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    version INTEGER NOT NULL CHECK (version = 1),
    proposer_profile_id TEXT NOT NULL,
    proposer_profile_version_id TEXT NOT NULL,
    proposer_profile_version INTEGER NOT NULL CHECK (proposer_profile_version >= 1),
    proposer_profile_digest TEXT NOT NULL CHECK (
        typeof(proposer_profile_digest) = 'text'
        AND length(CAST(proposer_profile_digest AS BLOB)) = 64
        AND proposer_profile_digest = lower(proposer_profile_digest)
        AND proposer_profile_digest NOT GLOB '*[^0-9a-f]*'
    ),
    memory_namespace_id TEXT NOT NULL,
    operation TEXT NOT NULL CHECK (operation IN ('set', 'delete')),
    display_key TEXT NOT NULL CHECK (
        length(CAST(display_key AS BLOB)) BETWEEN 1 AND 96
        AND instr(display_key, char(0)) = 0
    ),
    normalized_key TEXT NOT NULL CHECK (
        length(CAST(normalized_key AS BLOB)) >= 1
        AND instr(normalized_key, char(0)) = 0
    ),
    expected_kind TEXT NOT NULL CHECK (
        expected_kind IN ('absent', 'present', 'deleted')
    ),
    expected_entry_id TEXT,
    expected_entry_version_id TEXT,
    expected_entry_version INTEGER,
    expected_entry_digest TEXT,
    candidate_value TEXT,
    candidate_value_bytes INTEGER,
    candidate_purpose_tags_json BLOB,
    rationale TEXT NOT NULL CHECK (
        length(CAST(rationale AS BLOB)) BETWEEN 0 AND 512
        AND instr(rationale, char(0)) = 0
    ),
    plaintext_validation_version INTEGER NOT NULL CHECK (
        plaintext_validation_version = 1
    ),
    created_at_ms INTEGER NOT NULL,
    creation_event_sequence INTEGER NOT NULL CHECK (creation_event_sequence >= 1),
    creation_event_id TEXT NOT NULL REFERENCES event_stream(event_id)
        DEFERRABLE INITIALLY DEFERRED,
    approval_id TEXT NOT NULL UNIQUE REFERENCES approval_records(approval_id)
        DEFERRABLE INITIALLY DEFERRED,
    content_digest TEXT NOT NULL CHECK (
        typeof(content_digest) = 'text'
        AND length(CAST(content_digest AS BLOB)) = 64
        AND content_digest = lower(content_digest)
        AND content_digest NOT GLOB '*[^0-9a-f]*'
    ),
    record_digest TEXT NOT NULL CHECK (
        typeof(record_digest) = 'text'
        AND length(CAST(record_digest AS BLOB)) = 64
        AND record_digest = lower(record_digest)
        AND record_digest NOT GLOB '*[^0-9a-f]*'
    ),
    record_json BLOB NOT NULL CHECK (
        typeof(record_json) = 'blob'
        AND json_valid(CAST(record_json AS TEXT))
    ),
    UNIQUE (proposal_id, version, content_digest),
    UNIQUE (proposal_id, version, content_digest, approval_id),
    UNIQUE (
        memory_namespace_id,
        normalized_key,
        proposal_id,
        version,
        content_digest
    ),
    CHECK (
        (expected_kind = 'absent'
         AND expected_entry_id IS NULL
         AND expected_entry_version_id IS NULL
         AND expected_entry_version IS NULL
         AND expected_entry_digest IS NULL)
        OR (expected_kind IN ('present', 'deleted')
            AND expected_entry_id IS NOT NULL
            AND expected_entry_version_id IS NOT NULL
            AND expected_entry_version >= 1
            AND expected_entry_digest IS NOT NULL
            AND length(CAST(expected_entry_digest AS BLOB)) = 64
            AND expected_entry_digest = lower(expected_entry_digest)
            AND expected_entry_digest NOT GLOB '*[^0-9a-f]*')
    ),
    CHECK (
        (operation = 'set'
         AND candidate_value IS NOT NULL
         AND candidate_value_bytes = length(CAST(candidate_value AS BLOB))
         AND candidate_value_bytes BETWEEN 1 AND 4096
         AND candidate_purpose_tags_json IS NOT NULL
         AND typeof(candidate_purpose_tags_json) = 'blob'
         AND json_valid(CAST(candidate_purpose_tags_json AS TEXT))
         AND json_type(CAST(candidate_purpose_tags_json AS TEXT)) = 'array'
         AND json_array_length(CAST(candidate_purpose_tags_json AS TEXT)) BETWEEN 0 AND 8)
        OR (operation = 'delete'
            AND expected_kind = 'present'
            AND candidate_value IS NULL
            AND candidate_value_bytes IS NULL
            AND candidate_purpose_tags_json IS NULL)
    ),
    FOREIGN KEY (
        proposer_profile_id,
        proposer_profile_version_id,
        proposer_profile_version,
        proposer_profile_digest,
        memory_namespace_id
    ) REFERENCES agent_profile_versions (
        profile_id,
        profile_version_id,
        version,
        content_digest,
        memory_namespace_id
    ) DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY (
        memory_namespace_id,
        normalized_key,
        expected_entry_id,
        expected_entry_version_id,
        expected_entry_version,
        expected_kind,
        expected_entry_digest
    ) REFERENCES memory_entry_versions (
        memory_namespace_id,
        normalized_key,
        entry_id,
        entry_version_id,
        version,
        state,
        content_digest
    ) DEFERRABLE INITIALLY DEFERRED
) STRICT;
-- migration-boundary: create_memory_proposals

CREATE INDEX memory_proposals_pending_order_idx
ON memory_proposals (memory_namespace_id, created_at_ms, proposal_id);
-- migration-boundary: create_memory_proposals_pending_order_idx

CREATE TRIGGER memory_proposals_no_update
BEFORE UPDATE ON memory_proposals BEGIN
    SELECT RAISE(ABORT, 'memory_proposals_immutable');
END;
-- migration-boundary: create_memory_proposals_no_update

CREATE TRIGGER memory_proposals_no_delete
BEFORE DELETE ON memory_proposals BEGIN
    SELECT RAISE(ABORT, 'memory_proposals_immutable');
END;
-- migration-boundary: create_memory_proposals_no_delete

CREATE TABLE memory_proposal_resolutions (
    proposal_id TEXT PRIMARY KEY,
    proposal_version INTEGER NOT NULL CHECK (proposal_version = 1),
    proposal_content_digest TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('accepted', 'rejected', 'expired')),
    approval_id TEXT NOT NULL UNIQUE,
    resolved_by_kind TEXT NOT NULL CHECK (resolved_by_kind = 'human'),
    resolved_by_id TEXT CHECK (resolved_by_id IS NULL),
    resolved_at_ms INTEGER NOT NULL,
    resolution_event_sequence INTEGER NOT NULL CHECK (resolution_event_sequence >= 1),
    resolution_event_id TEXT NOT NULL REFERENCES event_stream(event_id)
        DEFERRABLE INITIALLY DEFERRED,
    resolution_json BLOB NOT NULL CHECK (
        typeof(resolution_json) = 'blob'
        AND json_valid(CAST(resolution_json AS TEXT))
    ),
    UNIQUE (proposal_id, status, resolution_event_id),
    FOREIGN KEY (
        proposal_id,
        proposal_version,
        proposal_content_digest,
        approval_id
    ) REFERENCES memory_proposals (
        proposal_id,
        version,
        content_digest,
        approval_id
    ) DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY (approval_id) REFERENCES approval_records(approval_id)
        DEFERRABLE INITIALLY DEFERRED
) STRICT;
-- migration-boundary: create_memory_proposal_resolutions

CREATE INDEX memory_proposal_resolutions_event_idx
ON memory_proposal_resolutions (resolution_event_id, proposal_id);
-- migration-boundary: create_memory_proposal_resolutions_event_idx

CREATE TRIGGER memory_proposal_resolutions_no_update
BEFORE UPDATE ON memory_proposal_resolutions BEGIN
    SELECT RAISE(ABORT, 'memory_proposal_resolutions_immutable');
END;
-- migration-boundary: create_memory_proposal_resolutions_no_update

CREATE TRIGGER memory_proposal_resolutions_no_delete
BEFORE DELETE ON memory_proposal_resolutions BEGIN
    SELECT RAISE(ABORT, 'memory_proposal_resolutions_immutable');
END;
-- migration-boundary: create_memory_proposal_resolutions_no_delete

CREATE TABLE current_memory_proposal_status (
    proposal_id TEXT PRIMARY KEY,
    proposal_version INTEGER NOT NULL CHECK (proposal_version = 1),
    proposal_content_digest TEXT NOT NULL,
    memory_namespace_id TEXT NOT NULL,
    normalized_key TEXT NOT NULL,
    status TEXT NOT NULL CHECK (
        status IN ('pending', 'accepted', 'rejected', 'expired')
    ),
    resolution_event_id TEXT,
    created_at_ms INTEGER NOT NULL,
    CHECK (
        (status = 'pending' AND resolution_event_id IS NULL)
        OR (status <> 'pending' AND resolution_event_id IS NOT NULL)
    ),
    FOREIGN KEY (
        memory_namespace_id,
        normalized_key,
        proposal_id,
        proposal_version,
        proposal_content_digest
    ) REFERENCES memory_proposals (
        memory_namespace_id,
        normalized_key,
        proposal_id,
        version,
        content_digest
    ) DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY (proposal_id, status, resolution_event_id)
        REFERENCES memory_proposal_resolutions(
            proposal_id,
            status,
            resolution_event_id
        ) DEFERRABLE INITIALLY DEFERRED
) STRICT;
-- migration-boundary: create_current_memory_proposal_status

CREATE INDEX current_memory_proposals_pending_idx
ON current_memory_proposal_status (
    memory_namespace_id,
    status,
    created_at_ms ASC,
    proposal_id ASC
);
-- migration-boundary: create_current_memory_proposals_pending_idx

CREATE INDEX current_memory_proposals_all_idx
ON current_memory_proposal_status (
    memory_namespace_id,
    created_at_ms DESC,
    proposal_id ASC
);
-- migration-boundary: create_current_memory_proposals_all_idx
CREATE TABLE episodic_summaries (
    summary_id TEXT PRIMARY KEY CHECK (
        typeof(summary_id) = 'text'
        AND length(CAST(summary_id AS BLOB)) = 36
        AND summary_id = lower(summary_id)
        AND substr(summary_id, 9, 1) = '-'
        AND substr(summary_id, 14, 1) = '-'
        AND substr(summary_id, 19, 1) = '-'
        AND substr(summary_id, 24, 1) = '-'
        AND length(replace(summary_id, '-', '')) = 32
        AND replace(summary_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    version INTEGER NOT NULL CHECK (version = 1),
    memory_namespace_id TEXT NOT NULL,
    profile_id TEXT NOT NULL,
    profile_version_id TEXT NOT NULL,
    profile_version INTEGER NOT NULL CHECK (profile_version >= 1),
    profile_content_digest TEXT NOT NULL,
    label TEXT NOT NULL CHECK (
        length(CAST(label AS BLOB)) BETWEEN 1 AND 128
        AND instr(label, char(0)) = 0
    ),
    body TEXT NOT NULL CHECK (
        length(CAST(body AS BLOB)) BETWEEN 1 AND 8192
        AND instr(body, char(0)) = 0
    ),
    purpose_tags_json BLOB NOT NULL CHECK (
        typeof(purpose_tags_json) = 'blob'
        AND json_valid(CAST(purpose_tags_json AS TEXT))
        AND json_type(CAST(purpose_tags_json AS TEXT)) = 'array'
        AND json_array_length(CAST(purpose_tags_json AS TEXT)) BETWEEN 0 AND 8
    ),
    source_count INTEGER NOT NULL CHECK (source_count BETWEEN 1 AND 128),
    plaintext_validation_version INTEGER NOT NULL CHECK (
        plaintext_validation_version = 1
    ),
    created_at_ms INTEGER NOT NULL,
    creation_event_sequence INTEGER NOT NULL UNIQUE CHECK (
        creation_event_sequence >= 1
    ),
    creation_event_id TEXT NOT NULL UNIQUE REFERENCES event_stream(event_id)
        DEFERRABLE INITIALLY DEFERRED,
    source_set_digest TEXT NOT NULL CHECK (
        typeof(source_set_digest) = 'text'
        AND length(CAST(source_set_digest AS BLOB)) = 64
        AND source_set_digest = lower(source_set_digest)
        AND source_set_digest NOT GLOB '*[^0-9a-f]*'
    ),
    content_digest TEXT NOT NULL CHECK (
        typeof(content_digest) = 'text'
        AND length(CAST(content_digest AS BLOB)) = 64
        AND content_digest = lower(content_digest)
        AND content_digest NOT GLOB '*[^0-9a-f]*'
    ),
    record_digest TEXT NOT NULL CHECK (
        typeof(record_digest) = 'text'
        AND length(CAST(record_digest AS BLOB)) = 64
        AND record_digest = lower(record_digest)
        AND record_digest NOT GLOB '*[^0-9a-f]*'
    ),
    record_json BLOB NOT NULL CHECK (
        typeof(record_json) = 'blob'
        AND json_valid(CAST(record_json AS TEXT))
    ),
    UNIQUE (
        summary_id,
        version,
        memory_namespace_id,
        profile_id,
        profile_version_id,
        profile_version,
        profile_content_digest,
        creation_event_sequence,
        creation_event_id,
        source_set_digest,
        content_digest
    ),
    FOREIGN KEY (
        profile_id,
        profile_version_id,
        profile_version,
        profile_content_digest,
        memory_namespace_id
    ) REFERENCES agent_profile_versions (
        profile_id,
        profile_version_id,
        version,
        content_digest,
        memory_namespace_id
    ) DEFERRABLE INITIALLY DEFERRED
) STRICT;
-- migration-boundary: create_episodic_summaries

CREATE INDEX episodic_summaries_list_idx
ON episodic_summaries (memory_namespace_id, created_at_ms DESC, summary_id ASC);
-- migration-boundary: create_episodic_summaries_list_idx

CREATE TABLE episodic_summary_sources (
    summary_id TEXT NOT NULL REFERENCES episodic_summaries(summary_id)
        DEFERRABLE INITIALLY DEFERRED,
    source_ordinal INTEGER NOT NULL CHECK (source_ordinal BETWEEN 0 AND 127),
    event_sequence INTEGER NOT NULL CHECK (event_sequence >= 1),
    event_id TEXT NOT NULL,
    event_type TEXT NOT NULL CHECK (
        length(CAST(event_type AS BLOB)) >= 1
        AND instr(event_type, char(0)) = 0
    ),
    event_digest TEXT NOT NULL CHECK (
        typeof(event_digest) = 'text'
        AND length(CAST(event_digest AS BLOB)) = 64
        AND event_digest = lower(event_digest)
        AND event_digest NOT GLOB '*[^0-9a-f]*'
    ),
    PRIMARY KEY (summary_id, source_ordinal),
    UNIQUE (summary_id, event_id),
    UNIQUE (summary_id, event_sequence),
    FOREIGN KEY (event_sequence, event_id, event_type, event_digest)
        REFERENCES event_stream(sequence, event_id, event_type, event_digest)
        DEFERRABLE INITIALLY DEFERRED
) STRICT;
-- migration-boundary: create_episodic_summary_sources

CREATE INDEX episodic_summary_sources_event_idx
ON episodic_summary_sources (event_id, summary_id);
-- migration-boundary: create_episodic_summary_sources_event_idx

CREATE TRIGGER episodic_summary_sources_order_guard
BEFORE INSERT ON episodic_summary_sources
WHEN NOT EXISTS (
        SELECT 1
        FROM episodic_summaries summary
        WHERE summary.summary_id = NEW.summary_id
    )
 OR NEW.source_ordinal <> (
        SELECT COUNT(*)
        FROM episodic_summary_sources existing
        WHERE existing.summary_id = NEW.summary_id
    )
 OR NEW.source_ordinal >= (
        SELECT summary.source_count
        FROM episodic_summaries summary
        WHERE summary.summary_id = NEW.summary_id
    )
 OR NEW.event_sequence >= (
        SELECT summary.creation_event_sequence
        FROM episodic_summaries summary
        WHERE summary.summary_id = NEW.summary_id
    )
 OR (
        NEW.source_ordinal > 0
        AND NEW.event_sequence <= (
            SELECT MAX(existing.event_sequence)
            FROM episodic_summary_sources existing
            WHERE existing.summary_id = NEW.summary_id
        )
    )
BEGIN
    SELECT RAISE(ABORT, 'episodic_summary_source_order_mismatch');
END;
-- migration-boundary: create_episodic_summary_sources_order_guard

CREATE TRIGGER episodic_summaries_no_update
BEFORE UPDATE ON episodic_summaries BEGIN
    SELECT RAISE(ABORT, 'episodic_summaries_immutable');
END;
-- migration-boundary: create_episodic_summaries_no_update

CREATE TRIGGER episodic_summaries_no_delete
BEFORE DELETE ON episodic_summaries BEGIN
    SELECT RAISE(ABORT, 'episodic_summaries_immutable');
END;
-- migration-boundary: create_episodic_summaries_no_delete

CREATE TRIGGER episodic_summary_sources_no_update
BEFORE UPDATE ON episodic_summary_sources BEGIN
    SELECT RAISE(ABORT, 'episodic_summary_sources_immutable');
END;
-- migration-boundary: create_episodic_summary_sources_no_update

CREATE TRIGGER episodic_summary_sources_no_delete
BEFORE DELETE ON episodic_summary_sources BEGIN
    SELECT RAISE(ABORT, 'episodic_summary_sources_immutable');
END;
-- migration-boundary: create_episodic_summary_sources_no_delete
