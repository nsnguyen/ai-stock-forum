use std::collections::BTreeMap;

use crate::domain::{DomainError, SkillVersionId, canonical_json_bytes};

use super::{SkillVersion, SkillVersionRef};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct RetrievalBudget {
    skill_count: usize,
    accepted_bytes: usize,
    resource_count: usize,
}

impl RetrievalBudget {
    pub fn new(skill_count: usize, accepted_bytes: usize, resource_count: usize) -> Self {
        Self {
            skill_count,
            accepted_bytes,
            resource_count,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillRetrieval {
    skills: Vec<SkillVersion>,
    accepted_bytes: usize,
    resource_count: usize,
    omitted_skill_count: usize,
    omitted_resource_count: usize,
}

impl SkillRetrieval {
    pub fn skills(&self) -> &[SkillVersion] {
        &self.skills
    }

    pub fn accepted_bytes(&self) -> usize {
        self.accepted_bytes
    }

    pub fn resource_count(&self) -> usize {
        self.resource_count
    }

    pub fn omitted_skill_count(&self) -> usize {
        self.omitted_skill_count
    }

    pub fn omitted_resource_count(&self) -> usize {
        self.omitted_resource_count
    }
}

pub fn retrieve_assigned_skills(
    assignments: &[SkillVersionRef],
    available_versions: &[SkillVersion],
    budget: RetrievalBudget,
) -> Result<SkillRetrieval, DomainError> {
    let versions_by_id = index_versions(available_versions)?;
    let mut assignments = assignments.to_vec();
    assignments.sort_by_key(|assignment| {
        (
            assignment.skill_id(),
            assignment.version(),
            assignment.skill_version_id(),
        )
    });

    let resolved = assignments
        .iter()
        .map(|assignment| resolve_exact_version(&versions_by_id, assignment))
        .collect::<Result<Vec<_>, _>>()?;
    let mut retrieval = SkillRetrieval {
        skills: Vec::new(),
        accepted_bytes: 0,
        resource_count: 0,
        omitted_skill_count: 0,
        omitted_resource_count: 0,
    };

    for (index, skill) in resolved.iter().enumerate() {
        let section_bytes = canonical_json_bytes(skill.content())?.len();
        let section_resources = skill.content().resources.len();
        let exceeds_budget = retrieval.skills.len() >= budget.skill_count
            || retrieval
                .accepted_bytes
                .checked_add(section_bytes)
                .is_none_or(|bytes| bytes > budget.accepted_bytes)
            || retrieval
                .resource_count
                .checked_add(section_resources)
                .is_none_or(|count| count > budget.resource_count);
        if exceeds_budget {
            let omitted = &resolved[index..];
            retrieval.omitted_skill_count = omitted.len();
            retrieval.omitted_resource_count = omitted
                .iter()
                .map(|omitted_skill| omitted_skill.content().resources.len())
                .sum();
            return Ok(retrieval);
        }

        retrieval.accepted_bytes += section_bytes;
        retrieval.resource_count += section_resources;
        retrieval.skills.push((*skill).clone());
    }

    Ok(retrieval)
}

fn index_versions(
    available_versions: &[SkillVersion],
) -> Result<BTreeMap<SkillVersionId, &SkillVersion>, DomainError> {
    let mut versions_by_id = BTreeMap::new();
    for version in available_versions {
        if versions_by_id
            .insert(version.skill_version_id(), version)
            .is_some()
        {
            return Err(DomainError::InvalidSkillVersion);
        }
    }
    Ok(versions_by_id)
}

fn resolve_exact_version<'a>(
    versions_by_id: &BTreeMap<SkillVersionId, &'a SkillVersion>,
    assignment: &SkillVersionRef,
) -> Result<&'a SkillVersion, DomainError> {
    let version = versions_by_id
        .get(&assignment.skill_version_id())
        .ok_or(DomainError::InvalidSkillVersion)?;
    if version.reference() != *assignment {
        return Err(DomainError::InvalidSkillVersion);
    }
    Ok(version)
}
