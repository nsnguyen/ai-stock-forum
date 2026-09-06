//! Presentation-local, keyboard-first state machine for declarative skills.

use crate::{
    app::ApplicationCommand,
    domain::{DomainError, SkillId, SkillVersionId},
    skills::{
        DISPLAY_NAME_MAX_BYTES, SkillDraft, SkillEditPreview, SkillResource,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillEditorMode {
    Create,
    Version {
        skill_id: SkillId,
        expected_active_version_id: SkillVersionId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillEditorStep {
    Identity,
    Usage,
    Instructions,
    References,
    Review,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillEditorField {
    DisplayName,
    Purpose,
    UseWhen,
    Tags,
    Instructions,
    ReferenceName,
    ReferenceBody,
    Review,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillPreviewRequest {
    Create {
        generation: u64,
        candidate: SkillDraft,
    },
    Version {
        generation: u64,
        skill_id: SkillId,
        expected_active_version_id: SkillVersionId,
        candidate: SkillDraft,
    },
}

impl SkillPreviewRequest {
    pub fn generation(&self) -> u64 {
        match self {
            Self::Create { generation, .. } | Self::Version { generation, .. } => *generation,
        }
    }

    pub fn candidate(&self) -> &SkillDraft {
        match self {
            Self::Create { candidate, .. } | Self::Version { candidate, .. } => candidate,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillEditorEffect {
    None,
    Preview(SkillPreviewRequest),
    Execute(ApplicationCommand),
    CancelReview,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillEditorError {
    field: SkillEditorField,
    code: &'static str,
}

impl SkillEditorError {
    pub fn field(&self) -> SkillEditorField {
        self.field
    }

    pub fn code(&self) -> &'static str {
        self.code
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillEditorReview {
    preview: SkillEditPreview,
}

impl SkillEditorReview {
    pub fn preview(&self) -> &SkillEditPreview {
        &self.preview
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct RawSkillDraft {
    display_name: String,
    description: String,
    use_when: String,
    tags: String,
    instructions: String,
    resources: Vec<SkillResource>,
}

impl From<SkillDraft> for RawSkillDraft {
    fn from(draft: SkillDraft) -> Self {
        Self {
            display_name: draft.display_name,
            description: draft.description,
            use_when: draft.use_when,
            tags: draft.tags.join(", "),
            instructions: draft.instructions,
            resources: draft.resources,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillEditor {
    mode: SkillEditorMode,
    step: SkillEditorStep,
    field: SkillEditorField,
    raw: RawSkillDraft,
    pending_reference_name: Option<String>,
    review: Option<SkillEditorReview>,
    local_error: Option<SkillEditorError>,
    preview_generation: u64,
    pending_preview_generation: Option<u64>,
}

impl SkillEditor {
    pub fn for_create(seed: Option<SkillDraft>) -> Self {
        Self::new(SkillEditorMode::Create, seed.unwrap_or_else(empty_draft))
    }

    pub fn for_version(
        skill_id: SkillId,
        expected_active_version_id: SkillVersionId,
        draft: SkillDraft,
    ) -> Self {
        Self::new(
            SkillEditorMode::Version {
                skill_id,
                expected_active_version_id,
            },
            draft,
        )
    }

    fn new(mode: SkillEditorMode, draft: SkillDraft) -> Self {
        Self {
            mode,
            step: SkillEditorStep::Identity,
            field: SkillEditorField::DisplayName,
            raw: draft.into(),
            pending_reference_name: None,
            review: None,
            local_error: None,
            preview_generation: 0,
            pending_preview_generation: None,
        }
    }

    pub fn mode(&self) -> &SkillEditorMode {
        &self.mode
    }

    pub fn step(&self) -> SkillEditorStep {
        self.step
    }

    pub fn field(&self) -> SkillEditorField {
        self.field
    }

    pub fn raw_display_name(&self) -> &str {
        &self.raw.display_name
    }

    pub fn pending_reference_name(&self) -> Option<&str> {
        self.pending_reference_name.as_deref()
    }

    pub fn draft(&self) -> SkillDraft {
        self.try_draft().expect("editor exposes a draft after validation")
    }

    pub fn try_draft(&self) -> Result<SkillDraft, DomainError> {
        SkillDraft::new(
            self.raw.display_name.clone(),
            self.raw.description.clone(),
            self.raw.use_when.clone(),
            parse_tags(&self.raw.tags),
            self.raw.instructions.clone(),
            self.raw.resources.clone(),
        )
    }

    pub fn review(&self) -> Option<&SkillEditorReview> {
        self.review.as_ref()
    }

    pub fn local_error(&self) -> Option<&SkillEditorError> {
        self.local_error.as_ref()
    }

    pub fn report_error(&mut self, code: &'static str) {
        self.local_error = Some(SkillEditorError {
            field: self.field,
            code,
        });
    }

    pub fn clear_review(&mut self) {
        self.review = None;
        self.pending_preview_generation = None;
    }

    pub fn go_to_review(&mut self) -> Result<(), DomainError> {
        self.try_draft()?;
        self.step = SkillEditorStep::Review;
        self.field = SkillEditorField::Review;
        self.local_error = None;
        Ok(())
    }

    pub fn submit_keyboard_line(&mut self, input: &str) -> SkillEditorEffect {
        self.local_error = None;
        match self.field {
            SkillEditorField::DisplayName => {
                self.raw.display_name = input.to_owned();
                if input.is_empty() || input.len() > DISPLAY_NAME_MAX_BYTES || self.probe().is_err() {
                    return self.invalid(SkillEditorField::DisplayName, "skill_display_name_invalid");
                }
                self.field = SkillEditorField::Purpose;
            }
            SkillEditorField::Purpose => {
                self.raw.description = input.to_owned();
                if self.probe().is_err() {
                    return self.invalid(SkillEditorField::Purpose, "skill_purpose_invalid");
                }
                self.step = SkillEditorStep::Usage;
                self.field = SkillEditorField::UseWhen;
            }
            SkillEditorField::UseWhen => {
                self.raw.use_when = input.to_owned();
                if self.probe().is_err() {
                    return self.invalid(SkillEditorField::UseWhen, "skill_use_when_invalid");
                }
                self.field = SkillEditorField::Tags;
            }
            SkillEditorField::Tags => {
                self.raw.tags = input.to_owned();
                if self.probe().is_err() {
                    return self.invalid(SkillEditorField::Tags, "skill_tags_invalid");
                }
                self.step = SkillEditorStep::Instructions;
                self.field = SkillEditorField::Instructions;
            }
            SkillEditorField::Instructions => {
                self.raw.instructions = input.to_owned();
                if self.probe().is_err() {
                    return self.invalid(SkillEditorField::Instructions, "skill_instructions_invalid");
                }
                self.step = SkillEditorStep::References;
                self.field = SkillEditorField::ReferenceName;
            }
            SkillEditorField::ReferenceName if input.is_empty() => {
                if self.go_to_review().is_err() {
                    return self.invalid(SkillEditorField::ReferenceName, "skill_references_invalid");
                }
            }
            SkillEditorField::ReferenceName => {
                self.pending_reference_name = Some(input.to_owned());
                self.field = SkillEditorField::ReferenceBody;
            }
            SkillEditorField::ReferenceBody => {
                let name = self.pending_reference_name.take().unwrap_or_default();
                self.raw.resources.push(SkillResource {
                    name: name.clone(),
                    body: input.to_owned(),
                });
                if self.probe().is_err() {
                    self.raw.resources.pop();
                    self.pending_reference_name = Some(name);
                    return self.invalid(SkillEditorField::ReferenceBody, "skill_reference_invalid");
                }
                self.field = SkillEditorField::ReferenceName;
            }
            SkillEditorField::Review => return self.submit_review(),
        }
        SkillEditorEffect::None
    }

    pub fn back(&mut self) -> SkillEditorEffect {
        self.local_error = None;
        let had_review = self.review.take().is_some() || self.pending_preview_generation.take().is_some();
        match self.field {
            SkillEditorField::DisplayName => return SkillEditorEffect::Cancelled,
            SkillEditorField::Purpose => self.field = SkillEditorField::DisplayName,
            SkillEditorField::UseWhen => {
                self.step = SkillEditorStep::Identity;
                self.field = SkillEditorField::Purpose;
            }
            SkillEditorField::Tags => self.field = SkillEditorField::UseWhen,
            SkillEditorField::Instructions => {
                self.step = SkillEditorStep::Usage;
                self.field = SkillEditorField::Tags;
            }
            SkillEditorField::ReferenceName => {
                self.step = SkillEditorStep::Instructions;
                self.field = SkillEditorField::Instructions;
            }
            SkillEditorField::ReferenceBody => self.field = SkillEditorField::ReferenceName,
            SkillEditorField::Review => {
                self.step = SkillEditorStep::References;
                self.field = SkillEditorField::ReferenceName;
            }
        }
        if had_review {
            SkillEditorEffect::CancelReview
        } else {
            SkillEditorEffect::None
        }
    }

    pub fn apply_preview(&mut self, generation: u64, preview: SkillEditPreview) -> bool {
        if self.pending_preview_generation != Some(generation) || !self.preview_matches_mode(&preview) {
            return false;
        }
        self.pending_preview_generation = None;
        self.review = Some(SkillEditorReview { preview });
        self.local_error = None;
        true
    }

    fn submit_review(&mut self) -> SkillEditorEffect {
        let Ok(candidate) = self.try_draft() else {
            return self.invalid(SkillEditorField::Review, "skill_draft_invalid");
        };
        if let Some(review) = self.review.as_ref() {
            let preview = &review.preview;
            return SkillEditorEffect::Execute(match self.mode {
                SkillEditorMode::Create => ApplicationCommand::CreateSkill {
                    skill_id: preview.skill_id,
                    candidate,
                    review_token: preview.review_token,
                    review_digest: preview.review_digest.clone(),
                },
                SkillEditorMode::Version {
                    skill_id,
                    expected_active_version_id,
                } => ApplicationCommand::ActivateSkillVersion {
                    skill_id,
                    expected_active_version_id,
                    candidate,
                    review_token: preview.review_token,
                    review_digest: preview.review_digest.clone(),
                },
            });
        }
        self.preview_generation = self.preview_generation.saturating_add(1);
        let generation = self.preview_generation;
        self.pending_preview_generation = Some(generation);
        SkillEditorEffect::Preview(match self.mode {
            SkillEditorMode::Create => SkillPreviewRequest::Create {
                generation,
                candidate,
            },
            SkillEditorMode::Version {
                skill_id,
                expected_active_version_id,
            } => SkillPreviewRequest::Version {
                generation,
                skill_id,
                expected_active_version_id,
                candidate,
            },
        })
    }

    fn preview_matches_mode(&self, preview: &SkillEditPreview) -> bool {
        match self.mode {
            SkillEditorMode::Create => preview.expected_active_version_id.is_none(),
            SkillEditorMode::Version {
                skill_id,
                expected_active_version_id,
            } => {
                preview.skill_id == skill_id
                    && preview.expected_active_version_id == Some(expected_active_version_id)
            }
        }
    }

    fn probe(&self) -> Result<SkillDraft, DomainError> {
        SkillDraft::new(
            if self.raw.display_name.is_empty() {
                "Draft skill".to_owned()
            } else {
                self.raw.display_name.clone()
            },
            self.raw.description.clone(),
            if self.raw.use_when.is_empty() {
                "Use when appropriate.".to_owned()
            } else {
                self.raw.use_when.clone()
            },
            parse_tags(&self.raw.tags),
            if self.raw.instructions.is_empty() {
                "Follow these instructions.".to_owned()
            } else {
                self.raw.instructions.clone()
            },
            self.raw.resources.clone(),
        )
    }

    fn invalid(&mut self, field: SkillEditorField, code: &'static str) -> SkillEditorEffect {
        self.local_error = Some(SkillEditorError { field, code });
        SkillEditorEffect::None
    }
}

fn empty_draft() -> SkillDraft {
    SkillDraft {
        display_name: String::new(),
        description: String::new(),
        use_when: String::new(),
        tags: Vec::new(),
        instructions: String::new(),
        resources: Vec::new(),
    }
}

fn parse_tags(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(str::to_owned)
        .collect()
}
