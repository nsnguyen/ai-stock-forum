use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    app::ApplicationEvent,
    domain::{AgentProfileId, AgentProfileVersionId},
    persistence::RecoveryError,
};

use super::{AgentProfileVersion, NormalizedProfileName};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProfilesProjection {
    versions_by_id: BTreeMap<AgentProfileVersionId, AgentProfileVersion>,
    active_by_profile: BTreeMap<AgentProfileId, AgentProfileVersionId>,
    active_name_index: BTreeMap<NormalizedProfileName, AgentProfileId>,
}

impl AgentProfilesProjection {
    pub fn is_empty(&self) -> bool {
        self.versions_by_id.is_empty()
            && self.active_by_profile.is_empty()
            && self.active_name_index.is_empty()
    }

    pub fn active_profiles(&self) -> Vec<AgentProfileVersion> {
        let mut profiles = self
            .active_by_profile
            .values()
            .filter_map(|version_id| self.versions_by_id.get(version_id))
            .cloned()
            .collect::<Vec<_>>();
        profiles.sort_by(|left, right| {
            left
                .normalized_name()
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

    pub fn history(&self, profile_id: AgentProfileId) -> Vec<AgentProfileVersion> {
        let mut versions = self
            .versions_by_id
            .values()
            .filter(|profile| profile.profile_id() == profile_id)
            .cloned()
            .collect::<Vec<_>>();
        versions.sort_by(|left, right| {
            left
                .version()
                .get()
                .cmp(&right.version().get())
                .then_with(|| left.profile_version_id().cmp(&right.profile_version_id()))
        });
        versions
    }

    pub fn version(&self, version_id: AgentProfileVersionId) -> Option<&AgentProfileVersion> {
        self.versions_by_id.get(&version_id)
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
            || self.versions_by_id.contains_key(&profile.profile_version_id())
            || self.active_by_profile.contains_key(&profile.profile_id())
            || self
                .versions_by_id
                .values()
                .any(|existing| existing.profile_id() == profile.profile_id())
            || self.active_name_index.contains_key(profile.normalized_name())
        {
            return Err(RecoveryError::InvalidEventRecord);
        }
        self.versions_by_id
            .insert(profile.profile_version_id(), profile.clone());
        self.active_by_profile
            .insert(profile.profile_id(), profile.profile_version_id());
        self.active_name_index
            .insert(profile.normalized_name().clone(), profile.profile_id());
        Ok(())
    }

    fn activate(
        &mut self,
        profile: &AgentProfileVersion,
        previous_version_id: AgentProfileVersionId,
    ) -> Result<(), RecoveryError> {
        self.verify_digest(profile)?;
        if self.versions_by_id.contains_key(&profile.profile_version_id()) {
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
            || !name_is_available
        {
            return Err(RecoveryError::InvalidEventRecord);
        }
        self.versions_by_id
            .insert(profile.profile_version_id(), profile.clone());
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
