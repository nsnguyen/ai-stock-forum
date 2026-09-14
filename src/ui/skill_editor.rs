//! Presentation-local, keyboard-first state machine for declarative skills.

use crate::{
    app::ApplicationCommand,
    domain::{DomainError, SkillId, SkillVersionId},
    skills::{DISPLAY_NAME_MAX_BYTES, SkillDraft, SkillEditPreview, SkillResource},
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
#[expect(
    clippy::large_enum_variant,
    reason = "editor effects preserve their direct command and preview payload interfaces"
)]
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
    selected_reference: Option<usize>,
    editing_reference: Option<usize>,
    pending_reference_name: Option<String>,
    pending_reference_body: String,
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
            selected_reference: None,
            editing_reference: None,
            pending_reference_name: None,
            pending_reference_body: String::new(),
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

    /// Navigate without changing the draft or invalidating a completed review.
    pub fn select_tui_field(&mut self, field: SkillEditorField) {
        self.field = field;
        self.step = match field {
            SkillEditorField::DisplayName | SkillEditorField::Purpose => SkillEditorStep::Identity,
            SkillEditorField::UseWhen | SkillEditorField::Tags => SkillEditorStep::Usage,
            SkillEditorField::Instructions => SkillEditorStep::Instructions,
            SkillEditorField::ReferenceName | SkillEditorField::ReferenceBody => {
                SkillEditorStep::References
            }
            SkillEditorField::Review => SkillEditorStep::Review,
        };
    }

    /// Keep raw text recoverable while validating independently of other fields.
    pub fn set_tui_field(&mut self, field: SkillEditorField, text: &str) -> bool {
        if field == SkillEditorField::Review {
            return false;
        }
        if self.tui_field_text(field) != text {
            self.clear_review();
        }
        match field {
            SkillEditorField::DisplayName => self.raw.display_name = text.to_owned(),
            SkillEditorField::Purpose => self.raw.description = text.to_owned(),
            SkillEditorField::UseWhen => self.raw.use_when = text.to_owned(),
            SkillEditorField::Tags => self.raw.tags = text.to_owned(),
            SkillEditorField::Instructions => self.raw.instructions = text.to_owned(),
            SkillEditorField::ReferenceName => self.pending_reference_name = Some(text.to_owned()),
            SkillEditorField::ReferenceBody => self.pending_reference_body = text.to_owned(),
            SkillEditorField::Review => unreachable!("review has no text"),
        }
        if self.probe_tui_field(field).is_err() {
            let code = match field {
                SkillEditorField::DisplayName => "skill_display_name_invalid",
                SkillEditorField::Purpose => "skill_purpose_invalid",
                SkillEditorField::UseWhen => "skill_use_when_invalid",
                SkillEditorField::Tags => "skill_tags_invalid",
                SkillEditorField::Instructions => "skill_instructions_invalid",
                _ => "skill_reference_invalid",
            };
            self.invalid(field, code);
            false
        } else {
            self.local_error = None;
            true
        }
    }

    pub fn pending_reference_name(&self) -> Option<&str> {
        self.pending_reference_name.as_deref()
    }

    pub fn references(&self) -> &[SkillResource] {
        &self.raw.resources
    }

    pub fn selected_reference(&self) -> Option<usize> {
        self.selected_reference
    }

    pub fn select_reference(&mut self, forward: bool) {
        let Some(last) = self.raw.resources.len().checked_sub(1) else {
            self.selected_reference = None;
            return;
        };
        self.selected_reference = Some(match (self.selected_reference, forward) {
            (None, true) => 0,
            (None, false) => last,
            (Some(index), true) => index.saturating_add(1).min(last),
            (Some(index), false) => index.saturating_sub(1),
        });
    }

    pub fn clear_reference_selection(&mut self) {
        self.selected_reference = None;
    }

    pub fn has_pending_reference(&self) -> bool {
        self.editing_reference.is_some()
            || self
                .pending_reference_name
                .as_deref()
                .is_some_and(|name| !name.is_empty())
            || !self.pending_reference_body.is_empty()
    }

    /// Refuse to replace an unfinished note; navigation can resume that note.
    pub fn begin_add_reference(&mut self) -> bool {
        if self.has_pending_reference() {
            return false;
        }
        self.selected_reference = None;
        self.pending_reference_name = Some(String::new());
        self.select_tui_field(SkillEditorField::ReferenceName);
        self.local_error = None;
        true
    }

    /// Commit a complete note atomically, retaining raw text when validation fails.
    pub fn commit_tui_reference(&mut self) -> bool {
        let resource = SkillResource {
            name: self.pending_reference_name.clone().unwrap_or_default(),
            body: self.pending_reference_body.clone(),
        };
        let mut resources = self.raw.resources.clone();
        if let Some(index) = self.editing_reference {
            let Some(slot) = resources.get_mut(index) else {
                self.invalid(SkillEditorField::ReferenceName, "skill_reference_invalid");
                return false;
            };
            *slot = resource;
        } else {
            resources.push(resource);
        }
        if reference_probe(resources.clone()).is_err() {
            self.invalid(SkillEditorField::ReferenceBody, "skill_reference_invalid");
            return false;
        }
        self.raw.resources = resources;
        self.pending_reference_name = None;
        self.pending_reference_body.clear();
        self.editing_reference = None;
        self.selected_reference = None;
        self.clear_review();
        self.local_error = None;
        self.select_tui_field(SkillEditorField::ReferenceName);
        true
    }

    pub fn begin_edit_selected_reference(&mut self) -> bool {
        if self.has_pending_reference() {
            return false;
        }
        let Some(index) = self.selected_reference else {
            return false;
        };
        let Some(resource) = self.raw.resources.get(index).cloned() else {
            self.selected_reference = None;
            return false;
        };
        self.selected_reference = None;
        self.editing_reference = Some(index);
        self.pending_reference_name = Some(resource.name);
        self.pending_reference_body = resource.body;
        self.select_tui_field(SkillEditorField::ReferenceName);
        self.local_error = None;
        true
    }

    pub fn remove_selected_reference(&mut self) -> bool {
        if self.has_pending_reference() {
            return false;
        }
        let Some(index) = self.selected_reference else {
            return false;
        };
        if index >= self.raw.resources.len() {
            self.selected_reference = None;
            return false;
        }
        self.raw.resources.remove(index);
        self.selected_reference = self
            .raw
            .resources
            .len()
            .checked_sub(1)
            .map(|last| index.min(last));
        self.review = None;
        self.pending_preview_generation = None;
        self.local_error = None;
        true
    }

    pub fn cancel_reference_interaction(&mut self) -> bool {
        if self.editing_reference.take().is_some() {
            self.pending_reference_name = None;
            self.pending_reference_body.clear();
            self.field = SkillEditorField::ReferenceName;
            self.local_error = None;
            return true;
        }
        self.selected_reference.take().is_some()
    }

    pub fn current_value(&self) -> &str {
        self.tui_field_text(self.field)
    }

    pub fn tui_field_text(&self, field: SkillEditorField) -> &str {
        match field {
            SkillEditorField::DisplayName => &self.raw.display_name,
            SkillEditorField::Purpose => &self.raw.description,
            SkillEditorField::UseWhen => &self.raw.use_when,
            SkillEditorField::Tags => &self.raw.tags,
            SkillEditorField::Instructions => &self.raw.instructions,
            SkillEditorField::ReferenceName => self.pending_reference_name.as_deref().unwrap_or(""),
            SkillEditorField::ReferenceBody => &self.pending_reference_body,
            SkillEditorField::Review => "",
        }
    }

    pub fn draft(&self) -> SkillDraft {
        self.try_draft()
            .expect("editor exposes a draft after validation")
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
        if self.has_pending_reference() {
            self.invalid(SkillEditorField::ReferenceBody, "skill_reference_pending");
            return Err(DomainError::InvalidSkillField {
                field: "resource_name",
            });
        }
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
                if input.is_empty() || input.len() > DISPLAY_NAME_MAX_BYTES || self.probe().is_err()
                {
                    return self
                        .invalid(SkillEditorField::DisplayName, "skill_display_name_invalid");
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
                    return self
                        .invalid(SkillEditorField::Instructions, "skill_instructions_invalid");
                }
                self.step = SkillEditorStep::References;
                self.field = SkillEditorField::ReferenceName;
            }
            SkillEditorField::ReferenceName if input.is_empty() => {
                if self.go_to_review().is_err() {
                    return self
                        .invalid(SkillEditorField::ReferenceName, "skill_references_invalid");
                }
            }
            SkillEditorField::ReferenceName => {
                if self.editing_reference.is_none()
                    && self.pending_reference_name.as_deref() != Some(input)
                {
                    self.pending_reference_body.clear();
                }
                self.pending_reference_name = Some(input.to_owned());
                self.field = SkillEditorField::ReferenceBody;
            }
            SkillEditorField::ReferenceBody => {
                self.pending_reference_body = input.to_owned();
                let name = self.pending_reference_name.take().unwrap_or_default();
                let resource = SkillResource {
                    name: name.clone(),
                    body: self.pending_reference_body.clone(),
                };
                let editing_reference = self.editing_reference.take();
                let previous =
                    editing_reference.and_then(|index| self.raw.resources.get(index).cloned());
                if let Some(index) = editing_reference {
                    if let Some(slot) = self.raw.resources.get_mut(index) {
                        *slot = resource;
                    } else {
                        self.raw.resources.push(resource);
                    }
                } else {
                    self.raw.resources.push(resource);
                }
                if self.probe().is_err() {
                    if let (Some(index), Some(previous)) = (editing_reference, previous) {
                        self.raw.resources[index] = previous;
                    } else {
                        self.raw.resources.pop();
                    }
                    self.editing_reference = editing_reference;
                    self.pending_reference_name = Some(name);
                    return self
                        .invalid(SkillEditorField::ReferenceBody, "skill_reference_invalid");
                }
                self.pending_reference_body.clear();
                self.selected_reference = None;
                self.field = SkillEditorField::ReferenceName;
            }
            SkillEditorField::Review => return self.submit_review(),
        }
        SkillEditorEffect::None
    }

    pub fn back(&mut self) -> SkillEditorEffect {
        self.local_error = None;
        let had_review =
            self.review.take().is_some() || self.pending_preview_generation.take().is_some();
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
        if self.pending_preview_generation != Some(generation)
            || !self.preview_matches_mode(&preview)
        {
            return false;
        }
        self.pending_preview_generation = None;
        self.review = Some(SkillEditorReview { preview });
        self.local_error = None;
        true
    }

    fn submit_review(&mut self) -> SkillEditorEffect {
        if self.has_pending_reference() {
            return self.invalid(SkillEditorField::ReferenceBody, "skill_reference_pending");
        }
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

    fn probe_tui_field(&self, field: SkillEditorField) -> Result<SkillDraft, DomainError> {
        let text = self.tui_field_text(field).to_owned();
        let mut candidate = reference_probe(Vec::new())?;
        match field {
            SkillEditorField::DisplayName => candidate.display_name = text,
            SkillEditorField::Purpose => candidate.description = text,
            SkillEditorField::UseWhen => candidate.use_when = text,
            SkillEditorField::Tags => candidate.tags = parse_tags(&text),
            SkillEditorField::Instructions => candidate.instructions = text,
            SkillEditorField::ReferenceName => {
                candidate.resources.push(SkillResource {
                    name: text,
                    body: String::new(),
                });
            }
            SkillEditorField::ReferenceBody => {
                candidate.resources.push(SkillResource {
                    name: "Reference".to_owned(),
                    body: text,
                });
            }
            SkillEditorField::Review => {}
        }
        candidate.canonicalized()
    }

    fn invalid(&mut self, field: SkillEditorField, code: &'static str) -> SkillEditorEffect {
        self.local_error = Some(SkillEditorError { field, code });
        SkillEditorEffect::None
    }
}

fn reference_probe(resources: Vec<SkillResource>) -> Result<SkillDraft, DomainError> {
    SkillDraft::new(
        "Draft skill".to_owned(),
        String::new(),
        "Use when appropriate.".to_owned(),
        Vec::new(),
        "Follow these instructions.".to_owned(),
        resources,
    )
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
