-- Recovery may materialize an authenticated episodic source after a prior
-- crash left an ordinal hole.  Keep the v4 migration/checksum immutable and
-- replace only its insert guard with one that validates immediate neighbours.
DROP TRIGGER episodic_summary_sources_order_guard;
-- migration-boundary: drop_episodic_summary_sources_order_guard_v4

CREATE TRIGGER episodic_summary_sources_order_guard
BEFORE INSERT ON episodic_summary_sources
WHEN NOT EXISTS (
        SELECT 1 FROM episodic_summaries summary
        WHERE summary.summary_id = NEW.summary_id
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
        AND NOT EXISTS (
            SELECT 1 FROM episodic_summary_sources previous
            WHERE previous.summary_id = NEW.summary_id
              AND previous.source_ordinal = NEW.source_ordinal - 1
        )
    )
 OR (
        NEW.source_ordinal > 0
        AND NEW.event_sequence <= (
            SELECT previous.event_sequence
            FROM episodic_summary_sources previous
            WHERE previous.summary_id = NEW.summary_id
              AND previous.source_ordinal = NEW.source_ordinal - 1
        )
    )
 OR (
        EXISTS (
            SELECT 1 FROM episodic_summary_sources following
            WHERE following.summary_id = NEW.summary_id
              AND following.source_ordinal = NEW.source_ordinal + 1
              AND NEW.event_sequence >= following.event_sequence
        )
    )
BEGIN
    SELECT RAISE(ABORT, 'episodic_summary_source_order_mismatch');
END;
-- migration-boundary: create_episodic_summary_sources_recovery_order_guard
