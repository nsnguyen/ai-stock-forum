use serde::{Deserialize, Serialize};

use crate::{
    agents::{AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentRole},
    domain::DomainError,
    skills::SkillVersionRef,
};

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileDiffField {
    DisplayName,
    Description,
    Role,
    PrimarySpecialty,
    SpecialtyTags,
    Personality,
    Instructions,
    Bindings,
    SkillRefsAdded,
    SkillRefsUpgraded,
    SkillRefsRemoved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileFieldValue {
    Text(String),
    Role(AgentRole),
    SpecialtyTags(Vec<String>),
    Bindings(AgentBindings),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileFieldDiff {
    pub field: ProfileDiffField,
    pub before: ProfileFieldValue,
    pub after: ProfileFieldValue,
}

pub fn diff_profile(
    current: &AgentProfileVersion,
    candidate: &AgentProfileDraft,
) -> Result<Vec<ProfileFieldDiff>, DomainError> {
    let candidate = candidate.canonicalized()?;
    let mut changes = Vec::new();

    push_text_change(
        &mut changes,
        ProfileDiffField::DisplayName,
        current.display_name(),
        &candidate.display_name,
    );
    push_text_change(
        &mut changes,
        ProfileDiffField::Description,
        current.description(),
        &candidate.description,
    );
    if current.role() != candidate.role {
        changes.push(ProfileFieldDiff {
            field: ProfileDiffField::Role,
            before: ProfileFieldValue::Role(current.role()),
            after: ProfileFieldValue::Role(candidate.role),
        });
    }
    push_text_change(
        &mut changes,
        ProfileDiffField::PrimarySpecialty,
        current.primary_specialty(),
        &candidate.primary_specialty,
    );
    if current.specialty_tags() != candidate.specialty_tags {
        changes.push(ProfileFieldDiff {
            field: ProfileDiffField::SpecialtyTags,
            before: ProfileFieldValue::SpecialtyTags(current.specialty_tags().to_vec()),
            after: ProfileFieldValue::SpecialtyTags(candidate.specialty_tags.clone()),
        });
    }
    push_text_change(
        &mut changes,
        ProfileDiffField::Personality,
        current.personality(),
        &candidate.personality,
    );
    push_text_change(
        &mut changes,
        ProfileDiffField::Instructions,
        current.instructions(),
        &candidate.instructions,
    );
    if current.bindings() != &candidate.bindings {
        changes.push(ProfileFieldDiff {
            field: ProfileDiffField::Bindings,
            before: ProfileFieldValue::Bindings(current.bindings().clone()),
            after: ProfileFieldValue::Bindings(candidate.bindings.clone()),
        });
    }
    push_skill_ref_changes(&mut changes, current.skill_refs(), &candidate.skill_refs);

    if changes.is_empty() {
        Err(DomainError::AgentProfileUnchanged)
    } else {
        Ok(changes)
    }
}

fn push_skill_ref_changes(
    changes: &mut Vec<ProfileFieldDiff>,
    current: &[SkillVersionRef],
    candidate: &[SkillVersionRef],
) {
    for candidate_ref in candidate {
        match current
            .iter()
            .find(|current_ref| current_ref.skill_id() == candidate_ref.skill_id())
        {
            None => changes.push(ProfileFieldDiff {
                field: ProfileDiffField::SkillRefsAdded,
                before: ProfileFieldValue::Text(String::new()),
                after: ProfileFieldValue::Text(skill_ref_summary(candidate_ref)),
            }),
            Some(current_ref) if current_ref != candidate_ref => changes.push(ProfileFieldDiff {
                field: ProfileDiffField::SkillRefsUpgraded,
                before: ProfileFieldValue::Text(skill_ref_summary(current_ref)),
                after: ProfileFieldValue::Text(skill_ref_summary(candidate_ref)),
            }),
            Some(_) => {}
        }
    }
    for current_ref in current {
        if !candidate
            .iter()
            .any(|candidate_ref| candidate_ref.skill_id() == current_ref.skill_id())
        {
            changes.push(ProfileFieldDiff {
                field: ProfileDiffField::SkillRefsRemoved,
                before: ProfileFieldValue::Text(skill_ref_summary(current_ref)),
                after: ProfileFieldValue::Text(String::new()),
            });
        }
    }
}

fn skill_ref_summary(skill_ref: &SkillVersionRef) -> String {
    format!(
        "{}@{}#{}",
        skill_ref.skill_id(),
        skill_ref.skill_version_id(),
        skill_ref.version().get(),
    )
}

fn push_text_change(
    changes: &mut Vec<ProfileFieldDiff>,
    field: ProfileDiffField,
    before: &str,
    after: &str,
) {
    if before != after {
        changes.push(ProfileFieldDiff {
            field,
            before: ProfileFieldValue::Text(before.to_owned()),
            after: ProfileFieldValue::Text(after.to_owned()),
        });
    }
}
