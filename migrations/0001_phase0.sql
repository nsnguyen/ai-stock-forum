CREATE TABLE event_stream (
    sequence INTEGER PRIMARY KEY,
    event_id TEXT NOT NULL UNIQUE,
    event_schema_version INTEGER NOT NULL CHECK (event_schema_version > 0),
    event_type TEXT NOT NULL,
    actor_kind TEXT NOT NULL,
    actor_id TEXT,
    occurred_at_ms INTEGER NOT NULL,
    correlation_id TEXT NOT NULL,
    causation_id TEXT,
    object_kind TEXT,
    object_id TEXT,
    object_version INTEGER CHECK (object_version IS NULL OR object_version > 0),
    object_digest TEXT,
    previous_event_digest TEXT,
    payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
    event_digest TEXT NOT NULL UNIQUE
) STRICT;

CREATE TRIGGER event_stream_no_update
BEFORE UPDATE ON event_stream BEGIN
    SELECT RAISE(ABORT, 'event_stream is append-only');
END;

CREATE TRIGGER event_stream_no_delete
BEFORE DELETE ON event_stream BEGIN
    SELECT RAISE(ABORT, 'event_stream is append-only');
END;

CREATE INDEX event_stream_correlation_idx ON event_stream(correlation_id, sequence);
CREATE INDEX event_stream_type_idx ON event_stream(event_type, sequence);

CREATE TABLE command_receipts (
    command_id TEXT PRIMARY KEY,
    command_fingerprint TEXT NOT NULL CHECK (
        length(command_fingerprint) = 64
        AND command_fingerprint NOT GLOB '*[^0-9a-f]*'
    ),
    request_json TEXT NOT NULL CHECK (json_valid(request_json)),
    capability TEXT NOT NULL CHECK (capability IN (
        'help_read', 'status_read', 'setup_status_read', 'audit_read', 'shutdown',
        'discussion_run', 'mcp_use', 'engineering_job_run', 'git_merge', 'git_push',
        'finance_recommendation'
    )),
    policy_decision TEXT NOT NULL CHECK (policy_decision IN (
        'granted', 'denied', 'denied_by_default', 'approval_required'
    )),
    outcome_json TEXT NOT NULL CHECK (json_valid(outcome_json))
) STRICT;

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

CREATE UNIQUE INDEX command_event_refs_event_idx ON command_event_refs(event_id);

CREATE TRIGGER command_event_refs_no_update
BEFORE UPDATE ON command_event_refs BEGIN
    SELECT RAISE(ABORT, 'command event refs are immutable');
END;

CREATE TRIGGER command_event_refs_no_delete
BEFORE DELETE ON command_event_refs BEGIN
    SELECT RAISE(ABORT, 'command event refs are immutable');
END;

CREATE TABLE installation_projection (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    installation_id TEXT NOT NULL UNIQUE,
    created_event_id TEXT NOT NULL REFERENCES event_stream(event_id),
    created_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE process_session_projection (
    session_id TEXT PRIMARY KEY,
    started_event_id TEXT NOT NULL REFERENCES event_stream(event_id),
    started_at_ms INTEGER NOT NULL,
    ended_event_id TEXT REFERENCES event_stream(event_id),
    ended_at_ms INTEGER,
    end_reason TEXT,
    CHECK ((ended_event_id IS NULL AND ended_at_ms IS NULL AND end_reason IS NULL)
        OR (ended_event_id IS NOT NULL AND ended_at_ms IS NOT NULL AND end_reason IS NOT NULL))
) STRICT;

CREATE TABLE projection_metadata (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    last_event_sequence INTEGER NOT NULL CHECK (last_event_sequence >= 0),
    last_event_digest TEXT,
    projection_digest TEXT NOT NULL
) STRICT;

CREATE TABLE setup_drafts (
    draft_id TEXT PRIMARY KEY,
    schema_version INTEGER NOT NULL CHECK (schema_version > 0),
    state TEXT NOT NULL CHECK (state IN ('drafting', 'reviewed', 'applied', 'superseded')),
    path TEXT NOT NULL CHECK (path IN ('quick_start', 'customize')),
    current_review_digest TEXT,
    payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
) STRICT;

CREATE INDEX setup_drafts_state_idx ON setup_drafts(state, updated_at_ms);

CREATE TABLE installation_configuration_versions (
    configuration_id TEXT PRIMARY KEY,
    version INTEGER NOT NULL CHECK (version > 0),
    source_draft_id TEXT NOT NULL REFERENCES setup_drafts(draft_id),
    review_digest TEXT NOT NULL,
    object_digest TEXT NOT NULL,
    payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
    created_event_id TEXT NOT NULL REFERENCES event_stream(event_id),
    created_at_ms INTEGER NOT NULL,
    UNIQUE (source_draft_id, review_digest),
    UNIQUE (version)
) STRICT;

CREATE TRIGGER installation_configuration_versions_no_update
BEFORE UPDATE ON installation_configuration_versions BEGIN
    SELECT RAISE(ABORT, 'installation configuration versions are immutable');
END;

CREATE TRIGGER installation_configuration_versions_no_delete
BEFORE DELETE ON installation_configuration_versions BEGIN
    SELECT RAISE(ABORT, 'installation configuration versions are immutable');
END;

CREATE TABLE active_installation_configuration (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    configuration_id TEXT NOT NULL REFERENCES installation_configuration_versions(configuration_id),
    activated_event_id TEXT NOT NULL REFERENCES event_stream(event_id),
    activated_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE setup_step_outcomes (
    draft_id TEXT NOT NULL REFERENCES setup_drafts(draft_id),
    step_key TEXT NOT NULL,
    attempt INTEGER NOT NULL CHECK (attempt > 0),
    status TEXT NOT NULL CHECK (status IN ('passed', 'failed', 'skipped')),
    safe_code TEXT,
    occurred_at_ms INTEGER NOT NULL,
    PRIMARY KEY (draft_id, step_key, attempt)
) STRICT;

CREATE TABLE capability_readiness (
    configuration_id TEXT NOT NULL REFERENCES installation_configuration_versions(configuration_id),
    capability TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('ready', 'unavailable')),
    reason_code TEXT,
    checked_at_ms INTEGER NOT NULL,
    projection_digest TEXT NOT NULL,
    PRIMARY KEY (configuration_id, capability)
) STRICT;

CREATE TABLE approval_records (
    approval_id TEXT PRIMARY KEY,
    action_kind TEXT NOT NULL,
    object_kind TEXT NOT NULL,
    object_id TEXT NOT NULL,
    object_version INTEGER NOT NULL CHECK (object_version > 0),
    object_digest TEXT NOT NULL,
    actor_kind TEXT NOT NULL,
    actor_id TEXT,
    status TEXT NOT NULL CHECK (status IN ('pending', 'accepted', 'rejected', 'expired', 'cancelled')),
    created_at_ms INTEGER NOT NULL,
    expires_at_ms INTEGER,
    resolved_at_ms INTEGER,
    resolution_kind TEXT,
    resolution_event_id TEXT REFERENCES event_stream(event_id),
    CHECK (expires_at_ms IS NULL OR expires_at_ms > created_at_ms),
    CHECK ((status = 'pending' AND resolved_at_ms IS NULL AND resolution_kind IS NULL)
        OR (status <> 'pending' AND resolved_at_ms IS NOT NULL AND resolution_kind IS NOT NULL))
) STRICT;

CREATE INDEX approval_records_status_idx ON approval_records(status, created_at_ms);
