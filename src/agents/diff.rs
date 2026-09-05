use serde::{Deserialize, Serialize};

use crate::{
    agents::{AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentRole},
    domain::DomainError,
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

    if changes.is_empty() {
        Err(DomainError::AgentProfileUnchanged)
    } else {
        Ok(changes)
    }
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
