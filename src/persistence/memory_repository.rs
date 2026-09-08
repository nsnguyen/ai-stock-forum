//! Immutable SQLite codecs for the hybrid-memory domain records.
//!
//! This layer authenticates repository rows and their relational context.  It
//! intentionally does not authenticate event payloads; that correlation is
//! introduced by the later event-payload and verified-stream contracts.

use std::str::FromStr;

use rusqlite::{OptionalExtension, Row, params, params_from_iter, types::Value};

use crate::{
    agents::{AgentProfileVersion, AgentProfileVersionRef},
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

use super::{ImmediateTransaction, PersistenceError, load_all_versions};

const PAGE_LIMIT: u16 = 100;
const MAX_PENDING_MEMORY_PROPOSALS: u64 = 256;

pub struct MemoryRepository;

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
    pub fn insert_entry_version(
        tx: &ImmediateTransaction<'_>,
        creation_sequence: u64,
        entry: &MemoryEntryVersion,
    ) -> Result<(), PersistenceError> {
        validate_event(tx, creation_sequence, entry.creation_event_id())?;
        Self::validate_entry_context(tx, entry)?;
        let expected = entry_row(entry, creation_sequence)?;
        let existing = load_entry_rows_matching(tx, &expected)?;
        match existing.as_slice() {
            [] => insert_entry_row(tx, &expected),
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
        let mut statement = tx
            .transaction()
            .prepare(
                "SELECT entry_version_id FROM memory_entry_versions
             WHERE memory_namespace_id=?1 AND normalized_key=?2
             ORDER BY version DESC, entry_version_id ASC LIMIT ?3",
            )
            .map_err(query)?;
        let ids = statement
            .query_map(
                params![
                    namespace.to_string(),
                    key.as_str(),
                    i64::from(page_limit(limit))
                ],
                |r| r.get::<_, String>(0),
            )
            .map_err(query)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(query)?;
        let mut versions = Vec::with_capacity(ids.len());
        for id in ids {
            versions.push(
                load_entry_by_version_id(tx, parse_id(&id)?)?
                    .ok_or(PersistenceError::MemoryRowMismatch)?,
            );
        }
        page_history(versions, total_count)
    }

    pub fn list_current_entries(
        tx: &ImmediateTransaction<'_>,
        namespace: MemoryNamespaceId,
        limit: u16,
    ) -> Result<MemoryEntriesPage, PersistenceError> {
        let total_count = checked_count(
            tx,
            "SELECT COUNT(*) FROM current_memory_entries c JOIN memory_entry_versions v
             ON v.memory_namespace_id=c.memory_namespace_id AND v.normalized_key=c.normalized_key
             AND v.entry_id=c.entry_id AND v.entry_version_id=c.entry_version_id
             WHERE c.memory_namespace_id=?1 AND v.state='present'",
            [namespace.to_string()],
        )?;
        let mut statement = tx.transaction().prepare(
            "SELECT v.entry_version_id, v.display_key, v.purpose_tags_json, v.value_bytes, v.created_at_ms
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
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Vec<u8>>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, i64>(4)?,
                    ))
                },
            )
            .map_err(query)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(query)?;
        let mut records = Vec::with_capacity(rows.len());
        for (id, display_key, tags_json, value_bytes, created_at_ms) in rows {
            let entry = load_entry_by_version_id(tx, parse_id(&id)?)?
                .ok_or(PersistenceError::MemoryRowMismatch)?;
            let tags: Vec<String> = serde_json::from_slice(&tags_json)
                .map_err(|_| PersistenceError::MemoryRowMismatch)?;
            if entry.reference().namespace_id() != namespace
                || entry.reference().state() != MemoryEntryState::Present
                || entry.display_key() != display_key
                || entry.purpose_tags() != tags
                || value_bytes < 0
                || u64::try_from(value_bytes).map_err(|_| PersistenceError::MemoryRowMismatch)?
                    != entry
                        .value()
                        .ok_or(PersistenceError::MemoryRowMismatch)?
                        .len() as u64
                || created_at_ms != entry.created_at_ms()
            {
                return Err(PersistenceError::MemoryRowMismatch);
            }
            records.push(MemoryEntryListRecord {
                entry: entry.reference(),
                display_key,
                purpose_tags: tags,
                value_bytes: u64::try_from(value_bytes)
                    .map_err(|_| PersistenceError::MemoryRowMismatch)?,
                created_at_ms,
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
        validate_event(tx, creation_sequence, proposal.creation_event_id())?;
        let expected = proposal_row(proposal, creation_sequence)?;
        let existing = load_proposal_rows_matching(tx, &expected)?;
        if existing.is_empty()
            && Self::count_pending_proposals(tx, proposal.namespace_id())?
                >= MAX_PENDING_MEMORY_PROPOSALS
        {
            return Err(PersistenceError::Capacity);
        }
        validate_memory_approval(proposal, approval, None)?;
        insert_approval(tx, approval, None)?;
        Self::validate_proposal_context(tx, proposal)?;
        match existing.as_slice() {
            [] => {
                insert_proposal_row(tx, &expected)?;
                insert_proposal_status(tx, proposal, MemoryProposalStatus::Pending, None)
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
        validate_event(tx, resolution_sequence, resolution.resolution_event_id())?;
        let proposal = Self::load_proposal(tx, resolution.proposal().proposal_id())?
            .ok_or(PersistenceError::MemoryRowMismatch)?
            .0;
        if proposal.reference() != *resolution.proposal() {
            return Err(PersistenceError::MemoryRowMismatch);
        }
        let (_, status, _) = Self::load_proposal(tx, resolution.proposal().proposal_id())?
            .ok_or(PersistenceError::MemoryRowMismatch)?;
        if status != MemoryProposalStatus::Pending {
            return Err(PersistenceError::MemoryRowMismatch);
        }
        validate_memory_approval(&proposal, approval, Some(resolution))?;
        update_approval_resolution(tx, approval, resolution)?;
        let expected = resolution_row(resolution, resolution_sequence)?;
        let existing = load_resolution(tx, resolution.proposal().proposal_id())?;
        match existing {
            None => {
                insert_resolution_row(tx, &expected)?;
                insert_proposal_status(
                    tx,
                    &proposal,
                    resolution.status(),
                    Some(resolution.resolution_event_id()),
                )
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
        let filter_text = proposal_filter(filter);
        let total_count = checked_count(
            tx,
            "SELECT COUNT(*) FROM current_memory_proposal_status WHERE memory_namespace_id=?1 AND (?2='all' OR status='pending')",
            params![namespace.to_string(), filter_text],
        )?;
        let mut statement = tx
            .transaction()
            .prepare(
                "SELECT p.proposal_id FROM current_memory_proposal_status AS c
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
                |r| r.get::<_, String>(0),
            )
            .map_err(query)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(query)?;
        let mut records = Vec::with_capacity(ids.len());
        for id in ids {
            let (proposal, status, _) = Self::load_proposal(tx, parse_id(&id)?)?
                .ok_or(PersistenceError::MemoryRowMismatch)?;
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
        let status = load_proposal_status(tx, &proposal)?;
        let resolution = match load_resolution(tx, proposal_id)? {
            Some(row) => Some(decode_resolution_row(&proposal, row)?),
            None => None,
        };
        match (status, resolution.as_ref()) {
            (MemoryProposalStatus::Pending, None)
            | (
                MemoryProposalStatus::Accepted
                | MemoryProposalStatus::Rejected
                | MemoryProposalStatus::Expired,
                Some(_),
            ) => {}
            _ => return Err(PersistenceError::MemoryRowMismatch),
        }
        Self::validate_proposal_context(tx, &proposal)?;
        Ok(Some((proposal, status, resolution)))
    }

    pub fn count_pending_proposals(
        tx: &ImmediateTransaction<'_>,
        namespace: MemoryNamespaceId,
    ) -> Result<u64, PersistenceError> {
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
        let mut statement=tx.transaction().prepare("SELECT proposal_id FROM current_memory_proposal_status WHERE memory_namespace_id=?1 AND normalized_key=?2 AND status='pending' ORDER BY created_at_ms ASC, proposal_id ASC").map_err(query)?;
        let ids = statement
            .query_map(params![namespace.to_string(), key.as_str()], |r| {
                r.get::<_, String>(0)
            })
            .map_err(query)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(query)?;
        ids.into_iter()
            .map(|id| {
                Self::load_proposal(tx, parse_id(&id)?)?
                    .map(|(p, _, _)| p)
                    .ok_or(PersistenceError::MemoryRowMismatch)
            })
            .collect()
    }
    pub fn load_memory_approval(
        tx: &ImmediateTransaction<'_>,
        approval_id: ApprovalId,
    ) -> Result<Option<ApprovalRecord>, PersistenceError> {
        load_approval(tx, approval_id).map(|row| row.map(|x| x.record))
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
        let approval = Self::load_memory_approval(tx, proposal.approval_id())?
            .ok_or(PersistenceError::MemoryRowMismatch)?;
        validate_proposal_approval_binding(proposal, &approval)
    }

    pub(crate) fn validate_summary_context(
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
        for source in summary.sources() {
            validate_event_source(
                tx,
                source.sequence(),
                source.event_id(),
                source.event_type(),
                source.event_digest(),
            )?;
            if source.sequence() >= summary.creation_event_sequence() {
                return Err(PersistenceError::MemoryRowMismatch);
            }
        }
        Ok(())
    }

    #[allow(dead_code)] // Wired by the later application-service memory task.
    pub(crate) fn insert_episodic_summary(
        tx: &ImmediateTransaction<'_>,
        creation_sequence: u64,
        summary: &EpisodicSummary,
    ) -> Result<(), PersistenceError> {
        if creation_sequence != summary.creation_event_sequence() {
            return Err(PersistenceError::MemoryRowMismatch);
        }
        Self::validate_summary_context(tx, summary)?;
        let expected = summary_row(summary)?;
        let existing = load_summary_row(tx, summary.reference().summary_id())?;
        match existing {
            None => {
                insert_summary_row(tx, &expected)?;
                for (ordinal, source) in summary.sources().iter().enumerate() {
                    tx.transaction().execute("INSERT INTO episodic_summary_sources (summary_id,source_ordinal,event_sequence,event_id,event_type,event_digest) VALUES (?1,?2,?3,?4,?5,?6)",params![expected.summary_id,to_i64(ordinal as u64)?,to_i64(source.sequence())?,source.event_id().to_string(),source.event_type(),source.event_digest().as_str()]).map_err(query)?;
                }
                Ok(())
            }
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
        let total_count = checked_count(
            tx,
            "SELECT COUNT(*) FROM episodic_summaries WHERE memory_namespace_id=?1",
            [namespace.to_string()],
        )?;
        let mut statement=tx.transaction().prepare("SELECT summary_id FROM episodic_summaries WHERE memory_namespace_id=?1 ORDER BY created_at_ms DESC, summary_id ASC LIMIT ?2").map_err(query)?;
        let ids = statement
            .query_map(
                params![namespace.to_string(), i64::from(page_limit(limit))],
                |r| r.get::<_, String>(0),
            )
            .map_err(query)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(query)?;
        let mut records = Vec::with_capacity(ids.len());
        for id in ids {
            let summary = Self::load_episodic_summary(tx, parse_id(&id)?)?
                .ok_or(PersistenceError::MemoryRowMismatch)?;
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
        Self::validate_summary_context(tx, &summary)?;
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
        match scope.purpose() {
            MemoryPurposeScope::General => {
                stream_entries(
                    tx,
                    namespace,
                    "json_array_length(CAST(v.purpose_tags_json AS TEXT))=0",
                    Vec::new(),
                    &mut builder,
                )?;
                stream_summaries(
                    tx,
                    namespace,
                    scope.profile(),
                    "json_array_length(CAST(s.purpose_tags_json AS TEXT))=0",
                    Vec::new(),
                    &mut builder,
                )?;
            }
            MemoryPurposeScope::Tagged(tags) => {
                let clause = tag_clause("v.purpose_tags_json", 2, tags.len());
                let values = tags.iter().cloned().map(Value::Text).collect();
                stream_entries(tx, namespace, &clause, values, &mut builder)?;
                stream_entries(
                    tx,
                    namespace,
                    "json_array_length(CAST(v.purpose_tags_json AS TEXT))=0",
                    Vec::new(),
                    &mut builder,
                )?;
                let clause = tag_clause("s.purpose_tags_json", 6, tags.len());
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
                    "json_array_length(CAST(s.purpose_tags_json AS TEXT))=0",
                    Vec::new(),
                    &mut builder,
                )?;
            }
        }
        builder.finish().map_err(integrity)
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

fn insert_entry_row(tx: &ImmediateTransaction<'_>, r: &EntryRow) -> Result<(), PersistenceError> {
    tx.transaction().execute("INSERT INTO memory_entry_versions (memory_namespace_id,entry_id,entry_version_id,version,predecessor_version_id,display_key,normalized_key,state,value_text,value_bytes,purpose_tags_json,created_by_kind,created_by_id,created_at_ms,accepted_proposal_id,accepted_proposal_version,accepted_proposal_digest,plaintext_validation_version,creation_event_sequence,creation_event_id,content_digest,record_digest,record_json) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23)",params![r.namespace,r.entry_id,r.entry_version_id,r.version,r.predecessor,r.display_key,r.normalized_key,r.state,r.value,r.value_bytes,r.tags,r.created_by_kind,r.created_by_id,r.created_at,r.accepted_id,r.accepted_version,r.accepted_digest,r.plaintext,r.creation_sequence,r.creation_event_id,r.content_digest,r.record_digest,r.record_json]).map(|_|()).map_err(query)
}
fn load_entry_rows_matching(
    tx: &ImmediateTransaction<'_>,
    e: &EntryRow,
) -> Result<Vec<EntryRow>, PersistenceError> {
    let mut s=tx.transaction().prepare("SELECT memory_namespace_id,entry_id,entry_version_id,version,predecessor_version_id,display_key,normalized_key,state,value_text,value_bytes,purpose_tags_json,created_by_kind,created_by_id,created_at_ms,accepted_proposal_id,accepted_proposal_version,accepted_proposal_digest,plaintext_validation_version,creation_event_sequence,creation_event_id,content_digest,record_digest,record_json FROM memory_entry_versions WHERE (entry_id=?1 AND version=?2) OR entry_version_id=?3 ORDER BY entry_id,version").map_err(query)?;
    s.query_map(
        params![e.entry_id, e.version, e.entry_version_id],
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
    let entry: MemoryEntryVersion =
        serde_json::from_slice(&r.record_json).map_err(|_| PersistenceError::MemoryRowMismatch)?;
    let expected = entry_row(&entry, to_u64(r.creation_sequence)?)?;
    if expected != r {
        return Err(PersistenceError::MemoryRowMismatch);
    };
    MemoryRepository::validate_entry_context(tx, &entry)?;
    Ok(entry)
}
fn load_current_pointer(
    tx: &ImmediateTransaction<'_>,
    namespace: MemoryNamespaceId,
    key: &NormalizedMemoryKey,
) -> Result<Option<MemoryEntryRef>, PersistenceError> {
    let row=tx.transaction().query_row("SELECT entry_id,entry_version_id,version,state,content_digest FROM current_memory_entries WHERE memory_namespace_id=?1 AND normalized_key=?2",params![namespace.to_string(),key.as_str()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?))).optional().map_err(query)?;
    row.map(|(entry_id,version_id,version,state,digest)|{let version=ObjectVersion::new(to_u64(version)?).map_err(integrity)?;let digest=Digest::parse(&digest).map_err(integrity)?;let state=parse_entry_state(&state)?;let entry_id:crate::domain::MemoryEntryId=parse_id(&entry_id)?;let version_id:MemoryEntryVersionId=parse_id(&version_id)?;serde_json::from_value(serde_json::json!({"namespace_id":namespace,"entry_id":entry_id,"entry_version_id":version_id,"version":version,"normalized_key":key,"state":state,"content_digest":digest})).map_err(|_|PersistenceError::MemoryRowMismatch)}).transpose()
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
    let p: MemoryProposal =
        serde_json::from_slice(&r.record_json).map_err(|_| PersistenceError::MemoryRowMismatch)?;
    let expected = proposal_row(&p, to_u64(r.creation_sequence)?)?;
    if expected != r {
        return Err(PersistenceError::MemoryRowMismatch);
    };
    validate_event(tx, to_u64(r.creation_sequence)?, p.creation_event_id())?;
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
) -> Result<MemoryProposalStatus, PersistenceError> {
    let row=tx.transaction().query_row("SELECT status,resolution_event_id FROM current_memory_proposal_status WHERE proposal_id=?1",[p.reference().proposal_id().to_string()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?))).optional().map_err(query)?.ok_or(PersistenceError::MemoryRowMismatch)?;
    let status = parse_proposal_status(&row.0)?;
    validate_proposal_status(tx, p, status, row.1.map(|x| parse_id(&x)).transpose()?)?;
    Ok(status)
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
    let summary: EpisodicSummary =
        serde_json::from_slice(&r.record_json).map_err(|_| PersistenceError::MemoryRowMismatch)?;
    let expected = summary_row(&summary)?;
    if expected != r {
        return Err(PersistenceError::MemoryRowMismatch);
    };
    validate_summary_sources(tx, &summary)?;
    Ok(summary)
}
fn validate_summary_sources(
    tx: &ImmediateTransaction<'_>,
    summary: &EpisodicSummary,
) -> Result<(), PersistenceError> {
    let mut statement=tx.transaction().prepare("SELECT source_ordinal,event_sequence,event_id,event_type,event_digest FROM episodic_summary_sources WHERE summary_id=?1 ORDER BY source_ordinal ASC").map_err(query)?;
    let rows = statement
        .query_map([summary.reference().summary_id().to_string()], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })
        .map_err(query)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(query)?;
    if rows.len() != summary.sources().len() {
        return Err(PersistenceError::MemoryRowMismatch);
    };
    for (ordinal, ((stored_ordinal, sequence, event_id, event_type, event_digest), source)) in
        rows.into_iter().zip(summary.sources()).enumerate()
    {
        if stored_ordinal != to_i64(ordinal as u64)?
            || to_u64(sequence)? != source.sequence()
            || event_id != source.event_id().to_string()
            || event_type != source.event_type()
            || event_digest != source.event_digest().as_str()
        {
            return Err(PersistenceError::MemoryRowMismatch);
        }
    }
    MemoryRepository::validate_summary_context(tx, summary)
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
) -> Result<(), PersistenceError> {
    let c = approval_columns(record, resolution_event)?;
    let existing = load_approval(tx, record.approval_id())?;
    match existing{None=>tx.transaction().execute("INSERT INTO approval_records (approval_id,action_kind,object_kind,object_id,object_version,object_digest,actor_kind,actor_id,status,created_at_ms,expires_at_ms,resolved_at_ms,resolution_kind,resolution_event_id,resolution_actor_kind,resolution_actor_id) VALUES (?16,?1,?2,?3,?4,?5,?7,?6,?9,?10,?11,?12,?13,?8,?14,?15)",params![c.0,c.1,c.2,c.3,c.4,c.5,c.6,c.7,c.8,c.9,c.10,c.11,c.12,c.13,c.14,c.15]).map(|_|()).map_err(query),Some(stored) if approval_equal(&stored,record,resolution_event)=>Ok(()),_=>Err(PersistenceError::MemoryRowMismatch)}
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

fn stream_entries(
    tx: &ImmediateTransaction<'_>,
    namespace: MemoryNamespaceId,
    where_clause: &str,
    values: Vec<Value>,
    builder: &mut MemorySnapshotBuilder,
) -> Result<(), PersistenceError> {
    let sql = format!(
        "SELECT v.memory_namespace_id,v.entry_id,v.entry_version_id,v.version,v.predecessor_version_id,v.display_key,v.normalized_key,v.state,v.value_text,v.value_bytes,v.purpose_tags_json,v.created_by_kind,v.created_by_id,v.created_at_ms,v.accepted_proposal_id,v.accepted_proposal_version,v.accepted_proposal_digest,v.plaintext_validation_version,v.creation_event_sequence,v.creation_event_id,v.content_digest,v.record_digest,v.record_json FROM current_memory_entries c JOIN memory_entry_versions v ON v.memory_namespace_id=c.memory_namespace_id AND v.normalized_key=c.normalized_key AND v.entry_id=c.entry_id AND v.entry_version_id=c.entry_version_id WHERE c.memory_namespace_id=?1 AND v.state='present' AND {where_clause} ORDER BY c.normalized_key ASC,v.version ASC,c.entry_id ASC,v.entry_version_id ASC"
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
        let entry = decode_entry_row_checked(tx, stored)?;
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
        "SELECT summary_id,version,memory_namespace_id,profile_id,profile_version_id,profile_version,profile_content_digest,label,body,purpose_tags_json,source_count,plaintext_validation_version,created_at_ms,creation_event_sequence,creation_event_id,source_set_digest,content_digest,record_digest,record_json FROM episodic_summaries s WHERE s.memory_namespace_id=?1 AND s.profile_id=?2 AND s.profile_version_id=?3 AND s.profile_version=?4 AND s.profile_content_digest=?5 AND {where_clause} ORDER BY s.created_at_ms DESC,s.summary_id ASC"
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
        let summary = decode_summary_row(tx, stored)?;
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

fn load_exact_profile(
    tx: &ImmediateTransaction<'_>,
    reference: &AgentProfileVersionRef,
) -> Result<AgentProfileVersion, PersistenceError> {
    let mut matches = load_all_versions(tx.transaction())?
        .into_iter()
        .filter(|stored| stored.profile.reference() == *reference)
        .map(|stored| stored.profile);
    let profile = matches.next().ok_or(PersistenceError::MemoryRowMismatch)?;
    if matches.next().is_some() {
        return Err(PersistenceError::MemoryRowMismatch);
    };
    Ok(profile)
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
fn validate_event_source(
    tx: &ImmediateTransaction<'_>,
    sequence: u64,
    id: EventId,
    event_type: &str,
    digest: &Digest,
) -> Result<(), PersistenceError> {
    let found:bool=tx.transaction().query_row("SELECT EXISTS(SELECT 1 FROM event_stream WHERE sequence=?1 AND event_id=?2 AND event_type=?3 AND event_digest=?4)",params![to_i64(sequence)?,id.to_string(),event_type,digest.as_str()],|r|r.get(0)).map_err(query)?;
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
fn query(_: rusqlite::Error) -> PersistenceError {
    PersistenceError::QueryFailed
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
