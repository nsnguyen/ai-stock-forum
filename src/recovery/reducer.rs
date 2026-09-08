use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Deserializer, Serialize};

use crate::{
    agents::AgentProfilesProjection,
    app::{ApplicationEvent, EVENT_SCHEMA_VERSION, EventEnvelope, ShutdownReason},
    domain::{
        Actor, Digest, EventId, InstallationId, MemoryNamespaceId, ObjectVersion, SessionId,
        Sha256Digest, SkillId, SkillVersionId, canonical_json_bytes, sha256,
    },
    memory::{
        ExpectedMemoryEntryState, MemoryEntryRef, MemoryEntryState, MemoryEntryVersion,
        MemoryProjection, MemoryProposal, MemoryProposalResolution, MemoryProposalStatus,
        NormalizedMemoryKey,
    },
    persistence::RecoveryError,
    policy::{ApprovalAction, ApprovalRecord, ApprovalStatus},
    setup::SetupStatus,
    skills::{SkillProvenance, SkillVersionRef},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectionState {
    pub installation: Option<InstallationProjection>,
    pub sessions: BTreeMap<SessionId, SessionProjection>,
    #[serde(skip_serializing_if = "AgentProfilesProjection::is_empty")]
    pub agent_profiles: AgentProfilesProjection,
    #[serde(skip_serializing_if = "SkillsProjection::is_empty")]
    pub skills: SkillsProjection,
    #[serde(default, skip_serializing_if = "MemoryProjection::is_empty")]
    pub memory: MemoryProjection,
    pub setup_status: SetupStatus,
    pub last_sequence: u64,
    pub last_event_digest: Option<Sha256Digest>,
}

impl Default for ProjectionState {
    fn default() -> Self {
        Self {
            installation: None,
            sessions: BTreeMap::new(),
            agent_profiles: AgentProfilesProjection::default(),
            skills: SkillsProjection::default(),
            memory: MemoryProjection::default(),
            setup_status: SetupStatus::NotStarted,
            last_sequence: 0,
            last_event_digest: None,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectionStateWire {
    installation: Option<InstallationProjection>,
    sessions: BTreeMap<SessionId, SessionProjection>,
    #[serde(default)]
    agent_profiles: AgentProfilesProjection,
    #[serde(default)]
    skills: SkillsProjection,
    #[serde(default)]
    memory: MemoryProjection,
    setup_status: SetupStatus,
    last_sequence: u64,
    last_event_digest: Option<Sha256Digest>,
}

impl<'de> Deserialize<'de> for ProjectionState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ProjectionStateWire::deserialize(deserializer)?;
        let state = Self {
            installation: wire.installation,
            sessions: wire.sessions,
            agent_profiles: wire.agent_profiles,
            skills: wire.skills,
            memory: wire.memory,
            setup_status: wire.setup_status,
            last_sequence: wire.last_sequence,
            last_event_digest: wire.last_event_digest,
        };
        state.validate().map_err(serde::de::Error::custom)?;
        Ok(state)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallationProjection {
    pub installation_id: InstallationId,
    pub created_event_id: EventId,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionProjection {
    pub session_id: SessionId,
    pub started_event_id: EventId,
    pub started_at_ms: i64,
    pub ended: Option<SessionEndProjection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionEndProjection {
    pub ended_event_id: EventId,
    pub ended_at_ms: i64,
    pub reason: ShutdownReason,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillsProjection {
    versions_by_id: BTreeMap<SkillVersionId, ProjectedSkillVersion>,
    active_by_skill: BTreeMap<SkillId, SkillVersionId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectedSkillVersion {
    skill: SkillVersionRef,
    display_name: String,
    provenance: SkillProvenance,
    predecessor_version_id: Option<SkillVersionId>,
    record_digest: Sha256Digest,
}

impl SkillsProjection {
    pub fn is_empty(&self) -> bool {
        self.versions_by_id.is_empty() && self.active_by_skill.is_empty()
    }

    pub fn active_skill(&self, skill_id: SkillId) -> Option<&SkillVersionRef> {
        self.active_by_skill
            .get(&skill_id)
            .and_then(|version_id| self.versions_by_id.get(version_id))
            .map(|version| &version.skill)
    }

    pub(crate) fn version_metadata(
        &self,
    ) -> impl Iterator<
        Item = (
            &SkillVersionRef,
            &str,
            &SkillProvenance,
            Option<SkillVersionId>,
            &Sha256Digest,
        ),
    > {
        self.versions_by_id.values().map(|version| {
            (
                &version.skill,
                version.display_name.as_str(),
                &version.provenance,
                version.predecessor_version_id,
                &version.record_digest,
            )
        })
    }

    pub(crate) fn active_refs(&self) -> impl Iterator<Item = &SkillVersionRef> {
        self.active_by_skill.values().filter_map(|version_id| {
            self.versions_by_id
                .get(version_id)
                .map(|version| &version.skill)
        })
    }

    fn create(
        &mut self,
        skill: &SkillVersionRef,
        display_name: &str,
        provenance: &SkillProvenance,
        record_digest: Sha256Digest,
    ) -> Result<(), RecoveryError> {
        if collides_with_canonical_builtin_identity(skill)?
            || skill.version().get() != 1
            || self.versions_by_id.contains_key(&skill.skill_version_id())
            || self.active_by_skill.contains_key(&skill.skill_id())
        {
            return Err(RecoveryError::InvalidEventRecord);
        }
        self.versions_by_id.insert(
            skill.skill_version_id(),
            ProjectedSkillVersion {
                skill: skill.clone(),
                display_name: display_name.to_owned(),
                provenance: provenance.clone(),
                predecessor_version_id: None,
                record_digest,
            },
        );
        self.active_by_skill
            .insert(skill.skill_id(), skill.skill_version_id());
        Ok(())
    }

    fn activate(
        &mut self,
        skill: &SkillVersionRef,
        previous_version_id: SkillVersionId,
        display_name: &str,
        provenance: &SkillProvenance,
        record_digest: Sha256Digest,
    ) -> Result<(), RecoveryError> {
        if self.versions_by_id.contains_key(&skill.skill_version_id()) {
            return Err(RecoveryError::InvalidEventRecord);
        }
        if let Some(current_id) = self.active_by_skill.get(&skill.skill_id()).copied() {
            let current = self
                .versions_by_id
                .get(&current_id)
                .ok_or(RecoveryError::InvalidEventRecord)?;
            if current_id != previous_version_id
                || current.skill.version().get().checked_add(1) != Some(skill.version().get())
                || current.provenance != *provenance
            {
                return Err(RecoveryError::InvalidEventRecord);
            }
        } else if !is_canonical_builtin_successor(skill, previous_version_id, provenance)? {
            return Err(RecoveryError::InvalidEventRecord);
        }
        self.versions_by_id.insert(
            skill.skill_version_id(),
            ProjectedSkillVersion {
                skill: skill.clone(),
                display_name: display_name.to_owned(),
                provenance: provenance.clone(),
                predecessor_version_id: Some(previous_version_id),
                record_digest,
            },
        );
        self.active_by_skill
            .insert(skill.skill_id(), skill.skill_version_id());
        Ok(())
    }

    fn validate(&self) -> Result<(), RecoveryError> {
        let mut by_skill =
            BTreeMap::<SkillId, BTreeMap<ObjectVersion, &ProjectedSkillVersion>>::new();
        for (version_id, version) in &self.versions_by_id {
            if version_id != &version.skill.skill_version_id()
                || by_skill
                    .entry(version.skill.skill_id())
                    .or_default()
                    .insert(version.skill.version(), version)
                    .is_some()
            {
                return Err(RecoveryError::InvalidEventRecord);
            }
        }
        if by_skill.len() != self.active_by_skill.len() {
            return Err(RecoveryError::InvalidEventRecord);
        }
        for (skill_id, versions) in by_skill {
            let mut previous: Option<&ProjectedSkillVersion> = None;
            for version in versions.values() {
                if let Some(previous) = previous {
                    if previous.skill.version().get().checked_add(1)
                        != Some(version.skill.version().get())
                        || version.predecessor_version_id != Some(previous.skill.skill_version_id())
                        || version.provenance != previous.provenance
                    {
                        return Err(RecoveryError::InvalidEventRecord);
                    }
                } else if version.skill.version().get() == 1 {
                    if version.predecessor_version_id.is_some() {
                        return Err(RecoveryError::InvalidEventRecord);
                    }
                } else {
                    let predecessor = version
                        .predecessor_version_id
                        .ok_or(RecoveryError::InvalidEventRecord)?;
                    if !is_canonical_builtin_successor(
                        &version.skill,
                        predecessor,
                        &version.provenance,
                    )? {
                        return Err(RecoveryError::InvalidEventRecord);
                    }
                }
                previous = Some(version);
            }
            let active_id = self
                .active_by_skill
                .get(&skill_id)
                .ok_or(RecoveryError::InvalidEventRecord)?;
            if previous.map(|version| version.skill.skill_version_id()) != Some(*active_id) {
                return Err(RecoveryError::InvalidEventRecord);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReducerEffect {
    None,
    PreviousSessionInterrupted { session_id: SessionId },
}

impl ProjectionState {
    pub fn digest(&self) -> Result<Sha256Digest, crate::domain::DomainError> {
        canonical_json_bytes(&PersistentProjectionState {
            installation: &self.installation,
            sessions: &self.sessions,
            agent_profiles: &self.agent_profiles,
            skills: &self.skills,
            memory: &self.memory,
            setup_status: &self.setup_status,
            last_sequence: self.last_sequence,
            last_event_digest: &self.last_event_digest,
        })
        .map(|bytes| sha256(&bytes))
    }

    pub(crate) fn validate(&self) -> Result<(), RecoveryError> {
        if (self.last_sequence == 0) != self.last_event_digest.is_none() {
            return Err(RecoveryError::InvalidEventRecord);
        }
        if self.last_sequence == 0 && self.installation.is_some() {
            return Err(RecoveryError::InvalidEventRecord);
        }
        if self.setup_status != SetupStatus::NotStarted {
            return Err(RecoveryError::InvalidEventRecord);
        }
        self.skills.validate()?;
        if self.installation.is_none() && !self.sessions.is_empty() {
            return Err(RecoveryError::InvalidEventRecord);
        }
        if self
            .sessions
            .iter()
            .any(|(session_id, projection)| session_id != &projection.session_id)
        {
            return Err(RecoveryError::InvalidEventRecord);
        }
        let minimum_events = u64::from(self.installation.is_some())
            .checked_add(
                u64::try_from(self.sessions.len())
                    .map_err(|_| RecoveryError::InvalidEventRecord)?,
            )
            .and_then(|value| {
                value.checked_add(
                    u64::try_from(
                        self.sessions
                            .values()
                            .filter(|session| session.ended.is_some())
                            .count(),
                    )
                    .ok()?,
                )
            })
            .ok_or(RecoveryError::EventSequenceOverflow)?;
        if self.last_sequence < minimum_events {
            return Err(RecoveryError::InvalidEventRecord);
        }
        if self
            .sessions
            .values()
            .filter(|projection| projection.ended.is_none())
            .count()
            > 1
        {
            return Err(RecoveryError::InvalidEventRecord);
        }
        Ok(())
    }
}

#[derive(Serialize)]
struct PersistentProjectionState<'a> {
    installation: &'a Option<InstallationProjection>,
    sessions: &'a BTreeMap<SessionId, SessionProjection>,
    #[serde(skip_serializing_if = "agent_profiles_are_empty")]
    agent_profiles: &'a AgentProfilesProjection,
    #[serde(skip_serializing_if = "skills_are_empty")]
    skills: &'a SkillsProjection,
    #[serde(skip_serializing_if = "memory_is_empty")]
    memory: &'a MemoryProjection,
    setup_status: &'a SetupStatus,
    last_sequence: u64,
    last_event_digest: &'a Option<Sha256Digest>,
}

fn agent_profiles_are_empty(profiles: &&AgentProfilesProjection) -> bool {
    profiles.is_empty()
}

fn skills_are_empty(skills: &&SkillsProjection) -> bool {
    skills.is_empty()
}

fn memory_is_empty(memory: &&MemoryProjection) -> bool {
    memory.is_empty()
}

pub fn reduce(
    state: &mut ProjectionState,
    event: &EventEnvelope,
) -> Result<ReducerEffect, RecoveryError> {
    if event.event_schema_version != EVENT_SCHEMA_VERSION {
        return Err(RecoveryError::UnsupportedEventSchema);
    }
    let expected_sequence = state
        .last_sequence
        .checked_add(1)
        .ok_or(RecoveryError::EventSequenceOverflow)?;
    if event.sequence != expected_sequence {
        return Err(RecoveryError::EventSequenceGap);
    }
    if event.previous_event_digest != state.last_event_digest {
        return Err(RecoveryError::PreviousEventDigestMismatch);
    }

    let mut next = state.clone();
    let mut effect = ReducerEffect::None;
    match &event.event {
        ApplicationEvent::InstallationInitialized { installation_id } => {
            if next.installation.is_some() {
                return Err(RecoveryError::InvalidEventRecord);
            }
            next.installation = Some(InstallationProjection {
                installation_id: *installation_id,
                created_event_id: event.event_id,
                created_at_ms: event.occurred_at_ms,
            });
        }
        ApplicationEvent::ProcessSessionStarted { session_id } => {
            if next.installation.is_none()
                || next.sessions.contains_key(session_id)
                || next
                    .sessions
                    .values()
                    .any(|session| session.ended.is_none())
            {
                return Err(RecoveryError::InvalidEventRecord);
            }
            next.sessions.insert(
                *session_id,
                SessionProjection {
                    session_id: *session_id,
                    started_event_id: event.event_id,
                    started_at_ms: event.occurred_at_ms,
                    ended: None,
                },
            );
        }
        ApplicationEvent::PreviousSessionInterrupted { session_id } => {
            end_session(&mut next, *session_id, event, ShutdownReason::Interrupted)?;
            effect = ReducerEffect::PreviousSessionInterrupted {
                session_id: *session_id,
            };
        }
        ApplicationEvent::ProcessSessionEnded { session_id, reason } => {
            end_session(&mut next, *session_id, event, *reason)?;
        }
        ApplicationEvent::ProjectionRebuilt { through_sequence } => {
            if *through_sequence != event.sequence - 1 {
                return Err(RecoveryError::InvalidEventRecord);
            }
        }
        ApplicationEvent::HelpViewed
        | ApplicationEvent::StatusViewed
        | ApplicationEvent::SetupStatusViewed
        | ApplicationEvent::AuditTailViewed { .. }
        | ApplicationEvent::CommandRejected { .. }
        | ApplicationEvent::ShutdownRequested
        | ApplicationEvent::AgentProfileCreated { .. }
        | ApplicationEvent::AgentProfileVersionActivated { .. }
        | ApplicationEvent::AgentProfilesListed { .. }
        | ApplicationEvent::AgentProfileViewed { .. }
        | ApplicationEvent::AgentProfileHistoryViewed { .. }
        | ApplicationEvent::AgentProfileVersionViewed { .. }
        | ApplicationEvent::SkillsListed { .. }
        | ApplicationEvent::SkillViewed { .. }
        | ApplicationEvent::SkillHistoryViewed { .. }
        | ApplicationEvent::SkillVersionViewed { .. } => {}
        ApplicationEvent::SkillCreated {
            skill,
            display_name,
            provenance,
        } => {
            let record_digest = authenticated_skill_record_digest(event, skill)?;
            next.skills
                .create(skill, display_name, provenance, record_digest)?;
        }
        ApplicationEvent::SkillVersionActivated {
            skill,
            previous_version_id,
            display_name,
            provenance,
        } => {
            let record_digest = authenticated_skill_record_digest(event, skill)?;
            next.skills.activate(
                skill,
                *previous_version_id,
                display_name,
                provenance,
                record_digest,
            )?;
        }
        ApplicationEvent::AgentSkillAssigned {
            profile,
            previous_profile_version_id,
            ..
        }
        | ApplicationEvent::AgentSkillUpgraded {
            profile,
            previous_profile_version_id,
            ..
        }
        | ApplicationEvent::AgentSkillUnassigned {
            profile,
            previous_profile_version_id,
            ..
        } => {
            next.agent_profiles
                .reduce(&ApplicationEvent::AgentProfileVersionActivated {
                    profile: profile.clone(),
                    previous_version_id: *previous_profile_version_id,
                })?;
        }
        ApplicationEvent::MemoryEntrySet {
            entry,
            expired_proposals,
        } => apply_direct_memory_entry(
            &mut next.memory,
            event,
            entry,
            expired_proposals,
            MemoryEntryState::Present,
        )?,
        ApplicationEvent::MemoryEntryDeleted {
            entry,
            expired_proposals,
        } => apply_direct_memory_entry(
            &mut next.memory,
            event,
            entry,
            expired_proposals,
            MemoryEntryState::Deleted,
        )?,
        ApplicationEvent::MemoryProposalCreated { proposal, approval } => {
            validate_proposal_created(&next, event, proposal, approval)?;
            next.memory
                .create_proposal(proposal)
                .map_err(|_| RecoveryError::InvalidEventRecord)?;
        }
        ApplicationEvent::MemoryProposalAccepted {
            resolution,
            entry,
            expired_proposals,
        } => apply_accepted_memory_proposal(
            &mut next.memory,
            event,
            resolution,
            entry,
            expired_proposals,
        )?,
        ApplicationEvent::MemoryProposalRejected { resolution } => {
            validate_primary_proposal_object(event, resolution)?;
            validate_terminal_resolution(event, resolution, MemoryProposalStatus::Rejected)?;
            let projected = projected_proposal(&next.memory, resolution)?;
            if current_expected(&next.memory, projected.0, &projected.1) != projected.2 {
                return Err(RecoveryError::InvalidEventRecord);
            }
            next.memory
                .resolve_proposal(resolution)
                .map_err(|_| RecoveryError::InvalidEventRecord)?;
        }
        ApplicationEvent::EpisodicSummaryRecorded { summary } => {
            let reference = summary.reference();
            validate_primary_object(
                event,
                "episodic_summary",
                reference.summary_id().to_string(),
                reference.version(),
                reference.content_digest(),
            )?;
            if event.actor != Actor::System
                || summary.created_at_ms() != event.occurred_at_ms
                || summary.creation_event_id() != event.event_id
                || summary.creation_event_sequence() != event.sequence
            {
                return Err(RecoveryError::InvalidEventRecord);
            }
            let profile = next
                .agent_profiles
                .resolve_reference(reference.profile())
                .map_err(|_| RecoveryError::InvalidEventRecord)?;
            if profile.memory_namespace_id() != reference.namespace_id() {
                return Err(RecoveryError::InvalidEventRecord);
            }
        }
        ApplicationEvent::MemoryEntriesListed { .. }
        | ApplicationEvent::MemoryEntryShown { .. }
        | ApplicationEvent::MemoryEntryHistoryShown { .. }
        | ApplicationEvent::MemoryEntryVersionShown { .. }
        | ApplicationEvent::MemoryProposalsListed { .. }
        | ApplicationEvent::MemoryProposalShown { .. }
        | ApplicationEvent::EpisodicSummariesListed { .. }
        | ApplicationEvent::EpisodicSummaryShown { .. }
        | ApplicationEvent::MemorySnapshotBuilt { .. } => validate_memory_read_event(&next, event)?,
    }
    next.agent_profiles.reduce(&event.event)?;
    next.last_sequence = event.sequence;
    next.last_event_digest = Some(event.event_digest.clone());
    next.validate()?;
    *state = next;
    Ok(effect)
}

fn apply_direct_memory_entry(
    memory: &mut MemoryProjection,
    event: &EventEnvelope,
    entry: &MemoryEntryVersion,
    expired_proposals: &[MemoryProposalResolution],
    expected_state: MemoryEntryState,
) -> Result<(), RecoveryError> {
    validate_entry_envelope(event, entry, expected_state)?;
    if entry.accepted_proposal().is_some() {
        return Err(RecoveryError::InvalidEventRecord);
    }
    memory
        .apply_entry(entry)
        .map_err(|_| RecoveryError::InvalidEventRecord)?;
    apply_expirations(memory, event, entry, expired_proposals, None)
}

fn apply_accepted_memory_proposal(
    memory: &mut MemoryProjection,
    event: &EventEnvelope,
    resolution: &MemoryProposalResolution,
    entry: &MemoryEntryVersion,
    expired_proposals: &[MemoryProposalResolution],
) -> Result<(), RecoveryError> {
    validate_primary_proposal_object(event, resolution)?;
    validate_terminal_resolution(event, resolution, MemoryProposalStatus::Accepted)?;
    let projected = projected_proposal(memory, resolution)?;
    if current_expected(memory, projected.0, &projected.1) != projected.2
        || entry.reference().namespace_id() != projected.0
        || entry.reference().normalized_key() != &projected.1
        || entry.accepted_proposal() != Some(resolution.proposal())
    {
        return Err(RecoveryError::InvalidEventRecord);
    }
    validate_entry_record_context(event, entry)?;
    memory
        .apply_entry(entry)
        .map_err(|_| RecoveryError::InvalidEventRecord)?;
    memory
        .resolve_proposal(resolution)
        .map_err(|_| RecoveryError::InvalidEventRecord)?;
    apply_expirations(
        memory,
        event,
        entry,
        expired_proposals,
        Some(resolution.proposal().proposal_id()),
    )
}

fn validate_proposal_created(
    state: &ProjectionState,
    event: &EventEnvelope,
    proposal: &MemoryProposal,
    approval: &ApprovalRecord,
) -> Result<(), RecoveryError> {
    let reference = proposal.reference();
    let proposer = state
        .agent_profiles
        .resolve_reference(proposal.proposer())
        .map_err(|_| RecoveryError::InvalidEventRecord)?;
    validate_primary_object(
        event,
        "memory_proposal",
        reference.proposal_id().to_string(),
        reference.version(),
        reference.content_digest(),
    )?;
    if event.actor != Actor::Agent(proposal.proposer().profile_id())
        || proposer.memory_namespace_id() != proposal.namespace_id()
        || proposal.creation_event_id() != event.event_id
        || proposal.created_at_ms() != event.occurred_at_ms
        || current_expected(
            &state.memory,
            proposal.namespace_id(),
            proposal.normalized_key(),
        ) != *proposal.expected()
        || approval.approval_id() != proposal.approval_id()
        || approval.action() != ApprovalAction::MemoryMutation
        || approval.object()
            != &proposal
                .object_ref()
                .map_err(|_| RecoveryError::InvalidEventRecord)?
        || approval.actor() != &event.actor
        || approval.status() != ApprovalStatus::Pending
        || approval.created_at_millis() != event.occurred_at_ms
        || approval.expires_at_millis().is_some()
        || approval.resolution().is_some()
    {
        return Err(RecoveryError::InvalidEventRecord);
    }
    Ok(())
}

fn validate_entry_envelope(
    event: &EventEnvelope,
    entry: &MemoryEntryVersion,
    expected_state: MemoryEntryState,
) -> Result<(), RecoveryError> {
    let reference = entry.reference();
    validate_primary_object(
        event,
        "memory_entry_version",
        reference.entry_version_id().to_string(),
        reference.version(),
        reference.content_digest(),
    )?;
    if reference.state() != expected_state {
        return Err(RecoveryError::InvalidEventRecord);
    }
    validate_entry_record_context(event, entry)
}

fn validate_entry_record_context(
    event: &EventEnvelope,
    entry: &MemoryEntryVersion,
) -> Result<(), RecoveryError> {
    if event.actor != Actor::Human
        || entry.created_by() != &event.actor
        || entry.created_at_ms() != event.occurred_at_ms
        || entry.creation_event_id() != event.event_id
    {
        return Err(RecoveryError::InvalidEventRecord);
    }
    Ok(())
}

fn validate_primary_proposal_object(
    event: &EventEnvelope,
    resolution: &MemoryProposalResolution,
) -> Result<(), RecoveryError> {
    validate_primary_object(
        event,
        "memory_proposal",
        resolution.proposal().proposal_id().to_string(),
        resolution.proposal().version(),
        resolution.proposal().content_digest(),
    )
}

fn validate_primary_object(
    event: &EventEnvelope,
    kind: &str,
    id: String,
    version: ObjectVersion,
    digest: &Digest,
) -> Result<(), RecoveryError> {
    match event.object.as_ref() {
        Some(object)
            if object.kind == kind
                && object.id == id
                && object.version == version
                && &object.digest == digest =>
        {
            Ok(())
        }
        _ => Err(RecoveryError::InvalidEventRecord),
    }
}

fn validate_terminal_resolution(
    event: &EventEnvelope,
    resolution: &MemoryProposalResolution,
    status: MemoryProposalStatus,
) -> Result<(), RecoveryError> {
    if event.actor != Actor::Human
        || resolution.status() != status
        || resolution.resolved_by() != &event.actor
        || resolution.resolved_at_ms() != event.occurred_at_ms
        || resolution.resolution_event_id() != event.event_id
    {
        return Err(RecoveryError::InvalidEventRecord);
    }
    Ok(())
}

fn projected_proposal(
    memory: &MemoryProjection,
    resolution: &MemoryProposalResolution,
) -> Result<
    (
        MemoryNamespaceId,
        NormalizedMemoryKey,
        ExpectedMemoryEntryState,
    ),
    RecoveryError,
> {
    let projected = memory
        .proposals()
        .find(|(id, _)| **id == resolution.proposal().proposal_id())
        .map(|(_, projected)| projected)
        .ok_or(RecoveryError::InvalidEventRecord)?;
    if projected.proposal() != resolution.proposal()
        || projected.approval_id() != resolution.approval_id()
        || projected.status() != MemoryProposalStatus::Pending
        || projected.resolution_event_id().is_some()
    {
        return Err(RecoveryError::InvalidEventRecord);
    }
    Ok((
        projected.namespace_id(),
        projected.normalized_key().clone(),
        projected.expected().clone(),
    ))
}

fn apply_expirations(
    memory: &mut MemoryProjection,
    event: &EventEnvelope,
    entry: &MemoryEntryVersion,
    resolutions: &[MemoryProposalResolution],
    selected: Option<crate::domain::MemoryProposalId>,
) -> Result<(), RecoveryError> {
    let reference = entry.reference();
    let after = current_expected(memory, reference.namespace_id(), reference.normalized_key());
    let expected_ids = memory
        .proposals()
        .filter(|(id, projected)| {
            Some(**id) != selected
                && projected.namespace_id() == reference.namespace_id()
                && projected.normalized_key() == reference.normalized_key()
                && projected.status() == MemoryProposalStatus::Pending
                && projected.expected() != &after
        })
        .map(|(id, _)| *id)
        .collect::<Vec<_>>();
    if resolutions
        .iter()
        .map(|resolution| resolution.proposal().proposal_id())
        .collect::<Vec<_>>()
        != expected_ids
    {
        return Err(RecoveryError::InvalidEventRecord);
    }
    let mut previous = None;
    for resolution in resolutions {
        validate_terminal_resolution(event, resolution, MemoryProposalStatus::Expired)?;
        let proposal_id = resolution.proposal().proposal_id();
        if previous.is_some_and(|previous| proposal_id <= previous) || selected == Some(proposal_id)
        {
            return Err(RecoveryError::InvalidEventRecord);
        }
        let projected = projected_proposal(memory, resolution)?;
        if projected.0 != reference.namespace_id()
            || projected.1 != *reference.normalized_key()
            || projected.2 == after
        {
            return Err(RecoveryError::InvalidEventRecord);
        }
        previous = Some(proposal_id);
    }
    for resolution in resolutions {
        memory
            .resolve_proposal(resolution)
            .map_err(|_| RecoveryError::InvalidEventRecord)?;
    }
    Ok(())
}

fn current_expected(
    memory: &MemoryProjection,
    namespace: MemoryNamespaceId,
    key: &NormalizedMemoryKey,
) -> ExpectedMemoryEntryState {
    match memory.current_entry(namespace, key).cloned() {
        None => ExpectedMemoryEntryState::Absent,
        Some(reference) if reference.state() == MemoryEntryState::Present => {
            ExpectedMemoryEntryState::Present(reference)
        }
        Some(reference) => ExpectedMemoryEntryState::Deleted(reference),
    }
}

fn validate_memory_read_event(
    state: &ProjectionState,
    event: &EventEnvelope,
) -> Result<(), RecoveryError> {
    if event.actor != Actor::Human || event.object.is_some() {
        return Err(RecoveryError::InvalidEventRecord);
    }
    match &event.event {
        ApplicationEvent::MemoryEntriesListed {
            profile,
            namespace_id,
            entries,
            total_count,
            returned_count,
            omitted_count,
            ..
        } => {
            validate_counts(entries.len(), *total_count, *returned_count, *omitted_count)?;
            let profile = state
                .agent_profiles
                .resolve_reference(profile)
                .map_err(|_| RecoveryError::InvalidEventRecord)?;
            let projected_total = state
                .memory
                .current_entries()
                .filter(|((namespace, _), entry)| {
                    *namespace == *namespace_id && entry.state() == MemoryEntryState::Present
                })
                .count();
            if profile.memory_namespace_id() != *namespace_id
                || u64::try_from(projected_total).ok() != Some(*total_count)
                || entries.iter().any(|entry| {
                    entry.namespace_id() != *namespace_id
                        || entry.state() != MemoryEntryState::Present
                        || state
                            .memory
                            .current_entry(entry.namespace_id(), entry.normalized_key())
                            != Some(entry)
                })
                || !unique(entries.iter().map(MemoryEntryRef::entry_version_id))
            {
                return Err(RecoveryError::InvalidEventRecord);
            }
        }
        ApplicationEvent::MemoryEntryShown { profile, entry } => {
            let profile = state
                .agent_profiles
                .resolve_reference(profile)
                .map_err(|_| RecoveryError::InvalidEventRecord)?;
            if profile.memory_namespace_id() != entry.namespace_id()
                || entry.state() != MemoryEntryState::Present
                || state
                    .memory
                    .current_entry(entry.namespace_id(), entry.normalized_key())
                    != Some(entry)
            {
                return Err(RecoveryError::InvalidEventRecord);
            }
        }
        ApplicationEvent::MemoryEntryVersionShown { profile, entry } => {
            let profile = state
                .agent_profiles
                .resolve_reference(profile)
                .map_err(|_| RecoveryError::InvalidEventRecord)?;
            if profile.memory_namespace_id() != entry.namespace_id() {
                return Err(RecoveryError::InvalidEventRecord);
            }
        }
        ApplicationEvent::MemoryEntryHistoryShown {
            profile,
            current,
            versions,
            total_count,
            returned_count,
            omitted_count,
            ..
        } => {
            validate_counts(
                versions.len(),
                *total_count,
                *returned_count,
                *omitted_count,
            )?;
            let profile = state
                .agent_profiles
                .resolve_reference(profile)
                .map_err(|_| RecoveryError::InvalidEventRecord)?;
            if profile.memory_namespace_id() != current.namespace_id()
                || state
                    .memory
                    .current_entry(current.namespace_id(), current.normalized_key())
                    != Some(current)
                || versions.first() != Some(current)
                || !unique(versions.iter().map(MemoryEntryRef::entry_version_id))
                || versions.iter().any(|version| {
                    version.namespace_id() != current.namespace_id()
                        || version.entry_id() != current.entry_id()
                        || version.normalized_key() != current.normalized_key()
                })
                || versions
                    .windows(2)
                    .any(|window| window[0].version() <= window[1].version())
            {
                return Err(RecoveryError::InvalidEventRecord);
            }
        }
        ApplicationEvent::MemoryProposalsListed {
            profile,
            filter,
            proposals,
            total_count,
            returned_count,
            omitted_count,
            ..
        } => {
            validate_counts(
                proposals.len(),
                *total_count,
                *returned_count,
                *omitted_count,
            )?;
            let profile = state
                .agent_profiles
                .resolve_reference(profile)
                .map_err(|_| RecoveryError::InvalidEventRecord)?;
            let projected_total = state
                .memory
                .proposals()
                .filter(|(_, proposal)| {
                    proposal.namespace_id() == profile.memory_namespace_id()
                        && (*filter == crate::memory::MemoryProposalFilter::All
                            || proposal.status() == MemoryProposalStatus::Pending)
                })
                .count();
            if u64::try_from(projected_total).ok() != Some(*total_count)
                || !unique(
                    proposals
                        .iter()
                        .map(|proposal| proposal.proposal.proposal_id()),
                )
                || proposals.iter().any(|status| {
                    (*filter == crate::memory::MemoryProposalFilter::Pending
                        && status.status != MemoryProposalStatus::Pending)
                        || state
                            .memory
                            .proposals()
                            .find(|(id, _)| **id == status.proposal.proposal_id())
                            .is_none_or(|(_, proposal)| {
                                proposal.proposal() != &status.proposal
                                    || proposal.status() != status.status
                                    || proposal.namespace_id() != profile.memory_namespace_id()
                            })
                })
            {
                return Err(RecoveryError::InvalidEventRecord);
            }
        }
        ApplicationEvent::MemoryProposalShown {
            proposal,
            status,
            resolution,
        } => {
            let projected = state
                .memory
                .proposals()
                .find(|(id, _)| **id == proposal.proposal_id())
                .map(|(_, projected)| projected)
                .ok_or(RecoveryError::InvalidEventRecord)?;
            if projected.proposal() != proposal || projected.status() != *status {
                return Err(RecoveryError::InvalidEventRecord);
            }
            match (status, resolution) {
                (MemoryProposalStatus::Pending, None)
                    if projected.resolution_event_id().is_none() => {}
                (MemoryProposalStatus::Pending, Some(_)) => {
                    return Err(RecoveryError::InvalidEventRecord);
                }
                (_, Some(resolution))
                    if resolution.proposal() == proposal
                        && resolution.status() == *status
                        && projected.resolution_event_id()
                            == Some(resolution.resolution_event_id()) => {}
                _ => return Err(RecoveryError::InvalidEventRecord),
            }
        }
        ApplicationEvent::EpisodicSummariesListed {
            profile,
            summaries,
            total_count,
            returned_count,
            omitted_count,
        } => {
            validate_counts(
                summaries.len(),
                *total_count,
                *returned_count,
                *omitted_count,
            )?;
            let profile_record = state
                .agent_profiles
                .resolve_reference(profile)
                .map_err(|_| RecoveryError::InvalidEventRecord)?;
            if summaries.iter().any(|summary| {
                summary.profile() != profile
                    || summary.namespace_id() != profile_record.memory_namespace_id()
            }) || !unique(summaries.iter().map(|summary| summary.summary_id()))
            {
                return Err(RecoveryError::InvalidEventRecord);
            }
        }
        ApplicationEvent::EpisodicSummaryShown { summary } => {
            let profile = state
                .agent_profiles
                .resolve_reference(summary.profile())
                .map_err(|_| RecoveryError::InvalidEventRecord)?;
            if profile.memory_namespace_id() != summary.namespace_id() {
                return Err(RecoveryError::InvalidEventRecord);
            }
        }
        ApplicationEvent::MemorySnapshotBuilt { metadata } => {
            let profile = state
                .agent_profiles
                .resolve_reference(metadata.scope().profile())
                .map_err(|_| RecoveryError::InvalidEventRecord)?;
            metadata
                .scope()
                .validate_against(profile)
                .map_err(|_| RecoveryError::InvalidEventRecord)?;
        }
        _ => return Err(RecoveryError::InvalidEventRecord),
    }
    Ok(())
}

fn validate_counts(
    len: usize,
    total: u64,
    returned: u64,
    omitted: u64,
) -> Result<(), RecoveryError> {
    if u64::try_from(len).ok() != Some(returned) || returned.checked_add(omitted) != Some(total) {
        return Err(RecoveryError::InvalidEventRecord);
    }
    Ok(())
}

fn unique<T: Ord>(items: impl Iterator<Item = T>) -> bool {
    let mut seen = BTreeSet::new();
    items.into_iter().all(|item| seen.insert(item))
}

fn end_session(
    state: &mut ProjectionState,
    session_id: SessionId,
    event: &EventEnvelope,
    reason: ShutdownReason,
) -> Result<(), RecoveryError> {
    let open_sessions = state
        .sessions
        .values()
        .filter(|session| session.ended.is_none())
        .map(|session| session.session_id)
        .collect::<Vec<_>>();
    if open_sessions.as_slice() != [session_id] {
        return Err(RecoveryError::InvalidEventRecord);
    }
    let session = state
        .sessions
        .get_mut(&session_id)
        .ok_or(RecoveryError::InvalidEventRecord)?;
    session.ended = Some(SessionEndProjection {
        ended_event_id: event.event_id,
        ended_at_ms: event.occurred_at_ms,
        reason,
    });
    Ok(())
}

fn authenticated_skill_record_digest(
    event: &EventEnvelope,
    skill: &SkillVersionRef,
) -> Result<Sha256Digest, RecoveryError> {
    let object = event
        .object
        .as_ref()
        .ok_or(RecoveryError::InvalidEventRecord)?;
    if object.kind != "skill_version"
        || object.id != skill.skill_version_id().to_string()
        || object.version != skill.version()
    {
        return Err(RecoveryError::InvalidEventRecord);
    }
    Ok(object.digest.clone())
}

fn is_canonical_builtin_successor(
    skill: &SkillVersionRef,
    previous_version_id: SkillVersionId,
    provenance: &SkillProvenance,
) -> Result<bool, RecoveryError> {
    let manifests =
        crate::skills::builtin_manifests().map_err(|_| RecoveryError::InvalidEventRecord)?;
    Ok(manifests.iter().any(|manifest| {
        let previous = manifest.skill();
        previous.skill_id() == skill.skill_id()
            && previous.skill_version_id() == previous_version_id
            && previous.version().get().checked_add(1) == Some(skill.version().get())
            && previous.provenance() == provenance
    }))
}

fn collides_with_canonical_builtin_identity(
    skill: &SkillVersionRef,
) -> Result<bool, RecoveryError> {
    let manifests =
        crate::skills::builtin_manifests().map_err(|_| RecoveryError::InvalidEventRecord)?;
    Ok(manifests.iter().any(|manifest| {
        let builtin = manifest.skill();
        builtin.skill_id() == skill.skill_id()
            || builtin.skill_version_id() == skill.skill_version_id()
    }))
}
