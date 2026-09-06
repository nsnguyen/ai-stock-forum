use std::collections::BTreeMap;

use crate::domain::{DomainError, ObjectVersion, SkillId, SkillVersionId};

use super::{NormalizedSkillName, SkillVersion};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillsProjection {
    versions_by_id: BTreeMap<SkillVersionId, SkillVersion>,
    version_index_by_skill: BTreeMap<SkillId, BTreeMap<ObjectVersion, SkillVersionId>>,
    active_by_skill: BTreeMap<SkillId, SkillVersionId>,
    active_name_index: BTreeMap<NormalizedSkillName, SkillId>,
}

impl SkillsProjection {
    pub fn insert(&mut self, skill: &SkillVersion) -> Result<(), DomainError> {
        if skill.version.get() != 1
            || skill.predecessor.is_some()
            || self.versions_by_id.contains_key(&skill.skill_version_id)
            || self.active_by_skill.contains_key(&skill.skill_id)
            || self.version_index_by_skill.contains_key(&skill.skill_id)
            || self.active_name_index.contains_key(&skill.normalized_name()?)
            || skill.recompute_content_digest()? != skill.content_digest
        {
            return Err(DomainError::InvalidSkillVersion);
        }
        self.versions_by_id.insert(skill.skill_version_id, skill.clone());
        self.version_index_by_skill
            .entry(skill.skill_id)
            .or_default()
            .insert(skill.version, skill.skill_version_id);
        self.active_by_skill.insert(skill.skill_id, skill.skill_version_id);
        self.active_name_index
            .insert(skill.normalized_name()?, skill.skill_id);
        Ok(())
    }

    pub fn activate(
        &mut self,
        skill: &SkillVersion,
        previous_version_id: SkillVersionId,
    ) -> Result<(), DomainError> {
        let current_id = *self
            .active_by_skill
            .get(&skill.skill_id)
            .ok_or(DomainError::InvalidSkillVersion)?;
        let current = self
            .versions_by_id
            .get(&current_id)
            .ok_or(DomainError::InvalidSkillVersion)?;
        let expected = current
            .version
            .get()
            .checked_add(1)
            .ok_or(DomainError::InvalidObjectVersion)?;
        let normalized_name = skill.normalized_name()?;
        let name_available = self
            .active_name_index
            .get(&normalized_name)
            .is_none_or(|owner| *owner == skill.skill_id);
        if skill.skill_id != current.skill_id
            || skill.version.get() != expected
            || skill.predecessor != Some(current_id)
            || previous_version_id != current_id
            || self.versions_by_id.contains_key(&skill.skill_version_id)
            || !name_available
            || skill.recompute_content_digest()? != skill.content_digest
        {
            return Err(DomainError::InvalidSkillVersion);
        }
        let previous_name = current.normalized_name()?;
        self.versions_by_id.insert(skill.skill_version_id, skill.clone());
        self.version_index_by_skill
            .entry(skill.skill_id)
            .or_default()
            .insert(skill.version, skill.skill_version_id);
        self.active_by_skill.insert(skill.skill_id, skill.skill_version_id);
        self.active_name_index.remove(&previous_name);
        self.active_name_index.insert(normalized_name, skill.skill_id);
        Ok(())
    }

    pub fn active_skill(&self, skill_id: SkillId) -> Option<&SkillVersion> {
        self.active_by_skill
            .get(&skill_id)
            .and_then(|version_id| self.versions_by_id.get(version_id))
    }

    pub fn history(&self, skill_id: SkillId) -> Vec<SkillVersion> {
        self.version_index_by_skill
            .get(&skill_id)
            .into_iter()
            .flat_map(|versions| versions.values())
            .filter_map(|version_id| self.versions_by_id.get(version_id))
            .cloned()
            .collect()
    }
}
