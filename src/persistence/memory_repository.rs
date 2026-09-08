//! Immutable SQLite codecs for the hybrid-memory domain records.
//!
//! This layer authenticates repository rows and their relational context. It
//! also derives recovery evidence only from the verified event stream before
//! reconciling immutable memory rows and rebuildable projections.

use std::{
    collections::{BTreeMap, BTreeSet},
    str::FromStr,
};

use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter, types::Value};

use crate::{
    agents::{AgentProfileVersion, AgentProfileVersionRef},
    app::{ApplicationEvent, EventEnvelope},
    domain::{
        Actor, ApprovalId, Digest, EpisodicSummaryId, EventId, MemoryEntryVersionId,
        MemoryNamespaceId, MemoryProposalId, ObjectVersion, canonical_json_bytes,
    },
    memory::{
        EpisodicContextItem, EpisodicSummary, EpisodicSummaryRef, ExpectedMemoryEntryState,
        MemoryEntryRef, MemoryEntryState, MemoryEntryVersion, MemoryKvContextItem,
        MemoryProjection, MemoryProposal, MemoryProposalFilter, MemoryProposalOperation,
        MemoryProposalOperationKind, MemoryProposalResolution, MemoryProposalStatus,
        MemoryPurposeScope, MemoryRetrievalRequest, MemorySnapshot, MemorySnapshotBuilder,
        NormalizedMemoryKey,
    },
    policy::{ApprovalAction, ApprovalRecord, ApprovalStatus},
};

use super::{
    ImmediateTransaction, PersistenceError, RecoveryError,
    agent_profile_repository::load_exact_profile_versions_batch, load_exact_profile_version,
};

const PAGE_LIMIT: u16 = 100;
const MAX_PENDING_MEMORY_PROPOSALS: u64 = 256;

pub struct MemoryRepository;

#[derive(Debug, Clone, Default)]
pub(crate) struct ExpectedMemoryRecords {
    pub entries: BTreeMap<MemoryEntryVersionId, (u64, MemoryEntryVersion)>,
    pub proposals: BTreeMap<MemoryProposalId, (u64, MemoryProposal)>,
    pub resolutions: BTreeMap<MemoryProposalId, (u64, MemoryProposalResolution)>,
    pub summaries: BTreeMap<EpisodicSummaryId, (u64, EpisodicSummary)>,
    pub approvals: BTreeMap<ApprovalId, ExpectedMemoryApproval>,
}

#[derive(Debug, Clone)]
pub(crate) struct ExpectedMemoryApproval {
    pub record: ApprovalRecord,
    pub resolution_event_id: Option<EventId>,
    pub resolution_actor: Option<Actor>,
}

pub(crate) fn expected_memory_records(
    events: &[EventEnvelope],
) -> Result<ExpectedMemoryRecords, RecoveryError> {
    let mut state = crate::recovery::ProjectionState::default();
    derive_expected_memory_records(events, &mut state)
}

fn derive_expected_memory_records(
    events: &[EventEnvelope],
    state: &mut crate::recovery::ProjectionState,
) -> Result<ExpectedMemoryRecords, RecoveryError> {
    let mut expected = ExpectedMemoryRecords::default();
    let mut prior_events = BTreeMap::<u64, &EventEnvelope>::new();
    for envelope in events {
        crate::recovery::reduce(state, envelope)?;
        let mut add_entry = |entry: &MemoryEntryVersion| -> Result<(), RecoveryError> {
            if entry.creation_event_id() != envelope.event_id
                || entry.created_at_ms() != envelope.occurred_at_ms
            {
                return Err(RecoveryError::InvalidEventRecord);
            }
            match expected.entries.insert(
                entry.reference().entry_version_id(),
                (envelope.sequence, entry.clone()),
            ) {
                None => Ok(()),
                Some((sequence, prior)) if sequence == envelope.sequence && prior == *entry => {
                    Ok(())
                }
                Some(_) => Err(RecoveryError::InvalidEventRecord),
            }
        };
        let mut add_resolution =
            |resolution: &MemoryProposalResolution| -> Result<(), RecoveryError> {
                if resolution.resolution_event_id() != envelope.event_id
                    || resolution.resolved_at_ms() != envelope.occurred_at_ms
                    || envelope.actor != Actor::Human
                {
                    return Err(RecoveryError::InvalidEventRecord);
                }
                if expected
                    .resolutions
                    .insert(
                        resolution.proposal().proposal_id(),
                        (envelope.sequence, resolution.clone()),
                    )
                    .is_some()
                {
                    return Err(RecoveryError::InvalidEventRecord);
                }
                let approval = expected
                    .approvals
                    .get_mut(&resolution.approval_id())
                    .ok_or(RecoveryError::InvalidEventRecord)?;
                approval.record = approval
                    .record
                    .resolve(
                        match resolution.status() {
                            MemoryProposalStatus::Accepted => ApprovalStatus::Accepted,
                            MemoryProposalStatus::Rejected => ApprovalStatus::Rejected,
                            MemoryProposalStatus::Expired => ApprovalStatus::Expired,
                            MemoryProposalStatus::Pending => {
                                return Err(RecoveryError::InvalidEventRecord);
                            }
                        },
                        Actor::Human,
                        resolution.resolved_at_ms(),
                    )
                    .map_err(|_| RecoveryError::InvalidEventRecord)?;
                approval.resolution_event_id = Some(envelope.event_id);
                approval.resolution_actor = Some(Actor::Human);
                Ok(())
            };
        match &envelope.event {
            ApplicationEvent::MemoryEntrySet {
                entry,
                expired_proposals,
            }
            | ApplicationEvent::MemoryEntryDeleted {
                entry,
                expired_proposals,
            } => {
                add_entry(entry)?;
                for resolution in expired_proposals {
                    add_resolution(resolution)?;
                }
            }
            ApplicationEvent::MemoryProposalCreated { proposal, approval } => {
                if proposal.creation_event_id() != envelope.event_id
                    || proposal.created_at_ms() != envelope.occurred_at_ms
                    || approval.approval_id() != proposal.approval_id()
                {
                    return Err(RecoveryError::InvalidEventRecord);
                }
                if expected
                    .proposals
                    .insert(
                        proposal.reference().proposal_id(),
                        (envelope.sequence, proposal.clone()),
                    )
                    .is_some()
                    || expected
                        .approvals
                        .insert(
                            proposal.approval_id(),
                            ExpectedMemoryApproval {
                                record: approval.clone(),
                                resolution_event_id: None,
                                resolution_actor: None,
                            },
                        )
                        .is_some()
                {
                    return Err(RecoveryError::InvalidEventRecord);
                }
            }
            ApplicationEvent::MemoryProposalAccepted {
                resolution,
                entry,
                expired_proposals,
            } => {
                add_resolution(resolution)?;
                add_entry(entry)?;
                for resolution in expired_proposals {
                    add_resolution(resolution)?;
                }
            }
            ApplicationEvent::MemoryProposalRejected { resolution } => add_resolution(resolution)?,
            ApplicationEvent::EpisodicSummaryRecorded { summary } => {
                if summary.creation_event_sequence() != envelope.sequence
                    || summary.creation_event_id() != envelope.event_id
                    || summary.created_at_ms() != envelope.occurred_at_ms
                    || envelope.actor != Actor::System
                    || expected
                        .summaries
                        .insert(
                            summary.reference().summary_id(),
                            (envelope.sequence, summary.clone()),
                        )
                        .is_some()
                {
                    return Err(RecoveryError::InvalidEventRecord);
                }
                for source in summary.sources() {
                    let prior = prior_events
                        .get(&source.sequence())
                        .ok_or(RecoveryError::InvalidEventRecord)?;
                    if prior.event_id != source.event_id()
                        || prior.event.kind() != source.event_type()
                        || prior.event_digest.as_str() != source.event_digest().as_str()
                    {
                        return Err(RecoveryError::InvalidEventRecord);
                    }
                }
            }
            _ => {}
        }
        if prior_events.insert(envelope.sequence, envelope).is_some() {
            return Err(RecoveryError::InvalidEventRecord);
        }
    }
    Ok(expected)
}

/// Materialize only wholly missing immutable entry rows after deriving their
/// exact canonical representation from the verified stream. Any collision,
/// altered row, or extra immutable entry fails closed before mutable pointers
/// are rebuilt.
pub(crate) fn reconcile_verified_memory(
    tx: &rusqlite::Transaction<'_>,
    events: &[EventEnvelope],
    expected_projection: &MemoryProjection,
) -> Result<(), RecoveryError> {
    let expected = expected_memory_records(events)?;
    let mut derived_state = crate::recovery::ProjectionState::default();
    for event in events {
        crate::recovery::reduce(&mut derived_state, event)?;
    }
    if &derived_state.memory != expected_projection {
        return Err(RecoveryError::InvalidEventRecord);
    }
    reject_unexpected_ids(
        tx,
        "SELECT entry_version_id FROM memory_entry_versions",
        expected.entries.keys().map(ToString::to_string).collect(),
    )?;
    reject_unexpected_ids(
        tx,
        "SELECT proposal_id FROM memory_proposals",
        expected.proposals.keys().map(ToString::to_string).collect(),
    )?;
    reject_unexpected_ids(
        tx,
        "SELECT proposal_id FROM memory_proposal_resolutions",
        expected
            .resolutions
            .keys()
            .map(ToString::to_string)
            .collect(),
    )?;
    reject_unexpected_ids(
        tx,
        "SELECT summary_id FROM episodic_summaries",
        expected.summaries.keys().map(ToString::to_string).collect(),
    )?;
    reject_unexpected_source_keys(tx, &expected)?;
    for (approval_id, approval) in &expected.approvals {
        if approval
            .record
            .resolution()
            .map(|resolution| resolution.actor())
            != approval.resolution_actor.as_ref()
        {
            return Err(RecoveryError::InvalidEventRecord);
        }
        let columns = approval_columns(&approval.record, approval.resolution_event_id)
            .map_err(|_| RecoveryError::InvalidEventRecord)?;
        let actual = tx.query_row(
            "SELECT action_kind,object_kind,object_id,object_version,object_digest,actor_id,actor_kind,resolution_event_id,status,created_at_ms,expires_at_ms,resolved_at_ms,resolution_kind,resolution_actor_kind,resolution_actor_id,approval_id FROM approval_records WHERE approval_id=?1",
            [approval_id.to_string()],
            |row| -> rusqlite::Result<ApprovalColumns> { Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?,row.get(8)?,row.get(9)?,row.get(10)?,row.get(11)?,row.get(12)?,row.get(13)?,row.get(14)?,row.get(15)?)) },
        ).optional().map_err(|_| RecoveryError::QueryFailed)?;
        match actual {
            None => {
                tx.execute("INSERT INTO approval_records (approval_id,action_kind,object_kind,object_id,object_version,object_digest,actor_kind,actor_id,status,created_at_ms,expires_at_ms,resolved_at_ms,resolution_kind,resolution_event_id,resolution_actor_kind,resolution_actor_id) VALUES (?16,?1,?2,?3,?4,?5,?7,?6,?9,?10,?11,?12,?13,?8,?14,?15)", params![columns.0,columns.1,columns.2,columns.3,columns.4,columns.5,columns.6,columns.7,columns.8,columns.9,columns.10,columns.11,columns.12,columns.13,columns.14,columns.15]).map_err(|_| RecoveryError::QueryFailed)?;
            }
            Some(stored) if approval_columns_equal(&stored, &columns) => {}
            Some(_) => return Err(RecoveryError::InvalidEventRecord),
        }
    }
    for (sequence, proposal) in expected.proposals.values() {
        let row =
            proposal_row(proposal, *sequence).map_err(|_| RecoveryError::InvalidEventRecord)?;
        let found: Option<ProposalRow> = tx
            .query_row(
                "SELECT proposal_id,version,proposer_profile_id,proposer_profile_version_id,proposer_profile_version,proposer_profile_digest,memory_namespace_id,operation,display_key,normalized_key,expected_kind,expected_entry_id,expected_entry_version_id,expected_entry_version,expected_entry_digest,candidate_value,candidate_value_bytes,candidate_purpose_tags_json,rationale,plaintext_validation_version,created_at_ms,creation_event_sequence,creation_event_id,approval_id,content_digest,record_digest,record_json FROM memory_proposals WHERE proposal_id=?1",
                [row.proposal_id.clone()],
                decode_proposal_stored_row,
            )
            .optional()
            .map_err(|_| RecoveryError::QueryFailed)?;
        match found {
            None => {
                tx.execute("INSERT INTO memory_proposals (proposal_id,version,proposer_profile_id,proposer_profile_version_id,proposer_profile_version,proposer_profile_digest,memory_namespace_id,operation,display_key,normalized_key,expected_kind,expected_entry_id,expected_entry_version_id,expected_entry_version,expected_entry_digest,candidate_value,candidate_value_bytes,candidate_purpose_tags_json,rationale,plaintext_validation_version,created_at_ms,creation_event_sequence,creation_event_id,approval_id,content_digest,record_digest,record_json) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27)",params![row.proposal_id,row.version,row.profile_id,row.profile_version_id,row.profile_version,row.profile_digest,row.namespace,row.operation,row.display_key,row.normalized_key,row.expected_kind,row.expected_id,row.expected_version_id,row.expected_version,row.expected_digest,row.candidate_value,row.candidate_value_bytes,row.candidate_tags,row.rationale,row.plaintext,row.created_at,row.creation_sequence,row.creation_event_id,row.approval_id,row.content_digest,row.record_digest,row.record_json]).map_err(|_| RecoveryError::QueryFailed)?;
            }
            Some(stored) if stored == row => {}
            Some(_) => return Err(RecoveryError::InvalidEventRecord),
        }
    }
    for (sequence, resolution) in expected.resolutions.values() {
        let row =
            resolution_row(resolution, *sequence).map_err(|_| RecoveryError::InvalidEventRecord)?;
        let stored = tx
            .query_row(
                "SELECT proposal_id,proposal_version,proposal_content_digest,status,approval_id,resolved_by_kind,resolved_by_id,resolved_at_ms,resolution_event_sequence,resolution_event_id,resolution_json FROM memory_proposal_resolutions WHERE proposal_id=?1",
                [row.proposal_id.clone()],
                |record| Ok(ResolutionRow { proposal_id: record.get(0)?, proposal_version: record.get(1)?, proposal_digest: record.get(2)?, status: record.get(3)?, approval_id: record.get(4)?, resolved_by_kind: record.get(5)?, resolved_by_id: record.get(6)?, resolved_at: record.get(7)?, resolution_sequence: record.get(8)?, resolution_event_id: record.get(9)?, resolution_json: record.get(10)? }),
            )
            .optional()
            .map_err(|_| RecoveryError::QueryFailed)?;
        match stored {
            None => {
                tx.execute("INSERT INTO memory_proposal_resolutions (proposal_id,proposal_version,proposal_content_digest,status,approval_id,resolved_by_kind,resolved_by_id,resolved_at_ms,resolution_event_sequence,resolution_event_id,resolution_json) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)", params![row.proposal_id,row.proposal_version,row.proposal_digest,row.status,row.approval_id,row.resolved_by_kind,row.resolved_by_id,row.resolved_at,row.resolution_sequence,row.resolution_event_id,row.resolution_json]).map_err(|_| RecoveryError::QueryFailed)?;
            }
            Some(stored) if stored == row => {}
            Some(_) => return Err(RecoveryError::InvalidEventRecord),
        }
    }
    for (_sequence, summary) in expected.summaries.values() {
        let row = summary_row(summary).map_err(|_| RecoveryError::InvalidEventRecord)?;
        let found: Option<SummaryRow> = tx
            .query_row(
                "SELECT summary_id,version,memory_namespace_id,profile_id,profile_version_id,profile_version,profile_content_digest,label,body,purpose_tags_json,source_count,plaintext_validation_version,created_at_ms,creation_event_sequence,creation_event_id,source_set_digest,content_digest,record_digest,record_json FROM episodic_summaries WHERE summary_id=?1",
                [row.summary_id.clone()],
                decode_summary_stored_row,
            )
            .optional()
            .map_err(|_| RecoveryError::QueryFailed)?;
        match found {
            None => {
                tx.execute("INSERT INTO episodic_summaries (summary_id,version,memory_namespace_id,profile_id,profile_version_id,profile_version,profile_content_digest,label,body,purpose_tags_json,source_count,plaintext_validation_version,created_at_ms,creation_event_sequence,creation_event_id,source_set_digest,content_digest,record_digest,record_json) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)", params![row.summary_id,row.version,row.namespace,row.profile_id,row.profile_version_id,row.profile_version,row.profile_digest,row.label,row.body,row.tags,row.source_count,row.plaintext,row.created_at,row.creation_sequence,row.creation_event_id,row.source_digest,row.content_digest,row.record_digest,row.record_json]).map_err(|_| RecoveryError::QueryFailed)?;
            }
            Some(stored) if stored == row => {}
            Some(_) => return Err(RecoveryError::InvalidEventRecord),
        }
        for (ordinal, source) in summary.sources().iter().enumerate() {
            let ordinal = to_i64(ordinal as u64).map_err(|_| RecoveryError::InvalidEventRecord)?;
            let source_id = summary.reference().summary_id().to_string();
            let actual = tx.query_row("SELECT event_sequence,event_id,event_type,event_digest FROM episodic_summary_sources WHERE summary_id=?1 AND source_ordinal=?2", params![source_id, ordinal], |record| -> rusqlite::Result<(i64,String,String,String)> { Ok((record.get(0)?,record.get(1)?,record.get(2)?,record.get(3)?)) }).optional().map_err(|_| RecoveryError::QueryFailed)?;
            let wanted = (
                to_i64(source.sequence()).map_err(|_| RecoveryError::InvalidEventRecord)?,
                source.event_id().to_string(),
                source.event_type().to_owned(),
                source.event_digest().as_str().to_owned(),
            );
            match actual {
                None => {
                    tx.execute("INSERT INTO episodic_summary_sources (summary_id,source_ordinal,event_sequence,event_id,event_type,event_digest) VALUES (?1,?2,?3,?4,?5,?6)", params![summary.reference().summary_id().to_string(),ordinal,wanted.0,wanted.1,wanted.2,wanted.3]).map_err(|_| RecoveryError::QueryFailed)?;
                }
                Some(stored) if stored == wanted => {}
                Some(_) => return Err(RecoveryError::InvalidEventRecord),
            }
            let event_match: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM event_stream WHERE sequence=?1 AND event_id=?2 AND event_type=?3 AND event_digest=?4)",
                    params![wanted.0, wanted.1, wanted.2, wanted.3],
                    |row| row.get(0),
                )
                .map_err(|_| RecoveryError::QueryFailed)?;
            if !event_match {
                return Err(RecoveryError::InvalidEventRecord);
            }
        }
        let source_count: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM episodic_summary_sources WHERE summary_id=?1",
                [summary.reference().summary_id().to_string()],
                |row| row.get(0),
            )
            .map_err(|_| RecoveryError::QueryFailed)?;
        if source_count
            != i64::try_from(summary.sources().len())
                .map_err(|_| RecoveryError::InvalidEventRecord)?
        {
            return Err(RecoveryError::InvalidEventRecord);
        }
    }
    let mut entries = expected.entries.values().collect::<Vec<_>>();
    entries.sort_by_key(|(sequence, entry)| (*sequence, entry.reference().version().get()));
    for (sequence, entry) in entries {
        let row = entry_row(entry, *sequence).map_err(|_| RecoveryError::InvalidEventRecord)?;
        match load_entry_rows_matching(tx, &row)
            .map_err(|_| RecoveryError::InvalidEventRecord)?
            .as_slice()
        {
            [] => insert_entry_row(tx, &row).map_err(|_| RecoveryError::InvalidEventRecord)?,
            [stored] if stored == &row => {}
            _ => return Err(RecoveryError::InvalidEventRecord),
        }
    }
    let actual: i64 = tx
        .query_row("SELECT COUNT(*) FROM memory_entry_versions", [], |row| {
            row.get(0)
        })
        .map_err(|_| RecoveryError::QueryFailed)?;
    if u64::try_from(actual).map_err(|_| RecoveryError::InvalidEventRecord)?
        != u64::try_from(expected.entries.len()).map_err(|_| RecoveryError::InvalidEventRecord)?
    {
        return Err(RecoveryError::InvalidEventRecord);
    }
    let proposal_count: i64 = tx
        .query_row("SELECT COUNT(*) FROM memory_proposals", [], |row| {
            row.get(0)
        })
        .map_err(|_| RecoveryError::QueryFailed)?;
    if proposal_count
        != i64::try_from(expected.proposals.len()).map_err(|_| RecoveryError::InvalidEventRecord)?
    {
        return Err(RecoveryError::InvalidEventRecord);
    }
    let resolution_count: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM memory_proposal_resolutions",
            [],
            |row| row.get(0),
        )
        .map_err(|_| RecoveryError::QueryFailed)?;
    if resolution_count
        != i64::try_from(expected.resolutions.len())
            .map_err(|_| RecoveryError::InvalidEventRecord)?
    {
        return Err(RecoveryError::InvalidEventRecord);
    }
    let summary_count: i64 = tx
        .query_row("SELECT COUNT(*) FROM episodic_summaries", [], |row| {
            row.get(0)
        })
        .map_err(|_| RecoveryError::QueryFailed)?;
    if summary_count
        != i64::try_from(expected.summaries.len()).map_err(|_| RecoveryError::InvalidEventRecord)?
    {
        return Err(RecoveryError::InvalidEventRecord);
    }
    let memory_approval_count: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM approval_records WHERE action_kind='memory_mutation'",
            [],
            |row| row.get(0),
        )
        .map_err(|_| RecoveryError::QueryFailed)?;
    if memory_approval_count
        != i64::try_from(expected.approvals.len()).map_err(|_| RecoveryError::InvalidEventRecord)?
    {
        return Err(RecoveryError::InvalidEventRecord);
    }
    for ((_namespace, _key), current) in expected_projection.current_entries() {
        if !expected
            .entries
            .values()
            .any(|(_, entry)| entry.reference() == *current)
        {
            return Err(RecoveryError::InvalidEventRecord);
        }
    }
    for (proposal_id, projected) in expected_projection.proposals() {
        let (_, proposal) = expected
            .proposals
            .get(proposal_id)
            .ok_or(RecoveryError::InvalidEventRecord)?;
        if proposal.reference() != *projected.proposal()
            || proposal.approval_id() != projected.approval_id()
            || expected
                .resolutions
                .get(proposal_id)
                .map(|(_, resolution)| resolution.resolution_event_id())
                != projected.resolution_event_id()
        {
            return Err(RecoveryError::InvalidEventRecord);
        }
    }
    Ok(())
}

fn reject_unexpected_ids(
    tx: &Connection,
    query: &str,
    expected: BTreeSet<String>,
) -> Result<(), RecoveryError> {
    let mut statement = tx.prepare(query).map_err(|_| RecoveryError::QueryFailed)?;
    let actual = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|_| RecoveryError::QueryFailed)?
        .collect::<Result<BTreeSet<_>, _>>()
        .map_err(|_| RecoveryError::QueryFailed)?;
    if actual.is_subset(&expected) {
        Ok(())
    } else {
        Err(RecoveryError::InvalidEventRecord)
    }
}

fn reject_unexpected_source_keys(
    tx: &Connection,
    expected: &ExpectedMemoryRecords,
) -> Result<(), RecoveryError> {
    let mut expected_keys = BTreeSet::new();
    for (_, summary) in expected.summaries.values() {
        for (ordinal, _) in summary.sources().iter().enumerate() {
            expected_keys.insert((
                summary.reference().summary_id().to_string(),
                i64::try_from(ordinal).map_err(|_| RecoveryError::InvalidEventRecord)?,
            ));
        }
    }
    let mut statement = tx
        .prepare("SELECT summary_id,source_ordinal FROM episodic_summary_sources")
        .map_err(|_| RecoveryError::QueryFailed)?;
    let actual = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|_| RecoveryError::QueryFailed)?
        .collect::<Result<BTreeSet<_>, _>>()
        .map_err(|_| RecoveryError::QueryFailed)?;
    if actual.is_subset(&expected_keys) {
        Ok(())
    } else {
        Err(RecoveryError::InvalidEventRecord)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryEntryHistoryPage {
    pub versions: Vec<MemoryEntryVersion>,
    pub total_count: u64,
    pub returned_count: u64,
    pub omitted_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryEntryListRecord {
    pub entry: MemoryEntryRef,
    pub display_key: String,
    pub purpose_tags: Vec<String>,
    pub value_bytes: u64,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryEntriesPage {
    pub records: Vec<MemoryEntryListRecord>,
    pub total_count: u64,
    pub returned_count: u64,
    pub omitted_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryProposalListRecord {
    pub proposal: crate::memory::MemoryProposalRef,
    pub proposer: AgentProfileVersionRef,
    pub operation: MemoryProposalOperationKind,
    pub display_key: String,
    pub status: MemoryProposalStatus,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryProposalsPage {
    pub records: Vec<MemoryProposalListRecord>,
    pub total_count: u64,
    pub returned_count: u64,
    pub omitted_count: u64,
}

// The task brief repeats `source_count` and `created_at_ms` in its snippet.
// They are each one field here: the repeated line is a prose duplication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpisodicSummaryListRecord {
    pub summary: EpisodicSummaryRef,
    pub label: String,
    pub purpose_tags: Vec<String>,
    pub source_count: u64,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpisodicSummariesPage {
    pub records: Vec<EpisodicSummaryListRecord>,
    pub total_count: u64,
    pub returned_count: u64,
    pub omitted_count: u64,
}

impl MemoryRepository {
    /// Rebuilds only the mutable memory projection rows after the authoritative
    /// event stream has been verified and reduced.  Immutable records are never
    /// synthesized here: every pointer/status must resolve to its exact stored
    /// immutable record before replacement begins.
    pub(crate) fn replace_current_projection(
        tx: &rusqlite::Transaction<'_>,
        projection: &MemoryProjection,
    ) -> Result<(), PersistenceError> {
        for ((namespace, key), reference) in projection.current_entries() {
            let matches: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM memory_entry_versions WHERE memory_namespace_id=?1 AND normalized_key=?2 AND entry_id=?3 AND entry_version_id=?4 AND version=?5 AND state=?6 AND content_digest=?7",
                    params![
                        namespace.to_string(), key.as_str(), reference.entry_id().to_string(),
                        reference.entry_version_id().to_string(), to_i64(reference.version().get())?,
                        entry_state(reference.state()), reference.content_digest().as_str(),
                    ],
                    |row| row.get(0),
                )
                .map_err(query)?;
            if matches != 1 {
                return Err(PersistenceError::MemoryRowMismatch);
            }
        }
        for (_id, projected) in projection.proposals() {
            let reference = projected.proposal();
            let matches: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM memory_proposals WHERE proposal_id=?1 AND version=?2 AND content_digest=?3 AND memory_namespace_id=?4 AND normalized_key=?5 AND approval_id=?6",
                    params![
                        reference.proposal_id().to_string(), to_i64(reference.version().get())?,
                        reference.content_digest().as_str(), projected.namespace_id().to_string(),
                        projected.normalized_key().as_str(), projected.approval_id().to_string(),
                    ],
                    |row| row.get(0),
                )
                .map_err(query)?;
            if matches != 1 {
                return Err(PersistenceError::MemoryRowMismatch);
            }
        }

        tx.execute("DELETE FROM current_memory_entries", [])
            .map_err(query)?;
        tx.execute("DELETE FROM current_memory_proposal_status", [])
            .map_err(query)?;
        for ((namespace, key), reference) in projection.current_entries() {
            tx.execute(
                "INSERT INTO current_memory_entries (memory_namespace_id,normalized_key,entry_id,entry_version_id,version,state,content_digest) VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![
                    namespace.to_string(), key.as_str(), reference.entry_id().to_string(),
                    reference.entry_version_id().to_string(), to_i64(reference.version().get())?,
                    entry_state(reference.state()), reference.content_digest().as_str(),
                ],
            ).map_err(query)?;
        }
        for (_id, projected) in projection.proposals() {
            let reference = projected.proposal();
            tx.execute(
                "INSERT INTO current_memory_proposal_status (proposal_id,proposal_version,proposal_content_digest,memory_namespace_id,normalized_key,status,resolution_event_id,created_at_ms) SELECT proposal_id,version,content_digest,memory_namespace_id,normalized_key,?2,?3,created_at_ms FROM memory_proposals WHERE proposal_id=?1",
                params![
                    reference.proposal_id().to_string(), proposal_status(projected.status()),
                    projected.resolution_event_id().map(|id| id.to_string()),
                ],
            ).map_err(query)?;
        }
        Ok(())
    }

    pub fn insert_entry_version(
        tx: &ImmediateTransaction<'_>,
        creation_sequence: u64,
        entry: &MemoryEntryVersion,
    ) -> Result<(), PersistenceError> {
        validate_event(tx, creation_sequence, entry.creation_event_id())?;
        Self::validate_entry_context(tx, entry)?;
        let expected = entry_row(entry, creation_sequence)?;
        let existing = load_entry_rows_matching(tx.transaction(), &expected)?;
        match existing.as_slice() {
            [] => insert_entry_row(tx.transaction(), &expected),
            [row] if row == &expected => Ok(()),
            _ => Err(PersistenceError::MemoryRowMismatch),
        }
    }

    pub fn replace_current_entry(
        tx: &ImmediateTransaction<'_>,
        entry: &MemoryEntryVersion,
    ) -> Result<(), PersistenceError> {
        Self::validate_entry_context(tx, entry)?;
        let stored = load_entry_by_version_id(tx, entry.reference().entry_version_id())?
            .ok_or(PersistenceError::MemoryRowMismatch)?;
        if stored != *entry {
            return Err(PersistenceError::MemoryRowMismatch);
        }
        let prior = load_current_pointer(
            tx,
            entry.reference().namespace_id(),
            entry.reference().normalized_key(),
        )?;
        if let Some(prior) = prior {
            if prior == entry.reference() {
                return Ok(());
            }
            let prior_entry = load_entry_by_version_id(tx, prior.entry_version_id())?
                .ok_or(PersistenceError::MemoryRowMismatch)?;
            if prior_entry.reference().entry_id() != entry.reference().entry_id()
                || entry.predecessor_version_id() != Some(prior.entry_version_id())
                || entry.reference().version().get()
                    != prior_entry
                        .reference()
                        .version()
                        .get()
                        .checked_add(1)
                        .ok_or(PersistenceError::MemoryRowMismatch)?
            {
                return Err(PersistenceError::MemoryRowMismatch);
            }
        } else if entry.reference().version().get() != 1 {
            return Err(PersistenceError::MemoryRowMismatch);
        }
        let entry_ref = entry.reference();
        tx.transaction().execute(
            "INSERT INTO current_memory_entries (
                memory_namespace_id, normalized_key, entry_id, entry_version_id, version, state, content_digest
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(memory_namespace_id, normalized_key) DO UPDATE SET
                entry_id=excluded.entry_id, entry_version_id=excluded.entry_version_id,
                version=excluded.version, state=excluded.state, content_digest=excluded.content_digest",
            params![
                entry_ref.namespace_id().to_string(), entry_ref.normalized_key().as_str(),
                entry_ref.entry_id().to_string(), entry_ref.entry_version_id().to_string(),
                to_i64(entry_ref.version().get())?, entry_state(entry_ref.state()),
                entry_ref.content_digest().as_str(),
            ],
        ).map_err(query)?;
        Ok(())
    }

    pub fn load_current_entry(
        tx: &ImmediateTransaction<'_>,
        namespace: MemoryNamespaceId,
        key: &NormalizedMemoryKey,
    ) -> Result<Option<MemoryEntryVersion>, PersistenceError> {
        let Some(pointer) = load_current_pointer(tx, namespace, key)? else {
            return Ok(None);
        };
        let entry = load_entry_by_version_id(tx, pointer.entry_version_id())?
            .ok_or(PersistenceError::MemoryRowMismatch)?;
        if entry.reference() != pointer {
            return Err(PersistenceError::MemoryRowMismatch);
        }
        Ok(Some(entry))
    }

    pub fn load_entry_history(
        tx: &ImmediateTransaction<'_>,
        namespace: MemoryNamespaceId,
        key: &NormalizedMemoryKey,
        limit: u16,
    ) -> Result<MemoryEntryHistoryPage, PersistenceError> {
        let total_count = checked_count(
            tx,
            "SELECT COUNT(*) FROM memory_entry_versions WHERE memory_namespace_id=?1 AND normalized_key=?2",
            params![namespace.to_string(), key.as_str()],
        )?;
        let mut statement = tx.transaction().prepare(
                "SELECT memory_namespace_id,entry_id,entry_version_id,version,predecessor_version_id,
                        display_key,normalized_key,state,value_text,value_bytes,purpose_tags_json,
                        created_by_kind,created_by_id,created_at_ms,accepted_proposal_id,
                        accepted_proposal_version,accepted_proposal_digest,plaintext_validation_version,
                        creation_event_sequence,creation_event_id,content_digest,record_digest,record_json
                   FROM memory_entry_versions
             WHERE memory_namespace_id=?1 AND normalized_key=?2
             ORDER BY version DESC, entry_version_id ASC LIMIT ?3",
            )
            .map_err(query)?;
        let rows = statement
            .query_map(
                params![
                    namespace.to_string(),
                    key.as_str(),
                    i64::from(page_limit(limit))
                ],
                decode_entry_row,
            )
            .map_err(query)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(query)?;
        let versions = rows
            .into_iter()
            .map(decode_entry_row_value)
            .collect::<Result<Vec<_>, _>>()?;
        validate_entry_batch_context(tx, &versions)?;
        page_history(versions, total_count)
    }

    pub fn list_current_entries(
        tx: &ImmediateTransaction<'_>,
        namespace: MemoryNamespaceId,
        limit: u16,
    ) -> Result<MemoryEntriesPage, PersistenceError> {
        validate_current_entry_rows(tx, namespace)?;
        let total_count = checked_count(
            tx,
            "SELECT COUNT(*) FROM current_memory_entries c JOIN memory_entry_versions v
             ON v.memory_namespace_id=c.memory_namespace_id AND v.normalized_key=c.normalized_key
             AND v.entry_id=c.entry_id AND v.entry_version_id=c.entry_version_id
             WHERE c.memory_namespace_id=?1 AND v.state='present'",
            [namespace.to_string()],
        )?;
        let mut statement = tx.transaction().prepare(
            "SELECT v.memory_namespace_id,v.entry_id,v.entry_version_id,v.version,
                    v.predecessor_version_id,v.display_key,v.normalized_key,v.state,v.value_text,
                    v.value_bytes,v.purpose_tags_json,v.created_by_kind,v.created_by_id,v.created_at_ms,
                    v.accepted_proposal_id,v.accepted_proposal_version,v.accepted_proposal_digest,
                    v.plaintext_validation_version,v.creation_event_sequence,v.creation_event_id,
                    v.content_digest,v.record_digest,v.record_json,
                    c.version,c.state,c.content_digest
             FROM current_memory_entries AS c JOIN memory_entry_versions AS v
               ON v.memory_namespace_id=c.memory_namespace_id AND v.normalized_key=c.normalized_key
              AND v.entry_id=c.entry_id AND v.entry_version_id=c.entry_version_id
             WHERE c.memory_namespace_id=?1 AND v.state='present'
             ORDER BY c.normalized_key ASC, c.entry_id ASC LIMIT ?2"
        ).map_err(query)?;
        let rows = statement
            .query_map(
                params![namespace.to_string(), i64::from(page_limit(limit))],
                |r| {
                    Ok((
                        decode_entry_row(r)?,
                        r.get::<_, i64>(23)?,
                        r.get::<_, String>(24)?,
                        r.get::<_, String>(25)?,
                    ))
                },
            )
            .map_err(query)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(query)?;
        let mut records = Vec::with_capacity(rows.len());
        for (stored, pointer_version, pointer_state, pointer_digest) in rows {
            let entry = decode_entry_row_value(stored)?;
            validate_current_pointer_columns(
                &entry,
                pointer_version,
                &pointer_state,
                &pointer_digest,
            )?;
            if entry.reference().namespace_id() != namespace
                || entry.reference().state() != MemoryEntryState::Present
            {
                return Err(PersistenceError::MemoryRowMismatch);
            }
            records.push(MemoryEntryListRecord {
                entry: entry.reference(),
                display_key: entry.display_key().to_owned(),
                purpose_tags: entry.purpose_tags().to_vec(),
                value_bytes: u64::try_from(
                    entry
                        .value()
                        .ok_or(PersistenceError::MemoryRowMismatch)?
                        .len(),
                )
                .map_err(|_| PersistenceError::MemoryRowMismatch)?,
                created_at_ms: entry.created_at_ms(),
            });
        }
        page_entries(records, total_count)
    }

    pub fn load_entry_version(
        tx: &ImmediateTransaction<'_>,
        namespace: MemoryNamespaceId,
        key: &NormalizedMemoryKey,
        version: ObjectVersion,
    ) -> Result<Option<MemoryEntryVersion>, PersistenceError> {
        let id = tx.transaction().query_row(
            "SELECT entry_version_id FROM memory_entry_versions WHERE memory_namespace_id=?1 AND normalized_key=?2 AND version=?3",
            params![namespace.to_string(), key.as_str(), to_i64(version.get())?], |r| r.get::<_, String>(0)
        ).optional().map_err(query)?;
        id.map(|id| load_entry_by_version_id(tx, parse_id(&id)?))
            .transpose()
            .map(|entry| entry.flatten())
    }

    pub fn insert_proposal_with_approval(
        tx: &ImmediateTransaction<'_>,
        creation_sequence: u64,
        proposal: &MemoryProposal,
        approval: &ApprovalRecord,
    ) -> Result<(), PersistenceError> {
        Self::insert_proposal_with_approval_observed(
            tx,
            creation_sequence,
            proposal,
            approval,
            |_| Ok(()),
            |_| Ok(()),
            |_| Ok(()),
        )
    }

    pub(crate) fn insert_proposal_with_approval_observed<A, P, C>(
        tx: &ImmediateTransaction<'_>,
        creation_sequence: u64,
        proposal: &MemoryProposal,
        approval: &ApprovalRecord,
        mut after_approval: A,
        mut after_proposal: P,
        mut after_current: C,
    ) -> Result<(), PersistenceError>
    where
        A: FnMut(&rusqlite::Transaction<'_>) -> Result<(), PersistenceError>,
        P: FnMut(&rusqlite::Transaction<'_>) -> Result<(), PersistenceError>,
        C: FnMut(&rusqlite::Transaction<'_>) -> Result<(), PersistenceError>,
    {
        with_operation_savepoint(tx, || {
            Self::insert_proposal_with_approval_inner(
                tx,
                creation_sequence,
                proposal,
                approval,
                &mut after_approval,
                &mut after_proposal,
                &mut after_current,
            )
        })
    }

    fn insert_proposal_with_approval_inner<A, P, C>(
        tx: &ImmediateTransaction<'_>,
        creation_sequence: u64,
        proposal: &MemoryProposal,
        approval: &ApprovalRecord,
        after_approval: &mut A,
        after_proposal: &mut P,
        after_current: &mut C,
    ) -> Result<(), PersistenceError>
    where
        A: FnMut(&rusqlite::Transaction<'_>) -> Result<(), PersistenceError>,
        P: FnMut(&rusqlite::Transaction<'_>) -> Result<(), PersistenceError>,
        C: FnMut(&rusqlite::Transaction<'_>) -> Result<(), PersistenceError>,
    {
        validate_event(tx, creation_sequence, proposal.creation_event_id())?;
        let expected = proposal_row(proposal, creation_sequence)?;
        let existing = load_proposal_rows_matching(tx, &expected)?;
        match existing.as_slice() {
            [] | [_] if existing.first().is_none_or(|row| row == &expected) => {}
            _ => return Err(PersistenceError::MemoryRowMismatch),
        }
        if existing.is_empty()
            && Self::count_pending_proposals(tx, proposal.namespace_id())?
                >= MAX_PENDING_MEMORY_PROPOSALS
        {
            return Err(PersistenceError::Capacity);
        }
        validate_memory_approval(proposal, approval, None)?;
        Self::validate_proposal_base_context(tx, proposal)?;
        if insert_approval(tx, approval, None)? {
            after_approval(tx.transaction())?;
        }
        match existing.as_slice() {
            [] => {
                insert_proposal_row(tx, &expected)?;
                after_proposal(tx.transaction())?;
                insert_proposal_status(tx, proposal, MemoryProposalStatus::Pending, None)?;
                after_current(tx.transaction())
            }
            [row] if row == &expected => {
                validate_proposal_status(tx, proposal, MemoryProposalStatus::Pending, None)
            }
            _ => Err(PersistenceError::MemoryRowMismatch),
        }
    }

    pub fn resolve_proposal(
        tx: &ImmediateTransaction<'_>,
        resolution_sequence: u64,
        resolution: &MemoryProposalResolution,
        approval: &ApprovalRecord,
    ) -> Result<(), PersistenceError> {
        Self::resolve_proposal_observed(
            tx,
            resolution_sequence,
            resolution,
            approval,
            |_| Ok(()),
            |_| Ok(()),
            |_| Ok(()),
        )
    }

    pub(crate) fn resolve_proposal_observed<A, R, C>(
        tx: &ImmediateTransaction<'_>,
        resolution_sequence: u64,
        resolution: &MemoryProposalResolution,
        approval: &ApprovalRecord,
        mut after_approval: A,
        mut after_resolution: R,
        mut after_current: C,
    ) -> Result<(), PersistenceError>
    where
        A: FnMut(&rusqlite::Transaction<'_>) -> Result<(), PersistenceError>,
        R: FnMut(&rusqlite::Transaction<'_>) -> Result<(), PersistenceError>,
        C: FnMut(&rusqlite::Transaction<'_>) -> Result<(), PersistenceError>,
    {
        with_operation_savepoint(tx, || {
            Self::resolve_proposal_inner(
                tx,
                resolution_sequence,
                resolution,
                approval,
                &mut after_approval,
                &mut after_resolution,
                &mut after_current,
            )
        })
    }

    fn resolve_proposal_inner<A, R, C>(
        tx: &ImmediateTransaction<'_>,
        resolution_sequence: u64,
        resolution: &MemoryProposalResolution,
        approval: &ApprovalRecord,
        after_approval: &mut A,
        after_resolution: &mut R,
        after_current: &mut C,
    ) -> Result<(), PersistenceError>
    where
        A: FnMut(&rusqlite::Transaction<'_>) -> Result<(), PersistenceError>,
        R: FnMut(&rusqlite::Transaction<'_>) -> Result<(), PersistenceError>,
        C: FnMut(&rusqlite::Transaction<'_>) -> Result<(), PersistenceError>,
    {
        validate_event(tx, resolution_sequence, resolution.resolution_event_id())?;
        let (proposal, status, stored_resolution) =
            Self::load_proposal(tx, resolution.proposal().proposal_id())?
                .ok_or(PersistenceError::MemoryRowMismatch)?;
        if proposal.reference() != *resolution.proposal() {
            return Err(PersistenceError::MemoryRowMismatch);
        }
        let expected = resolution_row(resolution, resolution_sequence)?;
        if status != MemoryProposalStatus::Pending {
            let stored_row = load_resolution(tx, resolution.proposal().proposal_id())?
                .ok_or(PersistenceError::MemoryRowMismatch)?;
            let stored_approval = load_approval(tx, approval.approval_id())?
                .ok_or(PersistenceError::MemoryRowMismatch)?;
            if stored_resolution.as_ref() == Some(resolution)
                && stored_row == expected
                && approval_equal(
                    &stored_approval,
                    approval,
                    Some(resolution.resolution_event_id()),
                )
            {
                return Ok(());
            }
            return Err(PersistenceError::MemoryRowMismatch);
        }
        validate_memory_approval(&proposal, approval, Some(resolution))?;
        update_approval_resolution(tx, approval, resolution)?;
        after_approval(tx.transaction())?;
        let existing = load_resolution(tx, resolution.proposal().proposal_id())?;
        match existing {
            None => {
                insert_resolution_row(tx, &expected)?;
                after_resolution(tx.transaction())?;
                insert_proposal_status(
                    tx,
                    &proposal,
                    resolution.status(),
                    Some(resolution.resolution_event_id()),
                )?;
                after_current(tx.transaction())
            }
            Some(row) if row == expected => validate_proposal_status(
                tx,
                &proposal,
                resolution.status(),
                Some(resolution.resolution_event_id()),
            ),
            _ => Err(PersistenceError::MemoryRowMismatch),
        }
    }

    pub fn list_proposals(
        tx: &ImmediateTransaction<'_>,
        namespace: MemoryNamespaceId,
        filter: MemoryProposalFilter,
        limit: u16,
    ) -> Result<MemoryProposalsPage, PersistenceError> {
        validate_proposal_rows(tx, namespace)?;
        let filter_text = proposal_filter(filter);
        let total_count = checked_count(
            tx,
            "SELECT COUNT(*) FROM current_memory_proposal_status WHERE memory_namespace_id=?1 AND (?2='all' OR status='pending')",
            params![namespace.to_string(), filter_text],
        )?;
        let mut statement = tx
            .transaction()
            .prepare(
                "SELECT p.proposal_id,p.version,p.proposer_profile_id,p.proposer_profile_version_id,
                        p.proposer_profile_version,p.proposer_profile_digest,p.memory_namespace_id,
                        p.operation,p.display_key,p.normalized_key,p.expected_kind,p.expected_entry_id,
                        p.expected_entry_version_id,p.expected_entry_version,p.expected_entry_digest,
                        p.candidate_value,p.candidate_value_bytes,p.candidate_purpose_tags_json,
                        p.rationale,p.plaintext_validation_version,p.created_at_ms,
                        p.creation_event_sequence,p.creation_event_id,p.approval_id,p.content_digest,
                        p.record_digest,p.record_json,c.status
                   FROM current_memory_proposal_status AS c
             JOIN memory_proposals AS p ON p.proposal_id=c.proposal_id
             WHERE c.memory_namespace_id=?1 AND (?2='all' OR c.status='pending')
             ORDER BY CASE WHEN ?2='pending' THEN p.created_at_ms END ASC,
                      CASE WHEN ?2='all' THEN p.created_at_ms END DESC, p.proposal_id ASC LIMIT ?3",
            )
            .map_err(query)?;
        let ids = statement
            .query_map(
                params![
                    namespace.to_string(),
                    filter_text,
                    i64::from(page_limit(limit))
                ],
                |r| Ok((decode_proposal_stored_row(r)?, r.get::<_, String>(27)?)),
            )
            .map_err(query)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(query)?;
        let mut records = Vec::with_capacity(ids.len());
        for (row, status) in ids {
            let proposal = decode_proposal_row_value(row)?;
            let status = parse_proposal_status(&status)?;
            let operation = match proposal.operation() {
                MemoryProposalOperation::Set { .. } => MemoryProposalOperationKind::Set,
                MemoryProposalOperation::Delete => MemoryProposalOperationKind::Delete,
            };
            records.push(MemoryProposalListRecord {
                proposal: proposal.reference(),
                proposer: proposal.proposer().clone(),
                operation,
                display_key: proposal.display_key().to_owned(),
                status,
                created_at_ms: proposal.created_at_ms(),
            });
        }
        page_proposals(records, total_count)
    }

    pub fn load_proposal(
        tx: &ImmediateTransaction<'_>,
        proposal_id: MemoryProposalId,
    ) -> Result<
        Option<(
            MemoryProposal,
            MemoryProposalStatus,
            Option<MemoryProposalResolution>,
        )>,
        PersistenceError,
    > {
        let Some(row) = load_proposal_row_by_id(tx, proposal_id)? else {
            return Ok(None);
        };
        let proposal = decode_proposal_row(tx, row)?;
        let (status, status_event) = load_proposal_status(tx, &proposal)?;
        let resolution = match load_resolution(tx, proposal_id)? {
            Some(row) => Some(decode_resolution_row(tx, &proposal, row)?),
            None => None,
        };
        validate_proposal_coherence(tx, &proposal, status, status_event, resolution.as_ref())?;
        Self::validate_proposal_context(tx, &proposal)?;
        Ok(Some((proposal, status, resolution)))
    }

    pub fn count_pending_proposals(
        tx: &ImmediateTransaction<'_>,
        namespace: MemoryNamespaceId,
    ) -> Result<u64, PersistenceError> {
        validate_proposal_rows(tx, namespace)?;
        checked_count(
            tx,
            "SELECT COUNT(*) FROM current_memory_proposal_status WHERE memory_namespace_id=?1 AND status='pending'",
            [namespace.to_string()],
        )
    }
    pub fn count_active_entries(
        tx: &ImmediateTransaction<'_>,
        namespace: MemoryNamespaceId,
    ) -> Result<u64, PersistenceError> {
        validate_current_entry_rows(tx, namespace)?;
        checked_count(
            tx,
            "SELECT COUNT(*) FROM current_memory_entries WHERE memory_namespace_id=?1 AND state='present'",
            [namespace.to_string()],
        )
    }
    pub fn load_pending_proposals_for_key(
        tx: &ImmediateTransaction<'_>,
        namespace: MemoryNamespaceId,
        key: &NormalizedMemoryKey,
    ) -> Result<Vec<MemoryProposal>, PersistenceError> {
        validate_proposal_rows(tx, namespace)?;
        let mut statement=tx.transaction().prepare(
            "SELECT p.proposal_id,p.version,p.proposer_profile_id,p.proposer_profile_version_id,
                    p.proposer_profile_version,p.proposer_profile_digest,p.memory_namespace_id,
                    p.operation,p.display_key,p.normalized_key,p.expected_kind,p.expected_entry_id,
                    p.expected_entry_version_id,p.expected_entry_version,p.expected_entry_digest,
                    p.candidate_value,p.candidate_value_bytes,p.candidate_purpose_tags_json,
                    p.rationale,p.plaintext_validation_version,p.created_at_ms,
                    p.creation_event_sequence,p.creation_event_id,p.approval_id,p.content_digest,
                    p.record_digest,p.record_json
               FROM current_memory_proposal_status c
               JOIN memory_proposals p ON p.proposal_id=c.proposal_id
              WHERE c.memory_namespace_id=?1 AND c.normalized_key=?2 AND c.status='pending'
              ORDER BY p.created_at_ms ASC,p.proposal_id ASC"
        ).map_err(query)?;
        let rows = statement
            .query_map(params![namespace.to_string(), key.as_str()], |r| {
                decode_proposal_stored_row(r)
            })
            .map_err(query)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(query)?;
        rows.into_iter().map(decode_proposal_row_value).collect()
    }
    pub fn load_memory_approval(
        tx: &ImmediateTransaction<'_>,
        approval_id: ApprovalId,
    ) -> Result<Option<ApprovalRecord>, PersistenceError> {
        let Some(stored) = load_approval(tx, approval_id)? else {
            return Ok(None);
        };
        if stored.record.action() != ApprovalAction::MemoryMutation
            || stored.record.object().kind != "memory_proposal"
        {
            return Err(PersistenceError::MemoryRowMismatch);
        }
        let proposal_id: MemoryProposalId = parse_id(&stored.record.object().id)?;
        let (proposal, _, _) =
            Self::load_proposal(tx, proposal_id)?.ok_or(PersistenceError::MemoryRowMismatch)?;
        if proposal.approval_id() != approval_id {
            return Err(PersistenceError::MemoryRowMismatch);
        }
        Ok(Some(stored.record))
    }

    pub fn validate_projection(
        tx: &ImmediateTransaction<'_>,
        projection: &MemoryProjection,
    ) -> Result<(), PersistenceError> {
        for ((namespace, key), reference) in projection.current_entries() {
            let entry = Self::load_current_entry(tx, *namespace, key)?
                .ok_or(PersistenceError::MemoryRowMismatch)?;
            if entry.reference() != *reference {
                return Err(PersistenceError::MemoryRowMismatch);
            }
        }
        for (id, projected) in projection.proposals() {
            let (proposal, status, resolution) =
                Self::load_proposal(tx, *id)?.ok_or(PersistenceError::MemoryRowMismatch)?;
            if proposal.reference() != *projected.proposal()
                || proposal.namespace_id() != projected.namespace_id()
                || proposal.normalized_key() != projected.normalized_key()
                || proposal.expected() != projected.expected()
                || proposal.approval_id() != projected.approval_id()
                || status != projected.status()
                || resolution
                    .as_ref()
                    .map(MemoryProposalResolution::resolution_event_id)
                    != projected.resolution_event_id()
            {
                return Err(PersistenceError::MemoryRowMismatch);
            }
        }
        Ok(())
    }

    pub(crate) fn validate_entry_context(
        tx: &ImmediateTransaction<'_>,
        entry: &MemoryEntryVersion,
    ) -> Result<(), PersistenceError> {
        let reference = entry.reference();
        match entry.predecessor_version_id() {
            None if reference.version().get() == 1 => {}
            Some(id) => {
                let predecessor =
                    load_entry_by_version_id(tx, id)?.ok_or(PersistenceError::MemoryRowMismatch)?;
                let p = predecessor.reference();
                if p.namespace_id() != reference.namespace_id()
                    || p.normalized_key() != reference.normalized_key()
                    || p.entry_id() != reference.entry_id()
                    || p.version().get().checked_add(1) != Some(reference.version().get())
                {
                    return Err(PersistenceError::MemoryRowMismatch);
                }
            }
            _ => return Err(PersistenceError::MemoryRowMismatch),
        }
        if let Some(proposal_ref) = entry.accepted_proposal() {
            let (proposal, status, _) = Self::load_proposal(tx, proposal_ref.proposal_id())?
                .ok_or(PersistenceError::MemoryRowMismatch)?;
            if proposal.reference() != *proposal_ref || status != MemoryProposalStatus::Accepted {
                return Err(PersistenceError::MemoryRowMismatch);
            }
        }
        Ok(())
    }

    pub(crate) fn validate_proposal_context(
        tx: &ImmediateTransaction<'_>,
        proposal: &MemoryProposal,
    ) -> Result<(), PersistenceError> {
        Self::validate_proposal_base_context(tx, proposal)?;
        let approval = load_approval(tx, proposal.approval_id())?
            .ok_or(PersistenceError::MemoryRowMismatch)?;
        validate_proposal_approval_binding(proposal, &approval.record)
    }

    fn validate_proposal_base_context(
        tx: &ImmediateTransaction<'_>,
        proposal: &MemoryProposal,
    ) -> Result<(), PersistenceError> {
        let profile = load_exact_profile(tx, proposal.proposer())?;
        if profile.memory_namespace_id() != proposal.namespace_id() {
            return Err(PersistenceError::MemoryRowMismatch);
        }
        match proposal.expected() {
            ExpectedMemoryEntryState::Absent => {}
            ExpectedMemoryEntryState::Present(reference)
            | ExpectedMemoryEntryState::Deleted(reference) => {
                let entry = Self::load_entry_version(
                    tx,
                    proposal.namespace_id(),
                    proposal.normalized_key(),
                    reference.version(),
                )?
                .ok_or(PersistenceError::MemoryRowMismatch)?;
                if entry.reference() != *reference {
                    return Err(PersistenceError::MemoryRowMismatch);
                }
            }
        }
        Ok(())
    }

    pub(crate) fn validate_summary_context(
        tx: &ImmediateTransaction<'_>,
        summary: &EpisodicSummary,
    ) -> Result<(), PersistenceError> {
        Self::validate_summary_base_context(tx, summary)?;
        let sources = summary
            .sources()
            .iter()
            .map(|source| {
                serde_json::json!({
                    "sequence": source.sequence(),
                    "event_id": source.event_id().to_string(),
                    "event_type": source.event_type(),
                    "event_digest": source.event_digest().as_str(),
                })
            })
            .collect::<Vec<_>>();
        let sources_json = canonical_json_bytes(&sources).map_err(integrity)?;
        let mismatch: bool = tx
            .transaction()
            .query_row(
                "SELECT EXISTS(
                 SELECT 1 FROM json_each(CAST(?1 AS TEXT)) source
            LEFT JOIN event_stream event
                   ON event.sequence=json_extract(source.value,'$.sequence')
                  AND event.event_id=json_extract(source.value,'$.event_id')
                  AND event.event_type=json_extract(source.value,'$.event_type')
                  AND event.event_digest=json_extract(source.value,'$.event_digest')
                WHERE event.sequence IS NULL
                   OR json_extract(source.value,'$.sequence')>=?2
             )",
                params![sources_json, to_i64(summary.creation_event_sequence())?],
                |row| row.get(0),
            )
            .map_err(query)?;
        if mismatch {
            return Err(PersistenceError::MemoryRowMismatch);
        }
        Ok(())
    }

    fn validate_summary_base_context(
        tx: &ImmediateTransaction<'_>,
        summary: &EpisodicSummary,
    ) -> Result<(), PersistenceError> {
        let profile = load_exact_profile(tx, summary.reference().profile())?;
        if profile.memory_namespace_id() != summary.reference().namespace_id() {
            return Err(PersistenceError::MemoryRowMismatch);
        }
        validate_event(
            tx,
            summary.creation_event_sequence(),
            summary.creation_event_id(),
        )?;
        Ok(())
    }

    #[doc(hidden)]
    pub fn insert_episodic_summary(
        tx: &ImmediateTransaction<'_>,
        creation_sequence: u64,
        summary: &EpisodicSummary,
    ) -> Result<(), PersistenceError> {
        if creation_sequence != summary.creation_event_sequence() {
            return Err(PersistenceError::MemoryRowMismatch);
        }
        let expected = summary_row(summary)?;
        let existing = load_summary_row(tx, summary.reference().summary_id())?;
        match existing {
            None => with_operation_savepoint(tx, || {
                Self::validate_summary_context(tx, summary)?;
                insert_summary_row(tx, &expected)?;
                for (ordinal, source) in summary.sources().iter().enumerate() {
                    tx.transaction().execute("INSERT INTO episodic_summary_sources (summary_id,source_ordinal,event_sequence,event_id,event_type,event_digest) VALUES (?1,?2,?3,?4,?5,?6)",params![expected.summary_id,to_i64(ordinal as u64)?,to_i64(source.sequence())?,source.event_id().to_string(),source.event_type(),source.event_digest().as_str()]).map_err(query)?;
                }
                Ok(())
            }),
            Some(row) if row == expected => {
                validate_summary_sources(tx, summary)?;
                Ok(())
            }
            _ => Err(PersistenceError::MemoryRowMismatch),
        }
    }

    pub fn list_episodic_summaries(
        tx: &ImmediateTransaction<'_>,
        namespace: MemoryNamespaceId,
        limit: u16,
    ) -> Result<EpisodicSummariesPage, PersistenceError> {
        validate_summary_rows(tx, namespace)?;
        let total_count = checked_count(
            tx,
            "SELECT COUNT(*) FROM episodic_summaries WHERE memory_namespace_id=?1",
            [namespace.to_string()],
        )?;
        let mut statement = tx
            .transaction()
            .prepare(
                "SELECT summary_id,version,memory_namespace_id,profile_id,profile_version_id,
                    profile_version,profile_content_digest,label,body,purpose_tags_json,
                    source_count,plaintext_validation_version,created_at_ms,creation_event_sequence,
                    creation_event_id,source_set_digest,content_digest,record_digest,record_json
               FROM episodic_summaries
              WHERE memory_namespace_id=?1
              ORDER BY created_at_ms DESC,summary_id ASC LIMIT ?2",
            )
            .map_err(query)?;
        let rows = statement
            .query_map(
                params![namespace.to_string(), i64::from(page_limit(limit))],
                decode_summary_stored_row,
            )
            .map_err(query)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(query)?;
        let mut records = Vec::with_capacity(rows.len());
        for row in rows {
            let summary = decode_summary_row_value(row)?;
            records.push(EpisodicSummaryListRecord {
                summary: summary.reference(),
                label: summary.label().to_owned(),
                purpose_tags: summary.purpose_tags().to_vec(),
                source_count: u64::try_from(summary.sources().len())
                    .map_err(|_| PersistenceError::MemoryRowMismatch)?,
                created_at_ms: summary.created_at_ms(),
            });
        }
        page_summaries(records, total_count)
    }
    pub fn load_episodic_summary(
        tx: &ImmediateTransaction<'_>,
        summary_id: EpisodicSummaryId,
    ) -> Result<Option<EpisodicSummary>, PersistenceError> {
        let Some(row) = load_summary_row(tx, summary_id)? else {
            return Ok(None);
        };
        let summary = decode_summary_row(tx, row)?;
        Ok(Some(summary))
    }

    pub fn build_snapshot(
        tx: &ImmediateTransaction<'_>,
        request: &MemoryRetrievalRequest,
    ) -> Result<MemorySnapshot, PersistenceError> {
        let scope = request.scope();
        let profile = load_exact_profile(tx, scope.profile())?;
        scope.validate_against(&profile).map_err(integrity)?;
        let mut builder = MemorySnapshotBuilder::new(request.clone()).map_err(integrity)?;
        let namespace = scope.namespace_id();
        validate_current_entry_rows(tx, namespace)?;
        validate_summary_rows(tx, namespace)?;
        match scope.purpose() {
            MemoryPurposeScope::General => {
                stream_entries(
                    tx,
                    namespace,
                    "json_array_length(json_extract(CAST(v.record_json AS TEXT),'$.purpose_tags'))=0",
                    Vec::new(),
                    &mut builder,
                )?;
                stream_summaries(
                    tx,
                    namespace,
                    scope.profile(),
                    "json_array_length(json_extract(CAST(s.record_json AS TEXT),'$.purpose_tags'))=0",
                    Vec::new(),
                    &mut builder,
                )?;
            }
            MemoryPurposeScope::Tagged(tags) => {
                let clause = tag_clause(
                    "json_extract(CAST(v.record_json AS TEXT),'$.purpose_tags')",
                    2,
                    tags.len(),
                );
                let values = tags.iter().cloned().map(Value::Text).collect();
                stream_entries(tx, namespace, &clause, values, &mut builder)?;
                stream_entries(
                    tx,
                    namespace,
                    "json_array_length(json_extract(CAST(v.record_json AS TEXT),'$.purpose_tags'))=0",
                    Vec::new(),
                    &mut builder,
                )?;
                let clause = tag_clause(
                    "json_extract(CAST(s.record_json AS TEXT),'$.purpose_tags')",
                    6,
                    tags.len(),
                );
                let values = tags.iter().cloned().map(Value::Text).collect();
                stream_summaries(
                    tx,
                    namespace,
                    scope.profile(),
                    &clause,
                    values,
                    &mut builder,
                )?;
                stream_summaries(
                    tx,
                    namespace,
                    scope.profile(),
                    "json_array_length(json_extract(CAST(s.record_json AS TEXT),'$.purpose_tags'))=0",
                    Vec::new(),
                    &mut builder,
                )?;
            }
        }
        builder.finish().map_err(integrity)
    }
}

fn with_operation_savepoint<T>(
    tx: &ImmediateTransaction<'_>,
    operation: impl FnOnce() -> Result<T, PersistenceError>,
) -> Result<T, PersistenceError> {
    tx.transaction()
        .execute_batch("SAVEPOINT memory_repository_operation")
        .map_err(query)?;
    match operation() {
        Ok(value) => {
            release_operation_savepoint(
                tx.transaction(),
                tx.transaction()
                    .execute_batch("RELEASE memory_repository_operation"),
            )?;
            Ok(value)
        }
        Err(error) => {
            tx.transaction()
                .execute_batch(
                    "ROLLBACK TO memory_repository_operation; RELEASE memory_repository_operation",
                )
                .map_err(query)?;
            Err(error)
        }
    }
}

fn release_operation_savepoint(
    transaction: &rusqlite::Transaction<'_>,
    release: rusqlite::Result<()>,
) -> Result<(), PersistenceError> {
    match release {
        Ok(()) => Ok(()),
        Err(error) => {
            let mapped = query(error);
            let _ = transaction.execute_batch(
                "ROLLBACK TO memory_repository_operation; RELEASE memory_repository_operation",
            );
            Err(mapped)
        }
    }
}

// Stored representations deliberately include every immutable SQLite column.
#[derive(Debug, Clone, PartialEq, Eq)]
struct EntryRow {
    namespace: String,
    entry_id: String,
    entry_version_id: String,
    version: i64,
    predecessor: Option<String>,
    display_key: String,
    normalized_key: String,
    state: String,
    value: Option<String>,
    value_bytes: i64,
    tags: Vec<u8>,
    created_by_kind: String,
    created_by_id: Option<String>,
    created_at: i64,
    accepted_id: Option<String>,
    accepted_version: Option<i64>,
    accepted_digest: Option<String>,
    plaintext: i64,
    creation_sequence: i64,
    creation_event_id: String,
    content_digest: String,
    record_digest: String,
    record_json: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
struct ProposalRow {
    proposal_id: String,
    version: i64,
    profile_id: String,
    profile_version_id: String,
    profile_version: i64,
    profile_digest: String,
    namespace: String,
    operation: String,
    display_key: String,
    normalized_key: String,
    expected_kind: String,
    expected_id: Option<String>,
    expected_version_id: Option<String>,
    expected_version: Option<i64>,
    expected_digest: Option<String>,
    candidate_value: Option<String>,
    candidate_value_bytes: Option<i64>,
    candidate_tags: Option<Vec<u8>>,
    rationale: String,
    plaintext: i64,
    created_at: i64,
    creation_sequence: i64,
    creation_event_id: String,
    approval_id: String,
    content_digest: String,
    record_digest: String,
    record_json: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolutionRow {
    proposal_id: String,
    proposal_version: i64,
    proposal_digest: String,
    status: String,
    approval_id: String,
    resolved_by_kind: String,
    resolved_by_id: Option<String>,
    resolved_at: i64,
    resolution_sequence: i64,
    resolution_event_id: String,
    resolution_json: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
struct SummaryRow {
    summary_id: String,
    version: i64,
    namespace: String,
    profile_id: String,
    profile_version_id: String,
    profile_version: i64,
    profile_digest: String,
    label: String,
    body: String,
    tags: Vec<u8>,
    source_count: i64,
    plaintext: i64,
    created_at: i64,
    creation_sequence: i64,
    creation_event_id: String,
    source_digest: String,
    content_digest: String,
    record_digest: String,
    record_json: Vec<u8>,
}
#[derive(Clone)]
struct StoredApproval {
    record: ApprovalRecord,
    resolution_event_id: Option<EventId>,
}
type ApprovalColumns = (
    String,
    String,
    String,
    i64,
    String,
    Option<String>,
    String,
    Option<String>,
    String,
    i64,
    Option<i64>,
    Option<i64>,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
);

fn approval_columns_equal(left: &ApprovalColumns, right: &ApprovalColumns) -> bool {
    left.0 == right.0
        && left.1 == right.1
        && left.2 == right.2
        && left.3 == right.3
        && left.4 == right.4
        && left.5 == right.5
        && left.6 == right.6
        && left.7 == right.7
        && left.8 == right.8
        && left.9 == right.9
        && left.10 == right.10
        && left.11 == right.11
        && left.12 == right.12
        && left.13 == right.13
        && left.14 == right.14
        && left.15 == right.15
}

fn entry_row(
    entry: &MemoryEntryVersion,
    creation_sequence: u64,
) -> Result<EntryRow, PersistenceError> {
    let r = entry.reference();
    let (created_by_kind, created_by_id) = actor_columns(entry.created_by());
    let accepted = entry.accepted_proposal();
    let record_json = canonical_json_bytes(entry).map_err(integrity)?;
    Ok(EntryRow {
        namespace: r.namespace_id().to_string(),
        entry_id: r.entry_id().to_string(),
        entry_version_id: r.entry_version_id().to_string(),
        version: to_i64(r.version().get())?,
        predecessor: entry.predecessor_version_id().map(|x| x.to_string()),
        display_key: entry.display_key().to_owned(),
        normalized_key: r.normalized_key().as_str().to_owned(),
        state: entry_state(r.state()).to_owned(),
        value: entry.value().map(str::to_owned),
        value_bytes: to_i64(entry.value().map_or(0, |x| x.len() as u64))?,
        tags: canonical_json_bytes(&entry.purpose_tags().to_vec()).map_err(integrity)?,
        created_by_kind,
        created_by_id,
        created_at: entry.created_at_ms(),
        accepted_id: accepted.map(|x| x.proposal_id().to_string()),
        accepted_version: accepted.map(|x| to_i64(x.version().get())).transpose()?,
        accepted_digest: accepted.map(|x| x.content_digest().as_str().to_owned()),
        plaintext: i64::from(entry.plaintext_validation_version()),
        creation_sequence: to_i64(creation_sequence)?,
        creation_event_id: entry.creation_event_id().to_string(),
        content_digest: r.content_digest().as_str().to_owned(),
        record_digest: entry.record_digest().as_str().to_owned(),
        record_json,
    })
}
fn proposal_row(p: &MemoryProposal, sequence: u64) -> Result<ProposalRow, PersistenceError> {
    let operation = p.operation();
    let (operation, candidate_value, candidate_value_bytes, candidate_tags) = match operation {
        MemoryProposalOperation::Set { candidate } => (
            "set",
            Some(candidate.value().to_owned()),
            Some(to_i64(candidate.value().len() as u64)?),
            Some(canonical_json_bytes(&candidate.purpose_tags().to_vec()).map_err(integrity)?),
        ),
        MemoryProposalOperation::Delete => ("delete", None, None, None),
    };
    let (expected_kind, expected_id, expected_version_id, expected_version, expected_digest) =
        match p.expected() {
            ExpectedMemoryEntryState::Absent => ("absent", None, None, None, None),
            ExpectedMemoryEntryState::Present(r) => (
                "present",
                Some(r.entry_id().to_string()),
                Some(r.entry_version_id().to_string()),
                Some(to_i64(r.version().get())?),
                Some(r.content_digest().as_str().to_owned()),
            ),
            ExpectedMemoryEntryState::Deleted(r) => (
                "deleted",
                Some(r.entry_id().to_string()),
                Some(r.entry_version_id().to_string()),
                Some(to_i64(r.version().get())?),
                Some(r.content_digest().as_str().to_owned()),
            ),
        };
    let record_json = canonical_json_bytes(p).map_err(integrity)?;
    let profile = p.proposer();
    Ok(ProposalRow {
        proposal_id: p.reference().proposal_id().to_string(),
        version: to_i64(p.reference().version().get())?,
        profile_id: profile.profile_id().to_string(),
        profile_version_id: profile.profile_version_id().to_string(),
        profile_version: to_i64(profile.version().get())?,
        profile_digest: profile.content_digest().as_str().to_owned(),
        namespace: p.namespace_id().to_string(),
        operation: operation.to_owned(),
        display_key: p.display_key().to_owned(),
        normalized_key: p.normalized_key().as_str().to_owned(),
        expected_kind: expected_kind.to_owned(),
        expected_id,
        expected_version_id,
        expected_version,
        expected_digest,
        candidate_value,
        candidate_value_bytes,
        candidate_tags,
        rationale: p.rationale().to_owned(),
        plaintext: i64::from(p.plaintext_validation_version()),
        created_at: p.created_at_ms(),
        creation_sequence: to_i64(sequence)?,
        creation_event_id: p.creation_event_id().to_string(),
        approval_id: p.approval_id().to_string(),
        content_digest: p.content_digest().as_str().to_owned(),
        record_digest: p.record_digest().as_str().to_owned(),
        record_json,
    })
}
fn resolution_row(
    r: &MemoryProposalResolution,
    sequence: u64,
) -> Result<ResolutionRow, PersistenceError> {
    let json = canonical_json_bytes(r).map_err(integrity)?;
    Ok(ResolutionRow {
        proposal_id: r.proposal().proposal_id().to_string(),
        proposal_version: to_i64(r.proposal().version().get())?,
        proposal_digest: r.proposal().content_digest().as_str().to_owned(),
        status: proposal_status(r.status()).to_owned(),
        approval_id: r.approval_id().to_string(),
        resolved_by_kind: "human".to_owned(),
        resolved_by_id: None,
        resolved_at: r.resolved_at_ms(),
        resolution_sequence: to_i64(sequence)?,
        resolution_event_id: r.resolution_event_id().to_string(),
        resolution_json: json,
    })
}
fn summary_row(s: &EpisodicSummary) -> Result<SummaryRow, PersistenceError> {
    let r = s.reference();
    let p = r.profile();
    let json = canonical_json_bytes(s).map_err(integrity)?;
    Ok(SummaryRow {
        summary_id: r.summary_id().to_string(),
        version: to_i64(r.version().get())?,
        namespace: r.namespace_id().to_string(),
        profile_id: p.profile_id().to_string(),
        profile_version_id: p.profile_version_id().to_string(),
        profile_version: to_i64(p.version().get())?,
        profile_digest: p.content_digest().as_str().to_owned(),
        label: s.label().to_owned(),
        body: s.body().to_owned(),
        tags: canonical_json_bytes(&s.purpose_tags().to_vec()).map_err(integrity)?,
        source_count: to_i64(s.sources().len() as u64)?,
        plaintext: i64::from(s.plaintext_validation_version()),
        created_at: s.created_at_ms(),
        creation_sequence: to_i64(s.creation_event_sequence())?,
        creation_event_id: s.creation_event_id().to_string(),
        source_digest: s.source_set_digest().as_str().to_owned(),
        content_digest: s.content_digest().as_str().to_owned(),
        record_digest: s.record_digest().as_str().to_owned(),
        record_json: json,
    })
}

fn insert_entry_row(tx: &Connection, r: &EntryRow) -> Result<(), PersistenceError> {
    tx.execute("INSERT INTO memory_entry_versions (memory_namespace_id,entry_id,entry_version_id,version,predecessor_version_id,display_key,normalized_key,state,value_text,value_bytes,purpose_tags_json,created_by_kind,created_by_id,created_at_ms,accepted_proposal_id,accepted_proposal_version,accepted_proposal_digest,plaintext_validation_version,creation_event_sequence,creation_event_id,content_digest,record_digest,record_json) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23)",params![r.namespace,r.entry_id,r.entry_version_id,r.version,r.predecessor,r.display_key,r.normalized_key,r.state,r.value,r.value_bytes,r.tags,r.created_by_kind,r.created_by_id,r.created_at,r.accepted_id,r.accepted_version,r.accepted_digest,r.plaintext,r.creation_sequence,r.creation_event_id,r.content_digest,r.record_digest,r.record_json]).map(|_|()).map_err(query)
}
fn load_entry_rows_matching(
    tx: &Connection,
    e: &EntryRow,
) -> Result<Vec<EntryRow>, PersistenceError> {
    let mut s=tx.prepare("SELECT memory_namespace_id,entry_id,entry_version_id,version,predecessor_version_id,display_key,normalized_key,state,value_text,value_bytes,purpose_tags_json,created_by_kind,created_by_id,created_at_ms,accepted_proposal_id,accepted_proposal_version,accepted_proposal_digest,plaintext_validation_version,creation_event_sequence,creation_event_id,content_digest,record_digest,record_json FROM memory_entry_versions WHERE (entry_id=?1 AND version=?2) OR entry_version_id=?3 OR (memory_namespace_id=?4 AND normalized_key=?5 AND entry_id<>?1) ORDER BY entry_id,version").map_err(query)?;
    s.query_map(
        params![
            e.entry_id,
            e.version,
            e.entry_version_id,
            e.namespace,
            e.normalized_key
        ],
        decode_entry_row,
    )
    .map_err(query)?
    .collect::<Result<Vec<_>, _>>()
    .map_err(query)
}
fn load_entry_by_version_id(
    tx: &ImmediateTransaction<'_>,
    id: MemoryEntryVersionId,
) -> Result<Option<MemoryEntryVersion>, PersistenceError> {
    let row=tx.transaction().query_row("SELECT memory_namespace_id,entry_id,entry_version_id,version,predecessor_version_id,display_key,normalized_key,state,value_text,value_bytes,purpose_tags_json,created_by_kind,created_by_id,created_at_ms,accepted_proposal_id,accepted_proposal_version,accepted_proposal_digest,plaintext_validation_version,creation_event_sequence,creation_event_id,content_digest,record_digest,record_json FROM memory_entry_versions WHERE entry_version_id=?1",[id.to_string()],decode_entry_row).optional().map_err(query)?;
    row.map(|r| decode_entry_row_checked(tx, r)).transpose()
}
fn decode_entry_row(row: &Row<'_>) -> rusqlite::Result<EntryRow> {
    Ok(EntryRow {
        namespace: row.get(0)?,
        entry_id: row.get(1)?,
        entry_version_id: row.get(2)?,
        version: row.get(3)?,
        predecessor: row.get(4)?,
        display_key: row.get(5)?,
        normalized_key: row.get(6)?,
        state: row.get(7)?,
        value: row.get(8)?,
        value_bytes: row.get(9)?,
        tags: row.get(10)?,
        created_by_kind: row.get(11)?,
        created_by_id: row.get(12)?,
        created_at: row.get(13)?,
        accepted_id: row.get(14)?,
        accepted_version: row.get(15)?,
        accepted_digest: row.get(16)?,
        plaintext: row.get(17)?,
        creation_sequence: row.get(18)?,
        creation_event_id: row.get(19)?,
        content_digest: row.get(20)?,
        record_digest: row.get(21)?,
        record_json: row.get(22)?,
    })
}
fn decode_entry_row_checked(
    tx: &ImmediateTransaction<'_>,
    r: EntryRow,
) -> Result<MemoryEntryVersion, PersistenceError> {
    let creation_sequence = to_u64(r.creation_sequence)?;
    let entry = decode_entry_row_value(r)?;
    validate_event(tx, creation_sequence, entry.creation_event_id())?;
    MemoryRepository::validate_entry_context(tx, &entry)?;
    Ok(entry)
}

fn decode_entry_row_value(r: EntryRow) -> Result<MemoryEntryVersion, PersistenceError> {
    let entry: MemoryEntryVersion =
        serde_json::from_slice(&r.record_json).map_err(|_| PersistenceError::MemoryRowMismatch)?;
    let expected = entry_row(&entry, to_u64(r.creation_sequence)?)?;
    if expected != r {
        return Err(PersistenceError::MemoryRowMismatch);
    };
    Ok(entry)
}

fn validate_entry_batch_context(
    tx: &ImmediateTransaction<'_>,
    entries: &[MemoryEntryVersion],
) -> Result<(), PersistenceError> {
    let predecessor_ids = entries
        .iter()
        .filter_map(MemoryEntryVersion::predecessor_version_id)
        .map(|id| id.to_string())
        .collect::<Vec<_>>();
    let predecessor_json = canonical_json_bytes(&predecessor_ids).map_err(integrity)?;
    let mut statement = tx
        .transaction()
        .prepare(
            "SELECT memory_namespace_id,entry_id,entry_version_id,version,predecessor_version_id,
                display_key,normalized_key,state,value_text,value_bytes,purpose_tags_json,
                created_by_kind,created_by_id,created_at_ms,accepted_proposal_id,
                accepted_proposal_version,accepted_proposal_digest,plaintext_validation_version,
                creation_event_sequence,creation_event_id,content_digest,record_digest,record_json
           FROM memory_entry_versions
          WHERE entry_version_id IN (SELECT value FROM json_each(CAST(?1 AS TEXT)))
          ORDER BY entry_version_id",
        )
        .map_err(query)?;
    let predecessor_rows = statement
        .query_map([predecessor_json], decode_entry_row)
        .map_err(query)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(query)?;
    let mut predecessors = BTreeMap::new();
    for row in predecessor_rows {
        let predecessor = decode_entry_row_value(row)?;
        if predecessors
            .insert(predecessor.reference().entry_version_id(), predecessor)
            .is_some()
        {
            return Err(PersistenceError::MemoryRowMismatch);
        }
    }

    // The stored sequence is authenticated by the row comparison above.  Read it
    // directly from the immutable row for the set-based event check.
    let entry_ids = entries
        .iter()
        .map(|entry| entry.reference().entry_version_id().to_string())
        .collect::<Vec<_>>();
    let entry_ids_json = canonical_json_bytes(&entry_ids).map_err(integrity)?;
    let missing_event: bool = tx
        .transaction()
        .query_row(
            "SELECT EXISTS(
             SELECT 1
               FROM memory_entry_versions AS v
          LEFT JOIN event_stream AS e
                 ON e.sequence=v.creation_event_sequence AND e.event_id=v.creation_event_id
              WHERE v.entry_version_id IN (SELECT value FROM json_each(CAST(?1 AS TEXT)))
                AND e.sequence IS NULL
         )",
            [entry_ids_json],
            |row| row.get(0),
        )
        .map_err(query)?;
    if missing_event {
        return Err(PersistenceError::MemoryRowMismatch);
    }

    for entry in entries {
        match entry.predecessor_version_id() {
            None if entry.reference().version().get() == 1 => {}
            Some(id) => {
                let predecessor = predecessors
                    .get(&id)
                    .ok_or(PersistenceError::MemoryRowMismatch)?;
                let predecessor_ref = predecessor.reference();
                let entry_ref = entry.reference();
                if predecessor_ref.namespace_id() != entry_ref.namespace_id()
                    || predecessor_ref.normalized_key() != entry_ref.normalized_key()
                    || predecessor_ref.entry_id() != entry_ref.entry_id()
                    || predecessor_ref.version().get().checked_add(1)
                        != Some(entry_ref.version().get())
                {
                    return Err(PersistenceError::MemoryRowMismatch);
                }
            }
            _ => return Err(PersistenceError::MemoryRowMismatch),
        }
    }
    validate_accepted_proposal_refs(tx, entries)?;
    if !entries.is_empty() {
        validate_entry_dependency_graph(tx, entries)?;
    }
    Ok(())
}

fn validate_accepted_proposal_refs(
    tx: &ImmediateTransaction<'_>,
    entries: &[MemoryEntryVersion],
) -> Result<(), PersistenceError> {
    let references = entries
        .iter()
        .filter_map(MemoryEntryVersion::accepted_proposal)
        .map(|reference| {
            serde_json::json!({
                "id": reference.proposal_id().to_string(),
                "version": reference.version().get(),
                "digest": reference.content_digest().as_str(),
            })
        })
        .collect::<Vec<_>>();
    let references_json = canonical_json_bytes(&references).map_err(integrity)?;
    let mismatch: bool = tx
        .transaction()
        .query_row(
            "SELECT EXISTS(
             SELECT 1
               FROM json_each(CAST(?1 AS TEXT)) AS wanted
          LEFT JOIN memory_proposals AS p
                 ON p.proposal_id=json_extract(wanted.value,'$.id')
          LEFT JOIN current_memory_proposal_status AS c ON c.proposal_id=p.proposal_id
              WHERE p.proposal_id IS NULL
                 OR p.version<>json_extract(wanted.value,'$.version')
                 OR p.content_digest<>json_extract(wanted.value,'$.digest')
                 OR c.proposal_id IS NULL
                 OR c.status IS NULL
                 OR c.status<>'accepted'
         )",
            [references_json],
            |row| row.get(0),
        )
        .map_err(query)?;
    if mismatch {
        Err(PersistenceError::MemoryRowMismatch)
    } else {
        Ok(())
    }
}

const MAX_MEMORY_DEPENDENCY_NODES: usize = 4_096;

/// Authenticates the complete entry/proposal dependency graph in a fixed set of
/// bulk phases. `UNION` makes discovery cycle-safe and de-duplicates shared
/// dependencies; no phase issues SQL per record.
fn validate_entry_dependency_graph(
    tx: &ImmediateTransaction<'_>,
    roots: &[MemoryEntryVersion],
) -> Result<(), PersistenceError> {
    let root_ids = roots
        .iter()
        .map(|entry| entry.reference().entry_version_id().to_string())
        .collect::<Vec<_>>();
    let root_ids_json = canonical_json_bytes(&root_ids).map_err(integrity)?;
    let mut statement = tx
        .transaction()
        .prepare(
            "WITH RECURSIVE dependency(kind,id) AS (
                 SELECT 'entry', CAST(value AS TEXT)
                   FROM json_each(CAST(?1 AS TEXT))
                 UNION
                 SELECT 'entry', v.predecessor_version_id
                   FROM dependency d
                   JOIN memory_entry_versions v
                     ON d.kind='entry' AND v.entry_version_id=d.id
                  WHERE v.predecessor_version_id IS NOT NULL
                 UNION
                 SELECT 'proposal', v.accepted_proposal_id
                   FROM dependency d
                   JOIN memory_entry_versions v
                     ON d.kind='entry' AND v.entry_version_id=d.id
                  WHERE v.accepted_proposal_id IS NOT NULL
                 UNION
                 SELECT 'entry', p.expected_entry_version_id
                   FROM dependency d
                   JOIN memory_proposals p
                     ON d.kind='proposal' AND p.proposal_id=d.id
                  WHERE p.expected_entry_version_id IS NOT NULL
             )
             SELECT kind,id FROM dependency ORDER BY kind,id LIMIT ?2",
        )
        .map_err(query)?;
    let nodes = statement
        .query_map(
            params![
                root_ids_json,
                to_i64((MAX_MEMORY_DEPENDENCY_NODES + 1) as u64)?
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(query)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(query)?;
    if nodes.len() > MAX_MEMORY_DEPENDENCY_NODES {
        return Err(PersistenceError::MemoryRowMismatch);
    }
    let entry_ids = nodes
        .iter()
        .filter(|(kind, _)| kind == "entry")
        .map(|(_, id)| id.clone())
        .collect::<Vec<_>>();
    let proposal_ids = nodes
        .iter()
        .filter(|(kind, _)| kind == "proposal")
        .map(|(_, id)| id.clone())
        .collect::<Vec<_>>();
    if entry_ids.len() + proposal_ids.len() != nodes.len() {
        return Err(PersistenceError::MemoryRowMismatch);
    }

    let entries = load_dependency_entries(tx, &entry_ids)?;
    let proposals = load_dependency_proposals(tx, &proposal_ids)?;

    for root in roots {
        let id = root.reference().entry_version_id().to_string();
        if entries.get(&id) != Some(root) {
            return Err(PersistenceError::MemoryRowMismatch);
        }
    }
    for entry in entries.values() {
        let reference = entry.reference();
        match entry.predecessor_version_id() {
            None if reference.version().get() == 1 => {}
            Some(id) => {
                let predecessor = entries
                    .get(&id.to_string())
                    .ok_or(PersistenceError::MemoryRowMismatch)?;
                let predecessor_ref = predecessor.reference();
                if predecessor_ref.namespace_id() != reference.namespace_id()
                    || predecessor_ref.normalized_key() != reference.normalized_key()
                    || predecessor_ref.entry_id() != reference.entry_id()
                    || predecessor_ref.version().get().checked_add(1)
                        != Some(reference.version().get())
                {
                    return Err(PersistenceError::MemoryRowMismatch);
                }
            }
            _ => return Err(PersistenceError::MemoryRowMismatch),
        }
        if let Some(accepted) = entry.accepted_proposal() {
            let (proposal, status) = proposals
                .get(&accepted.proposal_id().to_string())
                .ok_or(PersistenceError::MemoryRowMismatch)?;
            if proposal.reference() != *accepted
                || proposal.namespace_id() != reference.namespace_id()
                || proposal.normalized_key() != reference.normalized_key()
                || *status != MemoryProposalStatus::Accepted
            {
                return Err(PersistenceError::MemoryRowMismatch);
            }
        }
    }
    for (proposal, _) in proposals.values() {
        if let ExpectedMemoryEntryState::Present(reference)
        | ExpectedMemoryEntryState::Deleted(reference) = proposal.expected()
        {
            let entry = entries
                .get(&reference.entry_version_id().to_string())
                .ok_or(PersistenceError::MemoryRowMismatch)?;
            if entry.reference() != *reference {
                return Err(PersistenceError::MemoryRowMismatch);
            }
        }
    }
    validate_dependency_graph_acyclic(&entries, &proposals)?;
    Ok(())
}

fn validate_dependency_graph_acyclic(
    entries: &BTreeMap<String, MemoryEntryVersion>,
    proposals: &BTreeMap<String, (MemoryProposal, MemoryProposalStatus)>,
) -> Result<(), PersistenceError> {
    let mut outgoing = BTreeMap::<String, Vec<String>>::new();
    let mut indegree = BTreeMap::<String, usize>::new();
    for id in entries.keys() {
        indegree.insert(format!("entry:{id}"), 0);
    }
    for id in proposals.keys() {
        indegree.insert(format!("proposal:{id}"), 0);
    }
    for (id, entry) in entries {
        let source = format!("entry:{id}");
        if let Some(predecessor) = entry.predecessor_version_id() {
            add_dependency_edge(
                &mut outgoing,
                &mut indegree,
                &source,
                format!("entry:{predecessor}"),
            )?;
        }
        if let Some(accepted) = entry.accepted_proposal() {
            add_dependency_edge(
                &mut outgoing,
                &mut indegree,
                &source,
                format!("proposal:{}", accepted.proposal_id()),
            )?;
        }
    }
    for (id, (proposal, _)) in proposals {
        if let ExpectedMemoryEntryState::Present(reference)
        | ExpectedMemoryEntryState::Deleted(reference) = proposal.expected()
        {
            add_dependency_edge(
                &mut outgoing,
                &mut indegree,
                &format!("proposal:{id}"),
                format!("entry:{}", reference.entry_version_id()),
            )?;
        }
    }

    let mut ready = indegree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(node, _)| node.clone())
        .collect::<Vec<_>>();
    let mut visited = 0usize;
    while let Some(node) = ready.pop() {
        visited += 1;
        if let Some(targets) = outgoing.get(&node) {
            for target in targets {
                let degree = indegree
                    .get_mut(target)
                    .ok_or(PersistenceError::MemoryRowMismatch)?;
                *degree = degree
                    .checked_sub(1)
                    .ok_or(PersistenceError::MemoryRowMismatch)?;
                if *degree == 0 {
                    ready.push(target.clone());
                }
            }
        }
    }
    if visited == indegree.len() {
        Ok(())
    } else {
        Err(PersistenceError::MemoryRowMismatch)
    }
}

fn add_dependency_edge(
    outgoing: &mut BTreeMap<String, Vec<String>>,
    indegree: &mut BTreeMap<String, usize>,
    source: &str,
    target: String,
) -> Result<(), PersistenceError> {
    let degree = indegree
        .get_mut(&target)
        .ok_or(PersistenceError::MemoryRowMismatch)?;
    *degree = degree
        .checked_add(1)
        .ok_or(PersistenceError::MemoryRowMismatch)?;
    outgoing.entry(source.to_owned()).or_default().push(target);
    Ok(())
}

fn load_dependency_entries(
    tx: &ImmediateTransaction<'_>,
    ids: &[String],
) -> Result<BTreeMap<String, MemoryEntryVersion>, PersistenceError> {
    let ids_json = canonical_json_bytes(&ids.to_vec()).map_err(integrity)?;
    let mut statement = tx
        .transaction()
        .prepare(
            "SELECT v.memory_namespace_id,v.entry_id,v.entry_version_id,v.version,
                v.predecessor_version_id,v.display_key,v.normalized_key,v.state,v.value_text,
                v.value_bytes,v.purpose_tags_json,v.created_by_kind,v.created_by_id,v.created_at_ms,
                v.accepted_proposal_id,v.accepted_proposal_version,v.accepted_proposal_digest,
                v.plaintext_validation_version,v.creation_event_sequence,v.creation_event_id,
                v.content_digest,v.record_digest,v.record_json,
                EXISTS(SELECT 1 FROM event_stream e
                        WHERE e.sequence=v.creation_event_sequence AND e.event_id=v.creation_event_id)
           FROM memory_entry_versions v
          WHERE v.entry_version_id IN (SELECT value FROM json_each(CAST(?1 AS TEXT)))
          ORDER BY v.entry_version_id",
        )
        .map_err(query)?;
    let mut rows = statement.query([ids_json]).map_err(query)?;
    let mut entries = BTreeMap::new();
    while let Some(row) = rows.next().map_err(query)? {
        if !row.get::<_, bool>(23).map_err(query)? {
            return Err(PersistenceError::MemoryRowMismatch);
        }
        let entry = decode_entry_row_value(decode_entry_row(row).map_err(query)?)?;
        let id = entry.reference().entry_version_id().to_string();
        if entries.insert(id, entry).is_some() {
            return Err(PersistenceError::MemoryRowMismatch);
        }
    }
    if entries.len() != ids.len() {
        return Err(PersistenceError::MemoryRowMismatch);
    }
    Ok(entries)
}

fn load_dependency_proposals(
    tx: &ImmediateTransaction<'_>,
    ids: &[String],
) -> Result<BTreeMap<String, (MemoryProposal, MemoryProposalStatus)>, PersistenceError> {
    validate_dependency_proposal_coherence(tx, ids)?;
    let ids_json = canonical_json_bytes(&ids.to_vec()).map_err(integrity)?;
    let mut statement = tx
        .transaction()
        .prepare(
            "SELECT p.proposal_id,p.version,p.proposer_profile_id,p.proposer_profile_version_id,
                p.proposer_profile_version,p.proposer_profile_digest,p.memory_namespace_id,
                p.operation,p.display_key,p.normalized_key,p.expected_kind,p.expected_entry_id,
                p.expected_entry_version_id,p.expected_entry_version,p.expected_entry_digest,
                p.candidate_value,p.candidate_value_bytes,p.candidate_purpose_tags_json,
                p.rationale,p.plaintext_validation_version,p.created_at_ms,
                p.creation_event_sequence,p.creation_event_id,p.approval_id,p.content_digest,
                p.record_digest,p.record_json,
                r.proposal_id,r.proposal_version,r.proposal_content_digest,r.status,r.approval_id,
                r.resolved_by_kind,r.resolved_by_id,r.resolved_at_ms,r.resolution_event_sequence,
                r.resolution_event_id,r.resolution_json,c.status,c.resolution_event_id
           FROM memory_proposals p
      LEFT JOIN current_memory_proposal_status c ON c.proposal_id=p.proposal_id
      LEFT JOIN memory_proposal_resolutions r ON r.proposal_id=p.proposal_id
          WHERE p.proposal_id IN (SELECT value FROM json_each(CAST(?1 AS TEXT)))
          ORDER BY p.proposal_id",
        )
        .map_err(query)?;
    let mut rows = statement.query([ids_json]).map_err(query)?;
    let mut proposals = BTreeMap::new();
    let mut profile_contexts = Vec::new();
    while let Some(row) = rows.next().map_err(query)? {
        let proposal = decode_proposal_row_value(decode_proposal_stored_row(row).map_err(query)?)?;
        let status_text = row
            .get::<_, Option<String>>(38)
            .map_err(query)?
            .ok_or(PersistenceError::MemoryRowMismatch)?;
        let status = parse_proposal_status(&status_text)?;
        let status_event = row
            .get::<_, Option<String>>(39)
            .map_err(query)?
            .map(|value| parse_id(&value))
            .transpose()?;
        match (status, row.get::<_, Option<String>>(27).map_err(query)?) {
            (MemoryProposalStatus::Pending, None) if status_event.is_none() => {}
            (MemoryProposalStatus::Pending, _) => {
                return Err(PersistenceError::MemoryRowMismatch);
            }
            (_, Some(proposal_id)) => {
                let resolution = decode_resolution_row_value(
                    &proposal,
                    ResolutionRow {
                        proposal_id,
                        proposal_version: row.get(28).map_err(query)?,
                        proposal_digest: row.get(29).map_err(query)?,
                        status: row.get(30).map_err(query)?,
                        approval_id: row.get(31).map_err(query)?,
                        resolved_by_kind: row.get(32).map_err(query)?,
                        resolved_by_id: row.get(33).map_err(query)?,
                        resolved_at: row.get(34).map_err(query)?,
                        resolution_sequence: row.get(35).map_err(query)?,
                        resolution_event_id: row.get(36).map_err(query)?,
                        resolution_json: row.get(37).map_err(query)?,
                    },
                )?;
                if resolution.status() != status
                    || Some(resolution.resolution_event_id()) != status_event
                {
                    return Err(PersistenceError::MemoryRowMismatch);
                }
            }
            _ => return Err(PersistenceError::MemoryRowMismatch),
        }
        profile_contexts.push((proposal.proposer().clone(), proposal.namespace_id()));
        let id = proposal.reference().proposal_id().to_string();
        if proposals.insert(id, (proposal, status)).is_some() {
            return Err(PersistenceError::MemoryRowMismatch);
        }
    }
    drop(rows);
    drop(statement);
    if proposals.len() != ids.len() {
        return Err(PersistenceError::MemoryRowMismatch);
    }

    let references = profile_contexts
        .iter()
        .map(|(reference, _)| reference.clone())
        .collect::<Vec<_>>();
    let profiles = load_exact_profile_versions_batch(tx.transaction(), &references)
        .map_err(|_| PersistenceError::MemoryRowMismatch)?;
    for (reference, namespace) in profile_contexts {
        let profile = profiles
            .iter()
            .find(|profile| profile.reference() == reference)
            .ok_or(PersistenceError::MemoryRowMismatch)?;
        if profile.memory_namespace_id() != namespace {
            return Err(PersistenceError::MemoryRowMismatch);
        }
    }
    Ok(proposals)
}

fn validate_dependency_proposal_coherence(
    tx: &ImmediateTransaction<'_>,
    ids: &[String],
) -> Result<(), PersistenceError> {
    let ids_json = canonical_json_bytes(&ids.to_vec()).map_err(integrity)?;
    let mismatch: bool = tx
        .transaction()
        .query_row(
            "SELECT EXISTS(
             SELECT 1
               FROM json_each(CAST(?1 AS TEXT)) wanted
          LEFT JOIN memory_proposals p ON p.proposal_id=CAST(wanted.value AS TEXT)
          LEFT JOIN current_memory_proposal_status c ON c.proposal_id=p.proposal_id
          LEFT JOIN memory_proposal_resolutions r ON r.proposal_id=p.proposal_id
          LEFT JOIN approval_records a ON a.approval_id=p.approval_id
          LEFT JOIN event_stream creation_event
                 ON creation_event.sequence=p.creation_event_sequence
                AND creation_event.event_id=p.creation_event_id
          LEFT JOIN event_stream resolution_event
                 ON resolution_event.sequence=r.resolution_event_sequence
                AND resolution_event.event_id=r.resolution_event_id
          LEFT JOIN memory_entry_versions expected
                 ON expected.memory_namespace_id=p.memory_namespace_id
                AND expected.normalized_key=p.normalized_key
                AND expected.entry_id=p.expected_entry_id
                AND expected.entry_version_id=p.expected_entry_version_id
                AND expected.version=p.expected_entry_version
                AND expected.state=p.expected_kind
                AND expected.content_digest=p.expected_entry_digest
              WHERE p.proposal_id IS NULL OR c.proposal_id IS NULL
                 OR c.proposal_version IS NOT p.version
                 OR c.proposal_content_digest IS NOT p.content_digest
                 OR c.memory_namespace_id IS NOT p.memory_namespace_id
                 OR c.normalized_key IS NOT p.normalized_key
                 OR c.created_at_ms IS NOT p.created_at_ms
                 OR creation_event.sequence IS NULL
                 OR (p.expected_kind='absent' AND (p.expected_entry_id IS NOT NULL
                      OR p.expected_entry_version_id IS NOT NULL
                      OR p.expected_entry_version IS NOT NULL OR p.expected_entry_digest IS NOT NULL))
                 OR (p.expected_kind<>'absent' AND expected.entry_version_id IS NULL)
                 OR a.approval_id IS NULL OR a.approval_id IS NOT p.approval_id
                 OR a.action_kind IS NOT 'memory_mutation'
                 OR a.object_kind IS NOT 'memory_proposal'
                 OR a.object_id IS NOT p.proposal_id OR a.object_version IS NOT p.version
                 OR a.object_digest IS NOT p.content_digest OR a.actor_kind IS NOT 'agent'
                 OR a.actor_id IS NOT p.proposer_profile_id
                 OR a.created_at_ms IS NOT p.created_at_ms OR a.expires_at_ms IS NOT NULL
                 OR c.status IS NULL OR c.status NOT IN ('pending','accepted','rejected','expired')
                 OR (c.status='pending' AND (c.resolution_event_id IS NOT NULL
                      OR r.proposal_id IS NOT NULL OR a.status IS NOT 'pending'
                      OR a.resolved_at_ms IS NOT NULL OR a.resolution_kind IS NOT NULL
                      OR a.resolution_event_id IS NOT NULL OR a.resolution_actor_kind IS NOT NULL
                      OR a.resolution_actor_id IS NOT NULL))
                 OR (c.status<>'pending' AND (c.resolution_event_id IS NULL
                      OR r.proposal_id IS NULL OR r.status IS NOT c.status
                      OR r.proposal_version IS NOT p.version
                      OR r.proposal_content_digest IS NOT p.content_digest
                      OR r.approval_id IS NOT p.approval_id
                      OR r.resolved_by_kind IS NOT 'human' OR r.resolved_by_id IS NOT NULL
                      OR r.resolution_event_id IS NOT c.resolution_event_id
                      OR resolution_event.sequence IS NULL OR a.status IS NOT c.status
                      OR a.resolution_kind IS NOT c.status
                      OR a.resolution_event_id IS NOT c.resolution_event_id
                      OR a.resolved_at_ms IS NOT r.resolved_at_ms
                      OR a.resolution_actor_kind IS NOT 'human'
                      OR a.resolution_actor_id IS NOT NULL))
         )",
            [ids_json],
            |row| row.get(0),
        )
        .map_err(query)?;
    if mismatch {
        Err(PersistenceError::MemoryRowMismatch)
    } else {
        Ok(())
    }
}

fn load_current_pointer(
    tx: &ImmediateTransaction<'_>,
    namespace: MemoryNamespaceId,
    key: &NormalizedMemoryKey,
) -> Result<Option<MemoryEntryRef>, PersistenceError> {
    let mut statement = tx
        .transaction()
        .prepare(
            "SELECT c.memory_namespace_id,c.normalized_key,c.entry_id,c.entry_version_id,
                c.version,c.state,c.content_digest
           FROM current_memory_entries AS c
      LEFT JOIN memory_entry_versions AS v ON v.entry_version_id=c.entry_version_id
          WHERE (c.memory_namespace_id=?1 AND c.normalized_key=?2)
             OR (v.memory_namespace_id=?1 AND v.normalized_key=?2)
          ORDER BY c.entry_version_id",
        )
        .map_err(query)?;
    let rows = statement
        .query_map(params![namespace.to_string(), key.as_str()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
            ))
        })
        .map_err(query)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(query)?;
    match rows.as_slice() {
        [] => Ok(None),
        [(stored_namespace, stored_key, entry_id, version_id, version, state, digest)] => {
            let namespace: MemoryNamespaceId = parse_id(stored_namespace)?;
            let key = NormalizedMemoryKey::new(stored_key).map_err(integrity)?;
            let version = ObjectVersion::new(to_u64(*version)?).map_err(integrity)?;
            let digest = Digest::parse(digest).map_err(integrity)?;
            let state = parse_entry_state(state)?;
            let entry_id: crate::domain::MemoryEntryId = parse_id(entry_id)?;
            let version_id: MemoryEntryVersionId = parse_id(version_id)?;
            serde_json::from_value(serde_json::json!({"namespace_id":namespace,"entry_id":entry_id,"entry_version_id":version_id,"version":version,"normalized_key":key,"state":state,"content_digest":digest})).map(Some).map_err(|_|PersistenceError::MemoryRowMismatch)
        }
        _ => Err(PersistenceError::MemoryRowMismatch),
    }
}

fn insert_proposal_row(
    tx: &ImmediateTransaction<'_>,
    r: &ProposalRow,
) -> Result<(), PersistenceError> {
    tx.transaction().execute("INSERT INTO memory_proposals (proposal_id,version,proposer_profile_id,proposer_profile_version_id,proposer_profile_version,proposer_profile_digest,memory_namespace_id,operation,display_key,normalized_key,expected_kind,expected_entry_id,expected_entry_version_id,expected_entry_version,expected_entry_digest,candidate_value,candidate_value_bytes,candidate_purpose_tags_json,rationale,plaintext_validation_version,created_at_ms,creation_event_sequence,creation_event_id,approval_id,content_digest,record_digest,record_json) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27)",params![r.proposal_id,r.version,r.profile_id,r.profile_version_id,r.profile_version,r.profile_digest,r.namespace,r.operation,r.display_key,r.normalized_key,r.expected_kind,r.expected_id,r.expected_version_id,r.expected_version,r.expected_digest,r.candidate_value,r.candidate_value_bytes,r.candidate_tags,r.rationale,r.plaintext,r.created_at,r.creation_sequence,r.creation_event_id,r.approval_id,r.content_digest,r.record_digest,r.record_json]).map(|_|()).map_err(query)
}
fn load_proposal_rows_matching(
    tx: &ImmediateTransaction<'_>,
    p: &ProposalRow,
) -> Result<Vec<ProposalRow>, PersistenceError> {
    let mut s=tx.transaction().prepare("SELECT proposal_id,version,proposer_profile_id,proposer_profile_version_id,proposer_profile_version,proposer_profile_digest,memory_namespace_id,operation,display_key,normalized_key,expected_kind,expected_entry_id,expected_entry_version_id,expected_entry_version,expected_entry_digest,candidate_value,candidate_value_bytes,candidate_purpose_tags_json,rationale,plaintext_validation_version,created_at_ms,creation_event_sequence,creation_event_id,approval_id,content_digest,record_digest,record_json FROM memory_proposals WHERE proposal_id=?1 ORDER BY proposal_id").map_err(query)?;
    s.query_map([p.proposal_id.clone()], decode_proposal_stored_row)
        .map_err(query)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(query)
}
fn load_proposal_row_by_id(
    tx: &ImmediateTransaction<'_>,
    id: MemoryProposalId,
) -> Result<Option<ProposalRow>, PersistenceError> {
    tx.transaction().query_row("SELECT proposal_id,version,proposer_profile_id,proposer_profile_version_id,proposer_profile_version,proposer_profile_digest,memory_namespace_id,operation,display_key,normalized_key,expected_kind,expected_entry_id,expected_entry_version_id,expected_entry_version,expected_entry_digest,candidate_value,candidate_value_bytes,candidate_purpose_tags_json,rationale,plaintext_validation_version,created_at_ms,creation_event_sequence,creation_event_id,approval_id,content_digest,record_digest,record_json FROM memory_proposals WHERE proposal_id=?1",[id.to_string()],decode_proposal_stored_row).optional().map_err(query)
}
fn decode_proposal_stored_row(row: &Row<'_>) -> rusqlite::Result<ProposalRow> {
    Ok(ProposalRow {
        proposal_id: row.get(0)?,
        version: row.get(1)?,
        profile_id: row.get(2)?,
        profile_version_id: row.get(3)?,
        profile_version: row.get(4)?,
        profile_digest: row.get(5)?,
        namespace: row.get(6)?,
        operation: row.get(7)?,
        display_key: row.get(8)?,
        normalized_key: row.get(9)?,
        expected_kind: row.get(10)?,
        expected_id: row.get(11)?,
        expected_version_id: row.get(12)?,
        expected_version: row.get(13)?,
        expected_digest: row.get(14)?,
        candidate_value: row.get(15)?,
        candidate_value_bytes: row.get(16)?,
        candidate_tags: row.get(17)?,
        rationale: row.get(18)?,
        plaintext: row.get(19)?,
        created_at: row.get(20)?,
        creation_sequence: row.get(21)?,
        creation_event_id: row.get(22)?,
        approval_id: row.get(23)?,
        content_digest: row.get(24)?,
        record_digest: row.get(25)?,
        record_json: row.get(26)?,
    })
}
fn decode_proposal_row(
    tx: &ImmediateTransaction<'_>,
    r: ProposalRow,
) -> Result<MemoryProposal, PersistenceError> {
    let sequence = to_u64(r.creation_sequence)?;
    let p = decode_proposal_row_value(r)?;
    validate_event(tx, sequence, p.creation_event_id())?;
    Ok(p)
}

fn decode_proposal_row_value(r: ProposalRow) -> Result<MemoryProposal, PersistenceError> {
    let p: MemoryProposal =
        serde_json::from_slice(&r.record_json).map_err(|_| PersistenceError::MemoryRowMismatch)?;
    let expected = proposal_row(&p, to_u64(r.creation_sequence)?)?;
    if expected != r {
        return Err(PersistenceError::MemoryRowMismatch);
    };
    Ok(p)
}

fn insert_proposal_status(
    tx: &ImmediateTransaction<'_>,
    p: &MemoryProposal,
    status: MemoryProposalStatus,
    event: Option<EventId>,
) -> Result<(), PersistenceError> {
    tx.transaction().execute("INSERT INTO current_memory_proposal_status (proposal_id,proposal_version,proposal_content_digest,memory_namespace_id,normalized_key,status,resolution_event_id,created_at_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(proposal_id) DO UPDATE SET proposal_version=excluded.proposal_version,proposal_content_digest=excluded.proposal_content_digest,memory_namespace_id=excluded.memory_namespace_id,normalized_key=excluded.normalized_key,status=excluded.status,resolution_event_id=excluded.resolution_event_id,created_at_ms=excluded.created_at_ms",params![p.reference().proposal_id().to_string(),to_i64(p.reference().version().get())?,p.reference().content_digest().as_str(),p.namespace_id().to_string(),p.normalized_key().as_str(),proposal_status(status),event.map(|x|x.to_string()),p.created_at_ms()]).map(|_|()).map_err(query)
}
fn validate_proposal_status(
    tx: &ImmediateTransaction<'_>,
    p: &MemoryProposal,
    status: MemoryProposalStatus,
    event: Option<EventId>,
) -> Result<(), PersistenceError> {
    let actual=tx.transaction().query_row("SELECT proposal_version,proposal_content_digest,memory_namespace_id,normalized_key,status,resolution_event_id,created_at_ms FROM current_memory_proposal_status WHERE proposal_id=?1",[p.reference().proposal_id().to_string()],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,Option<String>>(5)?,r.get::<_,i64>(6)?))).optional().map_err(query)?.ok_or(PersistenceError::MemoryRowMismatch)?;
    if actual
        != (
            to_i64(p.reference().version().get())?,
            p.reference().content_digest().as_str().to_owned(),
            p.namespace_id().to_string(),
            p.normalized_key().as_str().to_owned(),
            proposal_status(status).to_owned(),
            event.map(|x| x.to_string()),
            p.created_at_ms(),
        )
    {
        return Err(PersistenceError::MemoryRowMismatch);
    };
    Ok(())
}
fn load_proposal_status(
    tx: &ImmediateTransaction<'_>,
    p: &MemoryProposal,
) -> Result<(MemoryProposalStatus, Option<EventId>), PersistenceError> {
    let row=tx.transaction().query_row("SELECT status,resolution_event_id FROM current_memory_proposal_status WHERE proposal_id=?1",[p.reference().proposal_id().to_string()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?))).optional().map_err(query)?.ok_or(PersistenceError::MemoryRowMismatch)?;
    let status = parse_proposal_status(&row.0)?;
    let event = row.1.map(|x| parse_id(&x)).transpose()?;
    validate_proposal_status(tx, p, status, event)?;
    Ok((status, event))
}

fn insert_resolution_row(
    tx: &ImmediateTransaction<'_>,
    r: &ResolutionRow,
) -> Result<(), PersistenceError> {
    tx.transaction().execute("INSERT INTO memory_proposal_resolutions (proposal_id,proposal_version,proposal_content_digest,status,approval_id,resolved_by_kind,resolved_by_id,resolved_at_ms,resolution_event_sequence,resolution_event_id,resolution_json) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",params![r.proposal_id,r.proposal_version,r.proposal_digest,r.status,r.approval_id,r.resolved_by_kind,r.resolved_by_id,r.resolved_at,r.resolution_sequence,r.resolution_event_id,r.resolution_json]).map(|_|()).map_err(query)
}
fn load_resolution(
    tx: &ImmediateTransaction<'_>,
    id: MemoryProposalId,
) -> Result<Option<ResolutionRow>, PersistenceError> {
    tx.transaction().query_row("SELECT proposal_id,proposal_version,proposal_content_digest,status,approval_id,resolved_by_kind,resolved_by_id,resolved_at_ms,resolution_event_sequence,resolution_event_id,resolution_json FROM memory_proposal_resolutions WHERE proposal_id=?1",[id.to_string()],|r|Ok(ResolutionRow{proposal_id:r.get(0)?,proposal_version:r.get(1)?,proposal_digest:r.get(2)?,status:r.get(3)?,approval_id:r.get(4)?,resolved_by_kind:r.get(5)?,resolved_by_id:r.get(6)?,resolved_at:r.get(7)?,resolution_sequence:r.get(8)?,resolution_event_id:r.get(9)?,resolution_json:r.get(10)?})).optional().map_err(query)
}
fn decode_resolution_row(
    tx: &ImmediateTransaction<'_>,
    p: &MemoryProposal,
    r: ResolutionRow,
) -> Result<MemoryProposalResolution, PersistenceError> {
    let sequence = to_u64(r.resolution_sequence)?;
    let resolution = decode_resolution_row_value(p, r)?;
    validate_event(tx, sequence, resolution.resolution_event_id())?;
    Ok(resolution)
}

fn decode_resolution_row_value(
    p: &MemoryProposal,
    r: ResolutionRow,
) -> Result<MemoryProposalResolution, PersistenceError> {
    let resolution: MemoryProposalResolution = serde_json::from_slice(&r.resolution_json)
        .map_err(|_| PersistenceError::MemoryRowMismatch)?;
    let expected = resolution_row(&resolution, to_u64(r.resolution_sequence)?)?;
    if expected != r || resolution.proposal() != &p.reference() {
        return Err(PersistenceError::MemoryRowMismatch);
    };
    Ok(resolution)
}

fn insert_summary_row(
    tx: &ImmediateTransaction<'_>,
    r: &SummaryRow,
) -> Result<(), PersistenceError> {
    tx.transaction().execute("INSERT INTO episodic_summaries (summary_id,version,memory_namespace_id,profile_id,profile_version_id,profile_version,profile_content_digest,label,body,purpose_tags_json,source_count,plaintext_validation_version,created_at_ms,creation_event_sequence,creation_event_id,source_set_digest,content_digest,record_digest,record_json) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)",params![r.summary_id,r.version,r.namespace,r.profile_id,r.profile_version_id,r.profile_version,r.profile_digest,r.label,r.body,r.tags,r.source_count,r.plaintext,r.created_at,r.creation_sequence,r.creation_event_id,r.source_digest,r.content_digest,r.record_digest,r.record_json]).map(|_|()).map_err(query)
}
fn load_summary_row(
    tx: &ImmediateTransaction<'_>,
    id: EpisodicSummaryId,
) -> Result<Option<SummaryRow>, PersistenceError> {
    tx.transaction().query_row("SELECT summary_id,version,memory_namespace_id,profile_id,profile_version_id,profile_version,profile_content_digest,label,body,purpose_tags_json,source_count,plaintext_validation_version,created_at_ms,creation_event_sequence,creation_event_id,source_set_digest,content_digest,record_digest,record_json FROM episodic_summaries WHERE summary_id=?1",[id.to_string()],decode_summary_stored_row).optional().map_err(query)
}
fn decode_summary_stored_row(row: &Row<'_>) -> rusqlite::Result<SummaryRow> {
    Ok(SummaryRow {
        summary_id: row.get(0)?,
        version: row.get(1)?,
        namespace: row.get(2)?,
        profile_id: row.get(3)?,
        profile_version_id: row.get(4)?,
        profile_version: row.get(5)?,
        profile_digest: row.get(6)?,
        label: row.get(7)?,
        body: row.get(8)?,
        tags: row.get(9)?,
        source_count: row.get(10)?,
        plaintext: row.get(11)?,
        created_at: row.get(12)?,
        creation_sequence: row.get(13)?,
        creation_event_id: row.get(14)?,
        source_digest: row.get(15)?,
        content_digest: row.get(16)?,
        record_digest: row.get(17)?,
        record_json: row.get(18)?,
    })
}
fn decode_summary_row(
    tx: &ImmediateTransaction<'_>,
    r: SummaryRow,
) -> Result<EpisodicSummary, PersistenceError> {
    let summary = decode_summary_row_value(r)?;
    validate_summary_sources(tx, &summary)?;
    Ok(summary)
}

fn decode_summary_row_value(r: SummaryRow) -> Result<EpisodicSummary, PersistenceError> {
    let summary: EpisodicSummary =
        serde_json::from_slice(&r.record_json).map_err(|_| PersistenceError::MemoryRowMismatch)?;
    let expected = summary_row(&summary)?;
    if expected != r {
        return Err(PersistenceError::MemoryRowMismatch);
    };
    Ok(summary)
}
fn validate_summary_sources(
    tx: &ImmediateTransaction<'_>,
    summary: &EpisodicSummary,
) -> Result<(), PersistenceError> {
    let mut statement=tx.transaction().prepare("SELECT s.source_ordinal,s.event_sequence,s.event_id,s.event_type,s.event_digest,e.sequence,e.event_id,e.event_type,e.event_digest FROM episodic_summary_sources AS s LEFT JOIN event_stream AS e ON e.sequence=s.event_sequence AND e.event_id=s.event_id WHERE s.summary_id=?1 ORDER BY s.source_ordinal ASC").map_err(query)?;
    let rows = statement
        .query_map([summary.reference().summary_id().to_string()], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, Option<i64>>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, Option<String>>(8)?,
            ))
        })
        .map_err(query)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(query)?;
    if rows.len() != summary.sources().len() {
        return Err(PersistenceError::MemoryRowMismatch);
    };
    for (
        ordinal,
        (
            (
                stored_ordinal,
                sequence,
                event_id,
                event_type,
                event_digest,
                event_sequence,
                event_id_exact,
                event_type_exact,
                event_digest_exact,
            ),
            source,
        ),
    ) in rows.into_iter().zip(summary.sources()).enumerate()
    {
        if stored_ordinal != to_i64(ordinal as u64)?
            || to_u64(sequence)? != source.sequence()
            || event_id != source.event_id().to_string()
            || event_type != source.event_type()
            || event_digest != source.event_digest().as_str()
            || event_sequence != Some(sequence)
            || event_id_exact.as_deref() != Some(event_id.as_str())
            || event_type_exact.as_deref() != Some(event_type.as_str())
            || event_digest_exact.as_deref() != Some(event_digest.as_str())
            || to_u64(sequence)? >= summary.creation_event_sequence()
        {
            return Err(PersistenceError::MemoryRowMismatch);
        }
    }
    MemoryRepository::validate_summary_base_context(tx, summary)
}

fn approval_columns(
    record: &ApprovalRecord,
    resolution_event: Option<EventId>,
) -> Result<ApprovalColumns, PersistenceError> {
    let object = record.object();
    let (actor_kind, actor_id) = actor_columns(record.actor());
    let (status, resolved_at, resolution_kind, resolution_actor_kind, resolution_actor_id) =
        match record.resolution() {
            None => (
                approval_status(record.status()).to_owned(),
                None,
                None,
                None,
                None,
            ),
            Some(r) => {
                let (kind, id) = actor_columns(r.actor());
                (
                    approval_status(record.status()).to_owned(),
                    Some(r.resolved_at_millis()),
                    Some(approval_status(r.status()).to_owned()),
                    Some(kind),
                    id,
                )
            }
        };
    Ok((
        approval_action(record.action()).to_owned(),
        object.kind.to_owned(),
        object.id.to_owned(),
        to_i64(object.version.get())?,
        object.digest.as_str().to_owned(),
        actor_id,
        actor_kind,
        resolution_event.map(|x| x.to_string()),
        status,
        record.created_at_millis(),
        record.expires_at_millis(),
        resolved_at,
        resolution_kind,
        resolution_actor_kind,
        resolution_actor_id,
        record.approval_id().to_string(),
    ))
}
fn insert_approval(
    tx: &ImmediateTransaction<'_>,
    record: &ApprovalRecord,
    resolution_event: Option<EventId>,
) -> Result<bool, PersistenceError> {
    let c = approval_columns(record, resolution_event)?;
    let existing = load_approval(tx, record.approval_id())?;
    match existing{None=>tx.transaction().execute("INSERT INTO approval_records (approval_id,action_kind,object_kind,object_id,object_version,object_digest,actor_kind,actor_id,status,created_at_ms,expires_at_ms,resolved_at_ms,resolution_kind,resolution_event_id,resolution_actor_kind,resolution_actor_id) VALUES (?16,?1,?2,?3,?4,?5,?7,?6,?9,?10,?11,?12,?13,?8,?14,?15)",params![c.0,c.1,c.2,c.3,c.4,c.5,c.6,c.7,c.8,c.9,c.10,c.11,c.12,c.13,c.14,c.15]).map(|_|true).map_err(query),Some(stored) if approval_equal(&stored,record,resolution_event)=>Ok(false),_=>Err(PersistenceError::MemoryRowMismatch)}
}
fn update_approval_resolution(
    tx: &ImmediateTransaction<'_>,
    record: &ApprovalRecord,
    resolution: &MemoryProposalResolution,
) -> Result<(), PersistenceError> {
    let stored =
        load_approval(tx, record.approval_id())?.ok_or(PersistenceError::MemoryRowMismatch)?;
    if stored.record.status() != ApprovalStatus::Pending
        || !approval_identity_equal(&stored.record, record)
        || record.resolution().is_none()
    {
        return Err(PersistenceError::MemoryRowMismatch);
    };
    let c = approval_columns(record, Some(resolution.resolution_event_id()))?;
    tx.transaction().execute("UPDATE approval_records SET status=?1,resolved_at_ms=?2,resolution_kind=?3,resolution_event_id=?4,resolution_actor_kind=?5,resolution_actor_id=?6 WHERE approval_id=?7",params![c.8,c.11,c.12,c.7,c.13,c.14,c.15]).map(|_|()).map_err(query)
}
fn load_approval(
    tx: &ImmediateTransaction<'_>,
    id: ApprovalId,
) -> Result<Option<StoredApproval>, PersistenceError> {
    let row=tx.transaction().query_row("SELECT action_kind,object_kind,object_id,object_version,object_digest,actor_kind,actor_id,status,created_at_ms,expires_at_ms,resolved_at_ms,resolution_kind,resolution_event_id,resolution_actor_kind,resolution_actor_id FROM approval_records WHERE approval_id=?1",[id.to_string()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,i64>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?,r.get::<_,Option<String>>(6)?,r.get::<_,String>(7)?,r.get::<_,i64>(8)?,r.get::<_,Option<i64>>(9)?,r.get::<_,Option<i64>>(10)?,r.get::<_,Option<String>>(11)?,r.get::<_,Option<String>>(12)?,r.get::<_,Option<String>>(13)?,r.get::<_,Option<String>>(14)?))).optional().map_err(query)?;
    row.map(|(action,kind,object_id,version,digest,actor_kind,actor_id,status,created,expires,resolved,resolution_kind,resolution_event,resolution_actor_kind,resolution_actor_id)|{let actor=actor_from_columns(&actor_kind,actor_id)?;let resolution=match(resolved,resolution_kind,resolution_actor_kind,resolution_actor_id){(None,None,None,None)=>serde_json::Value::Null,(Some(ms),Some(status),Some(kind),id)=>serde_json::json!({"status":parse_approval_status(&status)?,"actor":actor_json(actor_from_columns(&kind,id)?),"resolved_at_millis":ms}),_=>return Err(PersistenceError::MemoryRowMismatch)};let object=serde_json::json!({"kind":kind,"id":object_id,"version":ObjectVersion::new(to_u64(version)?).map_err(integrity)?,"digest":Digest::parse(&digest).map_err(integrity)?});let value=serde_json::json!({"approval_id":id,"action":parse_approval_action(&action)?,"object":object,"actor":actor_json(actor),"status":parse_approval_status(&status)?,"created_at_millis":created,"expires_at_millis":expires,"resolution":resolution});let record=serde_json::from_value::<ApprovalRecord>(value).map_err(|_|PersistenceError::MemoryRowMismatch)?;Ok(StoredApproval{record,resolution_event_id:resolution_event.map(|x|parse_id(&x)).transpose()?})}).transpose()
}
fn approval_identity_equal(left: &ApprovalRecord, right: &ApprovalRecord) -> bool {
    left.approval_id() == right.approval_id()
        && left.action() == right.action()
        && left.object() == right.object()
        && left.actor() == right.actor()
        && left.created_at_millis() == right.created_at_millis()
        && left.expires_at_millis() == right.expires_at_millis()
}
fn approval_equal(
    stored: &StoredApproval,
    record: &ApprovalRecord,
    event: Option<EventId>,
) -> bool {
    stored.record == *record && stored.resolution_event_id == event
}
fn validate_memory_approval(
    p: &MemoryProposal,
    approval: &ApprovalRecord,
    resolution: Option<&MemoryProposalResolution>,
) -> Result<(), PersistenceError> {
    if approval.approval_id() != p.approval_id()
        || approval.action() != ApprovalAction::MemoryMutation
        || approval.object() != &p.object_ref().map_err(integrity)?
        || approval.actor() != &Actor::Agent(p.proposer().profile_id())
    {
        return Err(PersistenceError::MemoryRowMismatch);
    };
    match resolution {
        None => {
            if approval.status() != ApprovalStatus::Pending || approval.resolution().is_some() {
                Err(PersistenceError::MemoryRowMismatch)
            } else {
                Ok(())
            }
        }
        Some(r) => {
            let a = approval
                .resolution()
                .ok_or(PersistenceError::MemoryRowMismatch)?;
            if approval.status() != approval_status_to_policy(r.status())
                || a.status() != approval_status_to_policy(r.status())
                || a.actor() != r.resolved_by()
                || a.resolved_at_millis() != r.resolved_at_ms()
            {
                Err(PersistenceError::MemoryRowMismatch)
            } else {
                Ok(())
            }
        }
    }
}

fn validate_proposal_approval_binding(
    proposal: &MemoryProposal,
    approval: &ApprovalRecord,
) -> Result<(), PersistenceError> {
    if approval.approval_id() != proposal.approval_id()
        || approval.action() != ApprovalAction::MemoryMutation
        || approval.object() != &proposal.object_ref().map_err(integrity)?
        || approval.actor() != &Actor::Agent(proposal.proposer().profile_id())
    {
        Err(PersistenceError::MemoryRowMismatch)
    } else {
        Ok(())
    }
}

fn validate_proposal_coherence(
    tx: &ImmediateTransaction<'_>,
    proposal: &MemoryProposal,
    status: MemoryProposalStatus,
    status_event: Option<EventId>,
    resolution: Option<&MemoryProposalResolution>,
) -> Result<(), PersistenceError> {
    let approval =
        load_approval(tx, proposal.approval_id())?.ok_or(PersistenceError::MemoryRowMismatch)?;
    validate_proposal_approval_binding(proposal, &approval.record)?;
    match (status, status_event, resolution) {
        (MemoryProposalStatus::Pending, None, None)
            if approval.record.status() == ApprovalStatus::Pending
                && approval.record.resolution().is_none()
                && approval.resolution_event_id.is_none() =>
        {
            Ok(())
        }
        (
            MemoryProposalStatus::Accepted
            | MemoryProposalStatus::Rejected
            | MemoryProposalStatus::Expired,
            Some(event_id),
            Some(resolution),
        ) if status == resolution.status()
            && event_id == resolution.resolution_event_id()
            && approval.resolution_event_id == Some(event_id) =>
        {
            validate_memory_approval(proposal, &approval.record, Some(resolution))
        }
        _ => Err(PersistenceError::MemoryRowMismatch),
    }
}

fn stream_entries(
    tx: &ImmediateTransaction<'_>,
    namespace: MemoryNamespaceId,
    where_clause: &str,
    values: Vec<Value>,
    builder: &mut MemorySnapshotBuilder,
) -> Result<(), PersistenceError> {
    let sql = format!(
        "SELECT v.memory_namespace_id,v.entry_id,v.entry_version_id,v.version,v.predecessor_version_id,v.display_key,v.normalized_key,v.state,v.value_text,v.value_bytes,v.purpose_tags_json,v.created_by_kind,v.created_by_id,v.created_at_ms,v.accepted_proposal_id,v.accepted_proposal_version,v.accepted_proposal_digest,v.plaintext_validation_version,v.creation_event_sequence,v.creation_event_id,v.content_digest,v.record_digest,v.record_json,c.version,c.state,c.content_digest FROM current_memory_entries c JOIN memory_entry_versions v ON v.memory_namespace_id=c.memory_namespace_id AND v.normalized_key=c.normalized_key AND v.entry_id=c.entry_id AND v.entry_version_id=c.entry_version_id WHERE c.memory_namespace_id=?1 AND v.state='present' AND {where_clause} ORDER BY c.normalized_key ASC,v.version ASC,c.entry_id ASC,v.entry_version_id ASC"
    );
    let mut parameters = Vec::with_capacity(values.len() + 1);
    parameters.push(Value::Text(namespace.to_string()));
    parameters.extend(values);
    let mut statement = tx.transaction().prepare(&sql).map_err(query)?;
    let mut rows = statement
        .query(params_from_iter(parameters))
        .map_err(query)?;
    while let Some(row) = rows.next().map_err(query)? {
        let stored = decode_entry_row(row).map_err(query)?;
        let pointer_version = row.get::<_, i64>(23).map_err(query)?;
        let pointer_state = row.get::<_, String>(24).map_err(query)?;
        let pointer_digest = row.get::<_, String>(25).map_err(query)?;
        let entry = decode_entry_row_value(stored)?;
        validate_current_pointer_columns(&entry, pointer_version, &pointer_state, &pointer_digest)?;
        builder
            .consider_entry(MemoryKvContextItem::from_entry(&entry).map_err(integrity)?)
            .map_err(integrity)?;
    }
    Ok(())
}
fn stream_summaries(
    tx: &ImmediateTransaction<'_>,
    namespace: MemoryNamespaceId,
    profile: &AgentProfileVersionRef,
    where_clause: &str,
    values: Vec<Value>,
    builder: &mut MemorySnapshotBuilder,
) -> Result<(), PersistenceError> {
    let sql = format!(
        "SELECT summary_id,version,memory_namespace_id,profile_id,profile_version_id,profile_version,profile_content_digest,label,body,purpose_tags_json,source_count,plaintext_validation_version,created_at_ms,creation_event_sequence,creation_event_id,source_set_digest,content_digest,record_digest,record_json FROM episodic_summaries s WHERE json_extract(CAST(s.record_json AS TEXT),'$.namespace_id')=?1 AND json_extract(CAST(s.record_json AS TEXT),'$.profile.profile_id')=?2 AND json_extract(CAST(s.record_json AS TEXT),'$.profile.profile_version_id')=?3 AND json_extract(CAST(s.record_json AS TEXT),'$.profile.version')=?4 AND json_extract(CAST(s.record_json AS TEXT),'$.profile.content_digest')=?5 AND {where_clause} ORDER BY s.created_at_ms DESC,s.summary_id ASC"
    );
    let mut parameters = vec![
        Value::Text(namespace.to_string()),
        Value::Text(profile.profile_id().to_string()),
        Value::Text(profile.profile_version_id().to_string()),
        Value::Integer(to_i64(profile.version().get())?),
        Value::Text(profile.content_digest().as_str().to_owned()),
    ];
    parameters.extend(values);
    let mut statement = tx.transaction().prepare(&sql).map_err(query)?;
    let mut rows = statement
        .query(params_from_iter(parameters))
        .map_err(query)?;
    while let Some(row) = rows.next().map_err(query)? {
        let stored = decode_summary_stored_row(row).map_err(query)?;
        let summary = decode_summary_row_value(stored)?;
        builder
            .consider_summary(EpisodicContextItem::from_summary(&summary).map_err(integrity)?)
            .map_err(integrity)?;
    }
    Ok(())
}
fn tag_clause(column: &str, first_parameter: usize, len: usize) -> String {
    let marks = (first_parameter..len + first_parameter)
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "EXISTS (SELECT 1 FROM json_each(CAST({column} AS TEXT)) AS tag WHERE tag.value IN ({marks}))"
    )
}

fn validate_current_pointer_columns(
    entry: &MemoryEntryVersion,
    version: i64,
    state: &str,
    digest: &str,
) -> Result<(), PersistenceError> {
    let reference = entry.reference();
    if version != to_i64(reference.version().get())?
        || state != entry_state(reference.state())
        || digest != reference.content_digest().as_str()
    {
        Err(PersistenceError::MemoryRowMismatch)
    } else {
        Ok(())
    }
}

/// Streams and authenticates every current immutable row in either the pointer
/// or immutable namespace.  This deliberately runs before any state/tag SQL
/// filter so a forged mirror cannot turn a corrupt row into an omission.
fn validate_current_entry_rows(
    tx: &ImmediateTransaction<'_>,
    namespace: MemoryNamespaceId,
) -> Result<(), PersistenceError> {
    validate_current_entry_pointers(tx, namespace)?;
    let mut statement = tx
        .transaction()
        .prepare(
            "SELECT v.memory_namespace_id,v.entry_id,v.entry_version_id,v.version,
                v.predecessor_version_id,v.display_key,v.normalized_key,v.state,v.value_text,
                v.value_bytes,v.purpose_tags_json,v.created_by_kind,v.created_by_id,v.created_at_ms,
                v.accepted_proposal_id,v.accepted_proposal_version,v.accepted_proposal_digest,
                v.plaintext_validation_version,v.creation_event_sequence,v.creation_event_id,
                v.content_digest,v.record_digest,v.record_json,
                c.memory_namespace_id,c.normalized_key,c.entry_id,c.entry_version_id,
                c.version,c.state,c.content_digest,
                EXISTS(SELECT 1 FROM event_stream e
                        WHERE e.sequence=v.creation_event_sequence
                          AND e.event_id=v.creation_event_id),
                CASE WHEN v.predecessor_version_id IS NULL THEN v.version=1 ELSE EXISTS(
                    SELECT 1 FROM memory_entry_versions predecessor
                     WHERE predecessor.entry_version_id=v.predecessor_version_id
                       AND predecessor.memory_namespace_id=v.memory_namespace_id
                       AND predecessor.normalized_key=v.normalized_key
                       AND predecessor.entry_id=v.entry_id
                       AND predecessor.version=v.version-1) END,
                CASE WHEN v.accepted_proposal_id IS NULL THEN 1 ELSE EXISTS(
                    SELECT 1 FROM memory_proposals p
                    JOIN current_memory_proposal_status ps ON ps.proposal_id=p.proposal_id
                     WHERE p.proposal_id=v.accepted_proposal_id
                       AND p.version=v.accepted_proposal_version
                       AND p.content_digest=v.accepted_proposal_digest
                       AND ps.status='accepted') END
           FROM current_memory_entries AS c
      LEFT JOIN memory_entry_versions AS v ON v.entry_version_id=c.entry_version_id
          WHERE c.memory_namespace_id=?1 OR v.memory_namespace_id=?1
          ORDER BY c.entry_version_id",
        )
        .map_err(query)?;
    let mut rows = statement.query([namespace.to_string()]).map_err(query)?;
    let mut entries = Vec::new();
    while let Some(row) = rows.next().map_err(query)? {
        if row.get::<_, Option<String>>(2).map_err(query)?.is_none() {
            return Err(PersistenceError::MemoryRowMismatch);
        }
        let entry = decode_entry_row_value(decode_entry_row(row).map_err(query)?)?;
        let reference = entry.reference();
        if row.get::<_, String>(23).map_err(query)? != reference.namespace_id().to_string()
            || row.get::<_, String>(24).map_err(query)? != reference.normalized_key().as_str()
            || row.get::<_, String>(25).map_err(query)? != reference.entry_id().to_string()
            || row.get::<_, String>(26).map_err(query)? != reference.entry_version_id().to_string()
            || row.get::<_, i64>(27).map_err(query)? != to_i64(reference.version().get())?
            || row.get::<_, String>(28).map_err(query)? != entry_state(reference.state())
            || row.get::<_, String>(29).map_err(query)? != reference.content_digest().as_str()
            || !row.get::<_, bool>(30).map_err(query)?
            || !row.get::<_, bool>(31).map_err(query)?
            || !row.get::<_, bool>(32).map_err(query)?
        {
            return Err(PersistenceError::MemoryRowMismatch);
        }
        entries.push(entry);
    }
    drop(rows);
    drop(statement);
    if !entries.is_empty() {
        validate_entry_dependency_graph(tx, &entries)?;
    }
    Ok(())
}

/// Authenticate pointer rows before an immutable-row join can turn a corrupt
/// pointer identity into a silently omitted current record.  The join only uses
/// the immutable version id to resolve the target; every mirrored pointer field
/// is then compared explicitly.
fn validate_current_entry_pointers(
    tx: &ImmediateTransaction<'_>,
    namespace: MemoryNamespaceId,
) -> Result<(), PersistenceError> {
    let mismatch: bool = tx
        .transaction()
        .query_row(
            "SELECT EXISTS(
                 SELECT 1
                   FROM current_memory_entries AS c
              LEFT JOIN memory_entry_versions AS v
                     ON v.entry_version_id=c.entry_version_id
                  WHERE (c.memory_namespace_id=?1 OR v.memory_namespace_id=?1)
                    AND (v.entry_version_id IS NULL
                      OR v.memory_namespace_id<>c.memory_namespace_id
                      OR v.normalized_key<>c.normalized_key
                      OR v.entry_id<>c.entry_id
                      OR v.version<>c.version
                      OR v.state<>c.state
                      OR v.content_digest<>c.content_digest)
             )",
            [namespace.to_string()],
            |row| row.get(0),
        )
        .map_err(query)?;
    if mismatch {
        Err(PersistenceError::MemoryRowMismatch)
    } else {
        Ok(())
    }
}

/// Authenticate every current proposal status in the requested namespace before
/// a pending/all SQL filter is allowed to suppress a malformed status row.
fn validate_proposal_status_rows(
    tx: &ImmediateTransaction<'_>,
    namespace: MemoryNamespaceId,
) -> Result<(), PersistenceError> {
    let mismatch: bool = tx
        .transaction()
        .query_row(
            "WITH scoped(proposal_id) AS (
                 SELECT c.proposal_id
                   FROM current_memory_proposal_status c
              LEFT JOIN memory_proposals p ON p.proposal_id=c.proposal_id
                  WHERE c.memory_namespace_id=?1 OR p.memory_namespace_id=?1
                 UNION
                 SELECT p.proposal_id
                   FROM memory_proposals p
              LEFT JOIN current_memory_proposal_status c ON c.proposal_id=p.proposal_id
                  WHERE p.memory_namespace_id=?1 OR c.memory_namespace_id=?1
             )
             SELECT EXISTS(
             SELECT 1 FROM scoped o
          LEFT JOIN current_memory_proposal_status AS c ON c.proposal_id=o.proposal_id
          LEFT JOIN memory_proposals AS p ON p.proposal_id=o.proposal_id
          LEFT JOIN memory_proposal_resolutions AS r ON r.proposal_id=c.proposal_id
          LEFT JOIN approval_records AS a ON a.approval_id=p.approval_id
          LEFT JOIN event_stream AS creation_event
                 ON creation_event.sequence=p.creation_event_sequence
                AND creation_event.event_id=p.creation_event_id
          LEFT JOIN event_stream AS resolution_event
                 ON resolution_event.sequence=r.resolution_event_sequence
                AND resolution_event.event_id=r.resolution_event_id
          LEFT JOIN memory_entry_versions expected
                 ON expected.memory_namespace_id=p.memory_namespace_id
                AND expected.normalized_key=p.normalized_key
                AND expected.entry_id=p.expected_entry_id
                AND expected.entry_version_id=p.expected_entry_version_id
                AND expected.version=p.expected_entry_version
                AND expected.state=p.expected_kind
                AND expected.content_digest=p.expected_entry_digest
              WHERE c.proposal_id IS NULL OR p.proposal_id IS NULL
                  OR c.proposal_version<>p.version
                  OR c.proposal_content_digest<>p.content_digest
                  OR c.memory_namespace_id<>p.memory_namespace_id
                  OR c.normalized_key<>p.normalized_key
                  OR c.created_at_ms<>p.created_at_ms
                  OR creation_event.sequence IS NULL
                  OR (p.expected_kind='absent' AND (p.expected_entry_id IS NOT NULL
                       OR p.expected_entry_version_id IS NOT NULL
                       OR p.expected_entry_version IS NOT NULL OR p.expected_entry_digest IS NOT NULL))
                  OR (p.expected_kind<>'absent' AND expected.entry_version_id IS NULL)
                  OR a.approval_id IS NULL OR a.approval_id<>p.approval_id
                  OR a.action_kind<>'memory_mutation' OR a.object_kind<>'memory_proposal'
                  OR a.object_id<>p.proposal_id OR a.object_version<>p.version
                  OR a.object_digest<>p.content_digest OR a.actor_kind<>'agent'
                  OR a.actor_id IS NOT p.proposer_profile_id OR a.created_at_ms<>p.created_at_ms
                  OR a.expires_at_ms IS NOT NULL
                  OR c.status NOT IN ('pending','accepted','rejected','expired')
                  OR (c.status='pending' AND (c.resolution_event_id IS NOT NULL
                       OR r.proposal_id IS NOT NULL OR a.status<>'pending'
                       OR a.resolved_at_ms IS NOT NULL OR a.resolution_kind IS NOT NULL
                       OR a.resolution_event_id IS NOT NULL OR a.resolution_actor_kind IS NOT NULL
                       OR a.resolution_actor_id IS NOT NULL))
                  OR (c.status<>'pending' AND (c.resolution_event_id IS NULL
                       OR r.proposal_id IS NULL OR r.status<>c.status
                       OR r.proposal_version<>p.version
                       OR r.proposal_content_digest<>p.content_digest
                       OR r.approval_id<>p.approval_id
                       OR r.resolved_by_kind<>'human' OR r.resolved_by_id IS NOT NULL
                       OR r.resolution_event_id<>c.resolution_event_id
                       OR resolution_event.sequence IS NULL
                       OR a.status<>c.status OR a.resolution_kind<>c.status
                       OR a.resolution_event_id IS NOT c.resolution_event_id
                       OR a.resolved_at_ms<>r.resolved_at_ms
                       OR a.resolution_actor_kind IS NOT 'human'
                       OR a.resolution_actor_id IS NOT NULL))
           )",
            [namespace.to_string()],
            |row| row.get(0),
        )
        .map_err(query)?;
    if mismatch {
        Err(PersistenceError::MemoryRowMismatch)
    } else {
        Ok(())
    }
}

fn validate_proposal_rows(
    tx: &ImmediateTransaction<'_>,
    namespace: MemoryNamespaceId,
) -> Result<(), PersistenceError> {
    validate_proposal_status_rows(tx, namespace)?;
    let mut statement = tx
        .transaction()
        .prepare(
            "SELECT p.proposal_id,p.version,p.proposer_profile_id,p.proposer_profile_version_id,
                p.proposer_profile_version,p.proposer_profile_digest,p.memory_namespace_id,
                p.operation,p.display_key,p.normalized_key,p.expected_kind,p.expected_entry_id,
                p.expected_entry_version_id,p.expected_entry_version,p.expected_entry_digest,
                p.candidate_value,p.candidate_value_bytes,p.candidate_purpose_tags_json,
                p.rationale,p.plaintext_validation_version,p.created_at_ms,
                p.creation_event_sequence,p.creation_event_id,p.approval_id,p.content_digest,
                p.record_digest,p.record_json,
                r.proposal_id,r.proposal_version,r.proposal_content_digest,r.status,r.approval_id,
                r.resolved_by_kind,r.resolved_by_id,r.resolved_at_ms,r.resolution_event_sequence,
                r.resolution_event_id,r.resolution_json,c.status,c.resolution_event_id
           FROM memory_proposals p
      LEFT JOIN current_memory_proposal_status c ON c.proposal_id=p.proposal_id
      LEFT JOIN memory_proposal_resolutions r ON r.proposal_id=p.proposal_id
          WHERE p.memory_namespace_id=?1 OR c.memory_namespace_id=?1
          ORDER BY p.proposal_id",
        )
        .map_err(query)?;
    let mut rows = statement.query([namespace.to_string()]).map_err(query)?;
    let mut profile_contexts = Vec::new();
    let mut expected_entries = Vec::new();
    while let Some(row) = rows.next().map_err(query)? {
        let proposal = decode_proposal_row_value(decode_proposal_stored_row(row).map_err(query)?)?;
        profile_contexts.push((proposal.proposer().clone(), proposal.namespace_id()));
        if let ExpectedMemoryEntryState::Present(reference)
        | ExpectedMemoryEntryState::Deleted(reference) = proposal.expected()
        {
            expected_entries.push(reference.clone());
        }
        let status = parse_proposal_status(&row.get::<_, String>(38).map_err(query)?)?;
        let status_event = row
            .get::<_, Option<String>>(39)
            .map_err(query)?
            .map(|value| parse_id(&value))
            .transpose()?;
        let resolution_id = row.get::<_, Option<String>>(27).map_err(query)?;
        match (status, resolution_id) {
            (MemoryProposalStatus::Pending, None) if status_event.is_none() => {}
            (MemoryProposalStatus::Pending, _) => {
                return Err(PersistenceError::MemoryRowMismatch);
            }
            (_, Some(proposal_id)) => {
                let stored = ResolutionRow {
                    proposal_id,
                    proposal_version: row.get(28).map_err(query)?,
                    proposal_digest: row.get(29).map_err(query)?,
                    status: row.get(30).map_err(query)?,
                    approval_id: row.get(31).map_err(query)?,
                    resolved_by_kind: row.get(32).map_err(query)?,
                    resolved_by_id: row.get(33).map_err(query)?,
                    resolved_at: row.get(34).map_err(query)?,
                    resolution_sequence: row.get(35).map_err(query)?,
                    resolution_event_id: row.get(36).map_err(query)?,
                    resolution_json: row.get(37).map_err(query)?,
                };
                let resolution = decode_resolution_row_value(&proposal, stored)?;
                if resolution.status() != status
                    || Some(resolution.resolution_event_id()) != status_event
                {
                    return Err(PersistenceError::MemoryRowMismatch);
                }
            }
            _ => return Err(PersistenceError::MemoryRowMismatch),
        }
    }
    drop(rows);
    drop(statement);

    let references = profile_contexts
        .iter()
        .map(|(reference, _)| reference.clone())
        .collect::<Vec<_>>();
    let profiles = load_exact_profile_versions_batch(tx.transaction(), &references)
        .map_err(|_| PersistenceError::MemoryRowMismatch)?;
    for (reference, proposal_namespace) in profile_contexts {
        let profile = profiles
            .iter()
            .find(|profile| profile.reference() == reference)
            .ok_or(PersistenceError::MemoryRowMismatch)?;
        if profile.memory_namespace_id() != proposal_namespace {
            return Err(PersistenceError::MemoryRowMismatch);
        }
    }
    validate_expected_entry_refs(tx, &expected_entries)
}

fn validate_expected_entry_refs(
    tx: &ImmediateTransaction<'_>,
    references: &[MemoryEntryRef],
) -> Result<(), PersistenceError> {
    let ids = references
        .iter()
        .map(|reference| reference.entry_version_id().to_string())
        .collect::<Vec<_>>();
    let ids_json = canonical_json_bytes(&ids).map_err(integrity)?;
    let mut statement = tx
        .transaction()
        .prepare(
            "SELECT memory_namespace_id,entry_id,entry_version_id,version,predecessor_version_id,
                display_key,normalized_key,state,value_text,value_bytes,purpose_tags_json,
                created_by_kind,created_by_id,created_at_ms,accepted_proposal_id,
                accepted_proposal_version,accepted_proposal_digest,plaintext_validation_version,
                creation_event_sequence,creation_event_id,content_digest,record_digest,record_json
           FROM memory_entry_versions
          WHERE entry_version_id IN (SELECT value FROM json_each(CAST(?1 AS TEXT)))
          ORDER BY entry_version_id",
        )
        .map_err(query)?;
    let rows = statement
        .query_map([ids_json], decode_entry_row)
        .map_err(query)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(query)?;
    let authenticated = rows
        .into_iter()
        .map(decode_entry_row_value)
        .collect::<Result<Vec<_>, _>>()?;
    if !references.iter().all(|reference| {
        authenticated
            .iter()
            .any(|entry| entry.reference() == *reference)
    }) {
        return Err(PersistenceError::MemoryRowMismatch);
    }
    if !authenticated.is_empty() {
        validate_entry_dependency_graph(tx, &authenticated)?;
    }
    Ok(())
}

fn validate_summary_rows(
    tx: &ImmediateTransaction<'_>,
    namespace: MemoryNamespaceId,
) -> Result<(), PersistenceError> {
    let mut statement = tx
        .transaction()
        .prepare(
            "SELECT summary_id,version,memory_namespace_id,profile_id,profile_version_id,
                profile_version,profile_content_digest,label,body,purpose_tags_json,
                source_count,plaintext_validation_version,created_at_ms,creation_event_sequence,
                creation_event_id,source_set_digest,content_digest,record_digest,record_json
           FROM episodic_summaries
          WHERE memory_namespace_id=?1
             OR json_extract(CAST(record_json AS TEXT),'$.namespace_id')=?1
          ORDER BY summary_id",
        )
        .map_err(query)?;
    let mut rows = statement.query([namespace.to_string()]).map_err(query)?;
    let mut profile_contexts = Vec::new();
    while let Some(row) = rows.next().map_err(query)? {
        let summary = decode_summary_row_value(decode_summary_stored_row(row).map_err(query)?)?;
        profile_contexts.push((
            summary.reference().profile().clone(),
            summary.reference().namespace_id(),
        ));
    }
    drop(rows);
    drop(statement);

    let mismatch: bool = tx
        .transaction()
        .query_row(
            "WITH scoped AS (
             SELECT * FROM episodic_summaries
              WHERE memory_namespace_id=?1
                 OR json_extract(CAST(record_json AS TEXT),'$.namespace_id')=?1
         )
         SELECT EXISTS(
             SELECT 1 FROM scoped s
              WHERE NOT EXISTS(
                    SELECT 1 FROM event_stream creation_event
                     WHERE creation_event.sequence=s.creation_event_sequence
                       AND creation_event.event_id=s.creation_event_id)
                 OR (SELECT COUNT(*) FROM episodic_summary_sources source
                      WHERE source.summary_id=s.summary_id)<>s.source_count
                 OR EXISTS(
                    SELECT 1
                      FROM json_each(
                           json_extract(CAST(s.record_json AS TEXT),'$.sources')) canonical
                 LEFT JOIN episodic_summary_sources source
                        ON source.summary_id=s.summary_id
                       AND source.source_ordinal=canonical.key
                 LEFT JOIN event_stream event
                        ON event.sequence=source.event_sequence
                       AND event.event_id=source.event_id
                       AND event.event_type=source.event_type
                       AND event.event_digest=source.event_digest
                     WHERE source.summary_id IS NULL
                        OR source.event_sequence<>json_extract(canonical.value,'$.sequence')
                        OR source.event_id<>json_extract(canonical.value,'$.event_id')
                        OR source.event_type<>json_extract(canonical.value,'$.event_type')
                        OR source.event_digest<>json_extract(canonical.value,'$.event_digest')
                        OR event.sequence IS NULL
                        OR source.event_sequence>=s.creation_event_sequence)
         )",
            [namespace.to_string()],
            |row| row.get(0),
        )
        .map_err(query)?;
    if mismatch {
        return Err(PersistenceError::MemoryRowMismatch);
    }

    let references = profile_contexts
        .iter()
        .map(|(reference, _)| reference.clone())
        .collect::<Vec<_>>();
    let profiles = load_exact_profile_versions_batch(tx.transaction(), &references)
        .map_err(|_| PersistenceError::MemoryRowMismatch)?;
    for (reference, summary_namespace) in profile_contexts {
        let profile = profiles
            .iter()
            .find(|profile| profile.reference() == reference)
            .ok_or(PersistenceError::MemoryRowMismatch)?;
        if profile.memory_namespace_id() != summary_namespace {
            return Err(PersistenceError::MemoryRowMismatch);
        }
    }
    Ok(())
}

fn load_exact_profile(
    tx: &ImmediateTransaction<'_>,
    reference: &AgentProfileVersionRef,
) -> Result<AgentProfileVersion, PersistenceError> {
    load_exact_profile_version(tx.transaction(), reference)?
        .ok_or(PersistenceError::MemoryRowMismatch)
}
fn validate_event(
    tx: &ImmediateTransaction<'_>,
    sequence: u64,
    id: EventId,
) -> Result<(), PersistenceError> {
    let found: bool = tx
        .transaction()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM event_stream WHERE sequence=?1 AND event_id=?2)",
            params![to_i64(sequence)?, id.to_string()],
            |r| r.get(0),
        )
        .map_err(query)?;
    if found {
        Ok(())
    } else {
        Err(PersistenceError::MemoryRowMismatch)
    }
}
fn checked_count<P: rusqlite::Params>(
    tx: &ImmediateTransaction<'_>,
    sql: &str,
    parameters: P,
) -> Result<u64, PersistenceError> {
    to_u64(
        tx.transaction()
            .query_row(sql, parameters, |r| r.get::<_, i64>(0))
            .map_err(query)?,
    )
}
fn page_limit(limit: u16) -> u16 {
    limit.min(PAGE_LIMIT)
}
fn page_history(
    versions: Vec<MemoryEntryVersion>,
    total_count: u64,
) -> Result<MemoryEntryHistoryPage, PersistenceError> {
    let returned_count =
        u64::try_from(versions.len()).map_err(|_| PersistenceError::MemoryRowMismatch)?;
    let omitted_count = total_count
        .checked_sub(returned_count)
        .ok_or(PersistenceError::MemoryRowMismatch)?;
    Ok(MemoryEntryHistoryPage {
        versions,
        total_count,
        returned_count,
        omitted_count,
    })
}
fn page_entries(
    records: Vec<MemoryEntryListRecord>,
    total_count: u64,
) -> Result<MemoryEntriesPage, PersistenceError> {
    let returned_count =
        u64::try_from(records.len()).map_err(|_| PersistenceError::MemoryRowMismatch)?;
    let omitted_count = total_count
        .checked_sub(returned_count)
        .ok_or(PersistenceError::MemoryRowMismatch)?;
    Ok(MemoryEntriesPage {
        records,
        total_count,
        returned_count,
        omitted_count,
    })
}
fn page_proposals(
    records: Vec<MemoryProposalListRecord>,
    total_count: u64,
) -> Result<MemoryProposalsPage, PersistenceError> {
    let returned_count =
        u64::try_from(records.len()).map_err(|_| PersistenceError::MemoryRowMismatch)?;
    let omitted_count = total_count
        .checked_sub(returned_count)
        .ok_or(PersistenceError::MemoryRowMismatch)?;
    Ok(MemoryProposalsPage {
        records,
        total_count,
        returned_count,
        omitted_count,
    })
}
fn page_summaries(
    records: Vec<EpisodicSummaryListRecord>,
    total_count: u64,
) -> Result<EpisodicSummariesPage, PersistenceError> {
    let returned_count =
        u64::try_from(records.len()).map_err(|_| PersistenceError::MemoryRowMismatch)?;
    let omitted_count = total_count
        .checked_sub(returned_count)
        .ok_or(PersistenceError::MemoryRowMismatch)?;
    Ok(EpisodicSummariesPage {
        records,
        total_count,
        returned_count,
        omitted_count,
    })
}
fn to_i64(value: u64) -> Result<i64, PersistenceError> {
    i64::try_from(value).map_err(|_| PersistenceError::MemoryRowMismatch)
}
fn to_u64(value: i64) -> Result<u64, PersistenceError> {
    u64::try_from(value).map_err(|_| PersistenceError::MemoryRowMismatch)
}
fn parse_id<T: FromStr>(value: &str) -> Result<T, PersistenceError> {
    value
        .parse()
        .map_err(|_| PersistenceError::MemoryRowMismatch)
}
fn query(error: rusqlite::Error) -> PersistenceError {
    super::database::persistence_error(error)
}
fn integrity(_: impl std::fmt::Display) -> PersistenceError {
    PersistenceError::MemoryRowMismatch
}
fn entry_state(state: MemoryEntryState) -> &'static str {
    match state {
        MemoryEntryState::Present => "present",
        MemoryEntryState::Deleted => "deleted",
    }
}
fn parse_entry_state(value: &str) -> Result<MemoryEntryState, PersistenceError> {
    match value {
        "present" => Ok(MemoryEntryState::Present),
        "deleted" => Ok(MemoryEntryState::Deleted),
        _ => Err(PersistenceError::MemoryRowMismatch),
    }
}
fn proposal_status(status: MemoryProposalStatus) -> &'static str {
    match status {
        MemoryProposalStatus::Pending => "pending",
        MemoryProposalStatus::Accepted => "accepted",
        MemoryProposalStatus::Rejected => "rejected",
        MemoryProposalStatus::Expired => "expired",
    }
}
fn parse_proposal_status(value: &str) -> Result<MemoryProposalStatus, PersistenceError> {
    match value {
        "pending" => Ok(MemoryProposalStatus::Pending),
        "accepted" => Ok(MemoryProposalStatus::Accepted),
        "rejected" => Ok(MemoryProposalStatus::Rejected),
        "expired" => Ok(MemoryProposalStatus::Expired),
        _ => Err(PersistenceError::MemoryRowMismatch),
    }
}
fn proposal_filter(filter: MemoryProposalFilter) -> &'static str {
    match filter {
        MemoryProposalFilter::Pending => "pending",
        MemoryProposalFilter::All => "all",
    }
}
fn approval_action(action: ApprovalAction) -> &'static str {
    match action {
        ApprovalAction::DiscussionRun => "discussion_run",
        ApprovalAction::McpUse => "mcp_use",
        ApprovalAction::EngineeringJobRun => "engineering_job_run",
        ApprovalAction::GitMerge => "git_merge",
        ApprovalAction::GitPush => "git_push",
        ApprovalAction::FinanceRecommendation => "finance_recommendation",
        ApprovalAction::MemoryMutation => "memory_mutation",
    }
}
fn parse_approval_action(value: &str) -> Result<ApprovalAction, PersistenceError> {
    match value {
        "discussion_run" => Ok(ApprovalAction::DiscussionRun),
        "mcp_use" => Ok(ApprovalAction::McpUse),
        "engineering_job_run" => Ok(ApprovalAction::EngineeringJobRun),
        "git_merge" => Ok(ApprovalAction::GitMerge),
        "git_push" => Ok(ApprovalAction::GitPush),
        "finance_recommendation" => Ok(ApprovalAction::FinanceRecommendation),
        "memory_mutation" => Ok(ApprovalAction::MemoryMutation),
        _ => Err(PersistenceError::MemoryRowMismatch),
    }
}
fn approval_status(status: ApprovalStatus) -> &'static str {
    match status {
        ApprovalStatus::Pending => "pending",
        ApprovalStatus::Accepted => "accepted",
        ApprovalStatus::Rejected => "rejected",
        ApprovalStatus::Expired => "expired",
        ApprovalStatus::Cancelled => "cancelled",
    }
}
fn parse_approval_status(value: &str) -> Result<ApprovalStatus, PersistenceError> {
    match value {
        "pending" => Ok(ApprovalStatus::Pending),
        "accepted" => Ok(ApprovalStatus::Accepted),
        "rejected" => Ok(ApprovalStatus::Rejected),
        "expired" => Ok(ApprovalStatus::Expired),
        "cancelled" => Ok(ApprovalStatus::Cancelled),
        _ => Err(PersistenceError::MemoryRowMismatch),
    }
}
fn approval_status_to_policy(status: MemoryProposalStatus) -> ApprovalStatus {
    match status {
        MemoryProposalStatus::Pending => ApprovalStatus::Pending,
        MemoryProposalStatus::Accepted => ApprovalStatus::Accepted,
        MemoryProposalStatus::Rejected => ApprovalStatus::Rejected,
        MemoryProposalStatus::Expired => ApprovalStatus::Expired,
    }
}
fn actor_columns(actor: &Actor) -> (String, Option<String>) {
    match actor {
        Actor::Human => ("human".to_owned(), None),
        Actor::System => ("system".to_owned(), None),
        Actor::Agent(id) => ("agent".to_owned(), Some(id.to_string())),
    }
}
fn actor_from_columns(kind: &str, id: Option<String>) -> Result<Actor, PersistenceError> {
    match (kind, id) {
        ("human", None) => Ok(Actor::Human),
        ("system", None) => Ok(Actor::System),
        ("agent", Some(id)) => Ok(Actor::Agent(parse_id(&id)?)),
        _ => Err(PersistenceError::MemoryRowMismatch),
    }
}
fn actor_json(actor: Actor) -> serde_json::Value {
    match actor {
        Actor::Human => serde_json::json!("Human"),
        Actor::System => serde_json::json!("System"),
        Actor::Agent(id) => serde_json::json!({"Agent":id}),
    }
}

#[cfg(test)]
mod task_13_tests {
    use rusqlite::{Connection, Error as SqliteError, ffi};

    use super::*;

    #[test]
    fn memory_queries_map_sqlite_capacity_codes_without_leaking_messages() {
        for code in [ffi::SQLITE_FULL, ffi::SQLITE_TOOBIG] {
            let mapped = query(SqliteError::SqliteFailure(
                ffi::Error::new(code),
                Some("sensitive sqlite detail".to_owned()),
            ));
            assert_eq!(mapped, PersistenceError::Capacity);
            assert_eq!(mapped.code(), "memory_proposal_capacity_reached");
            assert!(!mapped.to_string().contains("sensitive"));
        }
    }

    #[test]
    fn operation_release_maps_capacity_and_rolls_back_the_savepoint() {
        for code in [ffi::SQLITE_FULL, ffi::SQLITE_TOOBIG] {
            let mut connection = Connection::open_in_memory().unwrap();
            connection
                .execute_batch("CREATE TABLE marker (value INTEGER NOT NULL)")
                .unwrap();
            let transaction = connection.transaction().unwrap();
            transaction
                .execute_batch(
                    "SAVEPOINT memory_repository_operation;
                     INSERT INTO marker(value) VALUES(1)",
                )
                .unwrap();

            let error = release_operation_savepoint(
                &transaction,
                Err(SqliteError::SqliteFailure(
                    ffi::Error::new(code),
                    Some("sensitive release detail".to_owned()),
                )),
            )
            .unwrap_err();

            assert_eq!(error, PersistenceError::Capacity);
            assert_eq!(error.code(), "memory_proposal_capacity_reached");
            assert!(!error.to_string().contains("sensitive"));
            transaction.commit().unwrap();
            assert_eq!(
                connection
                    .query_row("SELECT COUNT(*) FROM marker", [], |row| row
                        .get::<_, i64>(0))
                    .unwrap(),
                0
            );
        }
    }
}
