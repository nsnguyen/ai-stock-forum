use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize};

use crate::{
    agents::AgentProfilesProjection,
    app::{ApplicationEvent, EVENT_SCHEMA_VERSION, EventEnvelope, ShutdownReason},
    domain::{
        EventId, InstallationId, ObjectVersion, SessionId, Sha256Digest, SkillId,
        SkillVersionId, canonical_json_bytes, sha256,
    },
    persistence::RecoveryError,
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
        ),
    > {
        self.versions_by_id.values().map(|version| {
            (
                &version.skill,
                version.display_name.as_str(),
                &version.provenance,
                version.predecessor_version_id,
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
    ) -> Result<(), RecoveryError> {
        if skill.version().get() != 1
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
        } else if skill.version().get() <= 1 {
            return Err(RecoveryError::InvalidEventRecord);
        }
        self.versions_by_id.insert(
            skill.skill_version_id(),
            ProjectedSkillVersion {
                skill: skill.clone(),
                display_name: display_name.to_owned(),
                provenance: provenance.clone(),
                predecessor_version_id: Some(previous_version_id),
            },
        );
        self.active_by_skill
            .insert(skill.skill_id(), skill.skill_version_id());
        Ok(())
    }

    fn validate(&self) -> Result<(), RecoveryError> {
        let mut by_skill = BTreeMap::<
            SkillId,
            BTreeMap<ObjectVersion, &ProjectedSkillVersion>,
        >::new();
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
                        || version.predecessor_version_id
                            != Some(previous.skill.skill_version_id())
                        || version.provenance != previous.provenance
                    {
                        return Err(RecoveryError::InvalidEventRecord);
                    }
                } else if version.skill.version().get() == 1
                    && version.predecessor_version_id.is_some()
                {
                    return Err(RecoveryError::InvalidEventRecord);
                } else if version.skill.version().get() > 1
                    && version.predecessor_version_id.is_none()
                {
                    return Err(RecoveryError::InvalidEventRecord);
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
        } => next.skills.create(skill, display_name, provenance)?,
        ApplicationEvent::SkillVersionActivated {
            skill,
            previous_version_id,
            display_name,
            provenance,
        } => next
            .skills
            .activate(skill, *previous_version_id, display_name, provenance)?,
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
    }
    next.agent_profiles.reduce(&event.event)?;
    next.last_sequence = event.sequence;
    next.last_event_digest = Some(event.event_digest.clone());
    next.validate()?;
    *state = next;
    Ok(effect)
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
