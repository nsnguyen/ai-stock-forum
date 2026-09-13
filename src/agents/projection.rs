use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    app::ApplicationEvent,
    domain::{
        AgentProfileId, AgentProfileVersionId, DomainError, MemoryNamespaceId, ObjectVersion,
    },
    persistence::RecoveryError,
};

use super::{AgentProfileVersion, AgentProfileVersionRef, NormalizedProfileName};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProfilesProjection {
    versions_by_id: BTreeMap<AgentProfileVersionId, AgentProfileVersion>,
    #[serde(default)]
    version_index_by_profile:
        BTreeMap<AgentProfileId, BTreeMap<ObjectVersion, AgentProfileVersionId>>,
    active_by_profile: BTreeMap<AgentProfileId, AgentProfileVersionId>,
    active_name_index: BTreeMap<NormalizedProfileName, AgentProfileId>,
    #[serde(default)]
    memory_namespace_index: BTreeMap<MemoryNamespaceId, AgentProfileId>,
}

impl AgentProfilesProjection {
    pub fn is_empty(&self) -> bool {
        self.versions_by_id.is_empty()
            && self.version_index_by_profile.is_empty()
            && self.active_by_profile.is_empty()
            && self.active_name_index.is_empty()
            && self.memory_namespace_index.is_empty()
    }

    pub fn active_profiles(&self) -> Vec<AgentProfileVersion> {
        let mut profiles = self
            .active_by_profile
            .values()
            .filter_map(|version_id| self.versions_by_id.get(version_id))
            .cloned()
            .collect::<Vec<_>>();
        profiles.sort_by(|left, right| {
            left.normalized_name()
                .as_str()
                .cmp(right.normalized_name().as_str())
                .then_with(|| left.profile_id().cmp(&right.profile_id()))
        });
        profiles
    }

    pub fn active_profile(&self, profile_id: AgentProfileId) -> Option<&AgentProfileVersion> {
        self.active_by_profile
            .get(&profile_id)
            .and_then(|version_id| self.versions_by_id.get(version_id))
    }

    pub fn active_profile_by_name(
        &self,
        normalized_name: &NormalizedProfileName,
    ) -> Option<&AgentProfileVersion> {
        self.active_name_index
            .get(normalized_name)
            .and_then(|profile_id| self.active_profile(*profile_id))
    }

    pub fn active_profile_count(&self) -> usize {
        self.active_by_profile.len()
    }

    pub fn active_profiles_bounded(&self, limit: usize) -> Vec<AgentProfileVersion> {
        self.active_name_index
            .values()
            .take(limit)
            .filter_map(|profile_id| self.active_profile(*profile_id))
            .cloned()
            .collect()
    }

    pub fn history(&self, profile_id: AgentProfileId) -> Vec<AgentProfileVersion> {
        self.version_index_by_profile
            .get(&profile_id)
            .into_iter()
            .flat_map(|index| index.values())
            .filter_map(|version_id| self.versions_by_id.get(version_id))
            .cloned()
            .collect()
    }

    pub fn version(&self, version_id: AgentProfileVersionId) -> Option<&AgentProfileVersion> {
        self.versions_by_id.get(&version_id)
    }

    pub fn resolve_reference(
        &self,
        reference: &AgentProfileVersionRef,
    ) -> Result<&AgentProfileVersion, DomainError> {
        let profile = self
            .versions_by_id
            .get(&reference.profile_version_id())
            .ok_or(DomainError::InvalidAgentProfileVersionReference)?;
        if profile.profile_id() != reference.profile_id()
            || profile.profile_version_id() != reference.profile_version_id()
            || profile.version() != reference.version()
            || profile.content_digest() != reference.content_digest()
        {
            return Err(DomainError::InvalidAgentProfileVersionReference);
        }
        Ok(profile)
    }

    pub fn profile_version(
        &self,
        profile_id: AgentProfileId,
        version: ObjectVersion,
    ) -> Option<&AgentProfileVersion> {
        self.version_index_by_profile
            .get(&profile_id)
            .and_then(|index| index.get(&version))
            .and_then(|version_id| self.versions_by_id.get(version_id))
    }

    pub fn history_count(&self, profile_id: AgentProfileId) -> usize {
        self.version_index_by_profile
            .get(&profile_id)
            .map_or(0, BTreeMap::len)
    }

    pub fn history_bounded_desc(
        &self,
        profile_id: AgentProfileId,
        limit: usize,
    ) -> Vec<AgentProfileVersion> {
        self.version_index_by_profile
            .get(&profile_id)
            .into_iter()
            .flat_map(|index| index.iter().rev())
            .take(limit)
            .filter_map(|(_, version_id)| self.versions_by_id.get(version_id))
            .cloned()
            .collect()
    }

    pub(crate) fn reduce(&mut self, event: &ApplicationEvent) -> Result<(), RecoveryError> {
        match event {
            ApplicationEvent::AgentProfileCreated { profile } => self.create(profile),
            ApplicationEvent::AgentProfileVersionActivated {
                profile,
                previous_version_id,
            } => self.activate(profile, *previous_version_id),
            _ => Ok(()),
        }
    }

    fn create(&mut self, profile: &AgentProfileVersion) -> Result<(), RecoveryError> {
        self.verify_digest(profile)?;
        if profile.version().get() != 1
            || profile.supersedes().is_some()
            || self
                .versions_by_id
                .contains_key(&profile.profile_version_id())
            || self.active_by_profile.contains_key(&profile.profile_id())
            || self
                .version_index_by_profile
                .contains_key(&profile.profile_id())
            || self
                .active_name_index
                .contains_key(profile.normalized_name())
            || self
                .memory_namespace_index
                .contains_key(&profile.memory_namespace_id())
        {
            return Err(RecoveryError::InvalidEventRecord);
        }
        self.versions_by_id
            .insert(profile.profile_version_id(), profile.clone());
        self.version_index_by_profile
            .entry(profile.profile_id())
            .or_default()
            .insert(profile.version(), profile.profile_version_id());
        self.active_by_profile
            .insert(profile.profile_id(), profile.profile_version_id());
        self.active_name_index
            .insert(profile.normalized_name().clone(), profile.profile_id());
        self.memory_namespace_index
            .insert(profile.memory_namespace_id(), profile.profile_id());
        Ok(())
    }

    fn activate(
        &mut self,
        profile: &AgentProfileVersion,
        previous_version_id: AgentProfileVersionId,
    ) -> Result<(), RecoveryError> {
        self.verify_digest(profile)?;
        if self
            .versions_by_id
            .contains_key(&profile.profile_version_id())
        {
            return Err(RecoveryError::InvalidEventRecord);
        }
        let current_version_id = *self
            .active_by_profile
            .get(&profile.profile_id())
            .ok_or(RecoveryError::InvalidEventRecord)?;
        let current = self
            .versions_by_id
            .get(&current_version_id)
            .cloned()
            .ok_or(RecoveryError::InvalidEventRecord)?;
        let expected_version = current
            .version()
            .get()
            .checked_add(1)
            .ok_or(RecoveryError::InvalidEventRecord)?;
        let name_is_available = self
            .active_name_index
            .get(profile.normalized_name())
            .is_none_or(|owner| *owner == profile.profile_id());
        if profile.version().get() != expected_version
            || previous_version_id != current_version_id
            || profile.supersedes() != Some(current_version_id)
            || profile.profile_id() != current.profile_id()
            || profile.memory_namespace_id() != current.memory_namespace_id()
            || profile.default_policy_ref() != current.default_policy_ref()
            || profile.template_provenance() != current.template_provenance()
            || self
                .memory_namespace_index
                .get(&profile.memory_namespace_id())
                != Some(&profile.profile_id())
            || !name_is_available
        {
            return Err(RecoveryError::InvalidEventRecord);
        }
        self.versions_by_id
            .insert(profile.profile_version_id(), profile.clone());
        self.version_index_by_profile
            .entry(profile.profile_id())
            .or_default()
            .insert(profile.version(), profile.profile_version_id());
        self.active_by_profile
            .insert(profile.profile_id(), profile.profile_version_id());
        self.active_name_index.remove(current.normalized_name());
        self.active_name_index
            .insert(profile.normalized_name().clone(), profile.profile_id());
        Ok(())
    }

    fn verify_digest(&self, profile: &AgentProfileVersion) -> Result<(), RecoveryError> {
        if profile
            .recompute_content_digest()
            .map_err(|_| RecoveryError::InvalidEventRecord)?
            != *profile.content_digest()
        {
            return Err(RecoveryError::InvalidEventRecord);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use crate::{
        agents::{
            AgentBindings, AgentProfileDraft, AgentRole, INSTRUCTIONS_MAX_BYTES,
            PERSONALITY_MAX_BYTES,
        },
        domain::{AgentProfileId, AgentProfileVersionId, MemoryNamespaceId},
    };

    use super::{AgentProfileVersion, AgentProfilesProjection};

    fn large_draft(version: u64) -> AgentProfileDraft {
        AgentProfileDraft::new(
            "Large History Analyst".to_owned(),
            format!("Immutable large version {version}."),
            AgentRole::Custom,
            "equity research".to_owned(),
            vec!["valuation".to_owned()],
            "p".repeat(PERSONALITY_MAX_BYTES),
            "i".repeat(INSTRUCTIONS_MAX_BYTES),
            AgentBindings::default(),
            Vec::new(),
            Vec::new(),
        )
        .expect("large bounded draft")
    }

    #[test]
    fn bounded_history_selects_one_hundred_of_one_hundred_sixty_large_versions() {
        let profile_id = AgentProfileId::from_uuid(Uuid::from_u128(1));
        let first = AgentProfileVersion::create(
            profile_id,
            AgentProfileVersionId::from_uuid(Uuid::from_u128(1_001)),
            MemoryNamespaceId::from_uuid(Uuid::from_u128(2_001)),
            1_800_000_000_001,
            large_draft(1),
            None,
        )
        .expect("first version");
        let mut projection = AgentProfilesProjection::default();
        projection.create(&first).expect("first projection row");
        let mut active = first;
        for version in 2_u64..=160 {
            let next = AgentProfileVersion::next_version(
                &active,
                AgentProfileVersionId::from_uuid(Uuid::from_u128(1_000 + u128::from(version))),
                1_800_000_000_000 + i64::try_from(version).unwrap(),
                large_draft(version),
            )
            .expect("next large version");
            projection
                .activate(&next, active.profile_version_id())
                .expect("activate projected version");
            active = next;
        }

        let bounded = projection.history_bounded_desc(profile_id, 100);
        assert_eq!(projection.history_count(profile_id), 160);
        assert_eq!(bounded.len(), 100);
        assert_eq!(bounded.first().unwrap().version().get(), 160);
        assert_eq!(bounded.last().unwrap().version().get(), 61);
        assert!(
            bounded
                .iter()
                .all(|profile| profile.instructions().len() == INSTRUCTIONS_MAX_BYTES)
        );
    }
}
