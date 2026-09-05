//! Presentation-local guided editor for agent profile drafts.
//!
//! This module owns only a draft and its navigation state. Hosts submit the
//! typed effects it emits to the application boundary and return previews via
//! [`ProfileEditor::apply_preview`]. It has no persistence or terminal
//! dependencies, and its safe summaries intentionally exclude profile prose.

use crate::{
    agents::{
        AgentProfileDraft, ProfileEditPreview, ProfileTemplate, ProfileTemplateProvenance,
        normalize_tag_key,
    },
    app::ApplicationCommand,
    domain::{AgentProfileId, AgentProfileVersionId, DomainError},
};

const MAX_SPECIALTY_TAGS: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileEditorMode {
    Create {
        provenance: ProfileTemplateProvenance,
    },
    Edit {
        profile_id: AgentProfileId,
        expected_active_version_id: AgentProfileVersionId,
    },
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ProfileEditorStep {
    Template,
    Identity,
    Specialty,
    Personality,
    Instructions,
    OptionalBindings,
    Review,
}

impl ProfileEditorStep {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Template => "template",
            Self::Identity => "identity",
            Self::Specialty => "specialty",
            Self::Personality => "personality",
            Self::Instructions => "instructions",
            Self::OptionalBindings => "optional_bindings",
            Self::Review => "review",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewEditRequest {
    pub generation: u64,
    pub profile_id: AgentProfileId,
    pub expected_active_version_id: AgentProfileVersionId,
    pub candidate: AgentProfileDraft,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileEditorEffect {
    None,
    PreviewEdit(PreviewEditRequest),
    Execute(ApplicationCommand),
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileEditorReview {
    preview: ProfileEditPreview,
}

impl ProfileEditorReview {
    pub fn preview(&self) -> &ProfileEditPreview {
        &self.preview
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafeUiMessage {
    code: &'static str,
}

impl SafeUiMessage {
    pub const fn code(&self) -> &'static str {
        self.code
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum IdentityField {
    DisplayName,
    Description,
}

#[derive(Debug, Clone)]
pub struct ProfileEditor {
    mode: ProfileEditorMode,
    step: ProfileEditorStep,
    draft: AgentProfileDraft,
    review: Option<ProfileEditorReview>,
    local_message: Option<SafeUiMessage>,
    identity_field: IdentityField,
    personality_started: bool,
    instructions_started: bool,
    preview_generation: u64,
    pending_preview_generation: Option<u64>,
}

impl ProfileEditor {
    pub fn for_create(template: &ProfileTemplate) -> Result<Self, DomainError> {
        Ok(Self {
            mode: ProfileEditorMode::Create {
                provenance: template.provenance(),
            },
            step: ProfileEditorStep::Template,
            draft: template.copy_to_draft()?,
            review: None,
            local_message: None,
            identity_field: IdentityField::DisplayName,
            personality_started: false,
            instructions_started: false,
            preview_generation: 0,
            pending_preview_generation: None,
        })
    }

    pub fn for_edit(
        profile_id: AgentProfileId,
        expected_active_version_id: AgentProfileVersionId,
        draft: AgentProfileDraft,
    ) -> Self {
        Self {
            mode: ProfileEditorMode::Edit {
                profile_id,
                expected_active_version_id,
            },
            step: ProfileEditorStep::Template,
            draft,
            review: None,
            local_message: None,
            identity_field: IdentityField::DisplayName,
            personality_started: false,
            instructions_started: false,
            preview_generation: 0,
            pending_preview_generation: None,
        }
    }

    pub fn mode(&self) -> &ProfileEditorMode {
        &self.mode
    }

    pub const fn step(&self) -> ProfileEditorStep {
        self.step
    }

    pub fn draft(&self) -> &AgentProfileDraft {
        &self.draft
    }

    pub fn review(&self) -> Option<&ProfileEditorReview> {
        self.review.as_ref()
    }

    pub fn local_message(&self) -> Option<&SafeUiMessage> {
        self.local_message.as_ref()
    }

    /// Provides only control state suitable for a host's command history or
    /// diagnostics. Draft values and review diff values are deliberately not
    /// included.
    pub fn control_summary(&self) -> String {
        let mode = match self.mode {
            ProfileEditorMode::Create { .. } => "create",
            ProfileEditorMode::Edit { .. } => "edit",
        };
        format!(
            "profile_editor mode={mode} step={} review={}",
            self.step.as_str(),
            if self.review.is_some() { "ready" } else { "none" }
        )
    }

    /// Accepts the authoritative result of a previously emitted preview
    /// request. Mismatched results are retained nowhere and receive a safe
    /// local error instead.
    pub fn apply_preview(&mut self, generation: u64, preview: ProfileEditPreview) {
        let ProfileEditorMode::Edit {
            profile_id,
            expected_active_version_id,
        } = self.mode
        else {
            self.message("preview_unavailable");
            return;
        };

        if self.pending_preview_generation != Some(generation)
            || self.step != ProfileEditorStep::Review
        {
            self.message("stale_preview");
            return;
        }

        if preview.profile_id != profile_id
            || preview.expected_active_version_id != expected_active_version_id
        {
            self.invalidate_review();
            self.message("preview_mismatch");
            return;
        }

        self.review = Some(ProfileEditorReview { preview });
        self.pending_preview_generation = None;
        self.local_message = None;
    }

    /// Consumes one presentation line. Lines that are not colon controls only
    /// edit the current text field and never enter a command or audit stream.
    pub fn submit_line(&mut self, line: &str) -> ProfileEditorEffect {
        if let Some(control) = line.strip_prefix(':') {
            return self.submit_control(control);
        }
        self.submit_text(line);
        ProfileEditorEffect::None
    }

    fn submit_control(&mut self, control: &str) -> ProfileEditorEffect {
        match control {
            "next" => self.next(),
            "back" => self.back(),
            "show" => {
                self.message("editor_state");
                ProfileEditorEffect::None
            }
            "clear" => {
                self.clear_current();
                ProfileEditorEffect::None
            }
            "review" => self.request_review(),
            "activate" => self.activate(),
            "cancel" => {
                self.invalidate_review();
                self.local_message = None;
                ProfileEditorEffect::Cancelled
            }
            _ if control.starts_with("tag add ") => {
                self.add_tag(&control[8..]);
                ProfileEditorEffect::None
            }
            _ if control.starts_with("tag remove ") => {
                self.remove_tag(&control[11..]);
                ProfileEditorEffect::None
            }
            _ if control.starts_with("provider ") => {
                self.set_provider(&control[9..]);
                ProfileEditorEffect::None
            }
            _ if control.starts_with("model ") => {
                self.set_model(&control[6..]);
                ProfileEditorEffect::None
            }
            _ => {
                self.message("unknown_editor_control");
                ProfileEditorEffect::None
            }
        }
    }

    fn submit_text(&mut self, line: &str) {
        let changed = match self.step {
            ProfileEditorStep::Identity => match self.identity_field {
                IdentityField::DisplayName => replace_text(&mut self.draft.display_name, line),
                IdentityField::Description => replace_text(&mut self.draft.description, line),
            },
            ProfileEditorStep::Specialty => replace_text(&mut self.draft.primary_specialty, line),
            ProfileEditorStep::Personality => append_line(
                &mut self.draft.personality,
                &mut self.personality_started,
                line,
            ),
            ProfileEditorStep::Instructions => append_line(
                &mut self.draft.instructions,
                &mut self.instructions_started,
                line,
            ),
            ProfileEditorStep::Template
            | ProfileEditorStep::OptionalBindings
            | ProfileEditorStep::Review => {
                self.message("editor_field_unavailable");
                false
            }
        };
        if changed {
            self.invalidate_review();
            self.local_message = None;
        }
    }

    fn next(&mut self) -> ProfileEditorEffect {
        match self.step {
            ProfileEditorStep::Template => {
                self.step = ProfileEditorStep::Identity;
                self.identity_field = IdentityField::DisplayName;
            }
            ProfileEditorStep::Identity => match self.identity_field {
                IdentityField::DisplayName => {
                    if self.is_valid() {
                        self.identity_field = IdentityField::Description;
                        self.local_message = None;
                    }
                }
                IdentityField::Description => self.advance_after_validation(ProfileEditorStep::Specialty),
            },
            ProfileEditorStep::Specialty => self.advance_after_validation(ProfileEditorStep::Personality),
            ProfileEditorStep::Personality => self.advance_after_validation(ProfileEditorStep::Instructions),
            ProfileEditorStep::Instructions => {
                self.advance_after_validation(ProfileEditorStep::OptionalBindings)
            }
            ProfileEditorStep::OptionalBindings => self.advance_after_validation(ProfileEditorStep::Review),
            ProfileEditorStep::Review => self.message("editor_last_step"),
        }
        ProfileEditorEffect::None
    }

    fn back(&mut self) -> ProfileEditorEffect {
        if self.step != ProfileEditorStep::Template {
            self.invalidate_review();
        }
        match self.step {
            ProfileEditorStep::Template => self.message("editor_first_step"),
            ProfileEditorStep::Identity => {
                self.step = ProfileEditorStep::Template;
                self.identity_field = IdentityField::DisplayName;
            }
            ProfileEditorStep::Specialty => {
                self.step = ProfileEditorStep::Identity;
                self.identity_field = IdentityField::Description;
            }
            ProfileEditorStep::Personality => self.step = ProfileEditorStep::Specialty,
            ProfileEditorStep::Instructions => self.step = ProfileEditorStep::Personality,
            ProfileEditorStep::OptionalBindings => self.step = ProfileEditorStep::Instructions,
            ProfileEditorStep::Review => self.step = ProfileEditorStep::OptionalBindings,
        }
        ProfileEditorEffect::None
    }

    fn clear_current(&mut self) {
        let changed = match self.step {
            ProfileEditorStep::Identity => match self.identity_field {
                IdentityField::DisplayName => self.draft.display_name.is_empty().not_then(|| {
                    self.draft.display_name.clear();
                }),
                IdentityField::Description => self.draft.description.is_empty().not_then(|| {
                    self.draft.description.clear();
                }),
            },
            ProfileEditorStep::Specialty => self.draft.primary_specialty.is_empty().not_then(|| {
                self.draft.primary_specialty.clear();
            }),
            ProfileEditorStep::Personality => self.draft.personality.is_empty().not_then(|| {
                self.draft.personality.clear();
                self.personality_started = true;
            }),
            ProfileEditorStep::Instructions => self.draft.instructions.is_empty().not_then(|| {
                self.draft.instructions.clear();
                self.instructions_started = true;
            }),
            ProfileEditorStep::OptionalBindings => {
                let changed = self.draft.bindings.model_provider.is_some()
                    || self.draft.bindings.model_name.is_some();
                self.draft.bindings.model_provider = None;
                self.draft.bindings.model_name = None;
                changed
            }
            ProfileEditorStep::Template | ProfileEditorStep::Review => {
                self.message("editor_field_unavailable");
                false
            }
        };
        if changed {
            self.invalidate_review();
            self.local_message = None;
        }
    }

    fn add_tag(&mut self, value: &str) {
        if self.step != ProfileEditorStep::Specialty {
            self.message("editor_field_unavailable");
            return;
        }
        if self.draft.specialty_tags.len() == MAX_SPECIALTY_TAGS {
            self.message("specialty_tag_limit");
            return;
        }
        self.draft.specialty_tags.push(value.to_owned());
        self.invalidate_review();
        self.local_message = None;
    }

    fn remove_tag(&mut self, value: &str) {
        if self.step != ProfileEditorStep::Specialty {
            self.message("editor_field_unavailable");
            return;
        }
        let Ok(key) = normalize_tag_key(value) else {
            self.message("invalid_profile_field");
            return;
        };
        let position = self.draft.specialty_tags.iter().position(|tag| {
            normalize_tag_key(tag)
                .map(|candidate| candidate == key)
                .unwrap_or(false)
        });
        let Some(position) = position else {
            self.message("unknown_specialty_tag");
            return;
        };
        self.draft.specialty_tags.remove(position);
        self.invalidate_review();
        self.local_message = None;
    }

    fn set_provider(&mut self, value: &str) {
        if self.step != ProfileEditorStep::OptionalBindings {
            self.message("editor_field_unavailable");
            return;
        }
        let next = Some(value.to_owned());
        if self.draft.bindings.model_provider != next {
            self.draft.bindings.model_provider = next;
            self.invalidate_review();
        }
        self.local_message = None;
    }

    fn set_model(&mut self, value: &str) {
        if self.step != ProfileEditorStep::OptionalBindings {
            self.message("editor_field_unavailable");
            return;
        }
        let next = Some(value.to_owned());
        if self.draft.bindings.model_name != next {
            self.draft.bindings.model_name = next;
            self.invalidate_review();
        }
        self.local_message = None;
    }

    fn request_review(&mut self) -> ProfileEditorEffect {
        if self.step != ProfileEditorStep::Review {
            self.message("review_unavailable");
            return ProfileEditorEffect::None;
        }
        if !self.is_valid() {
            return ProfileEditorEffect::None;
        }
        match self.mode {
            ProfileEditorMode::Create { .. } => {
                self.local_message = None;
                ProfileEditorEffect::None
            }
            ProfileEditorMode::Edit {
                profile_id,
                expected_active_version_id,
            } => {
                self.invalidate_review();
                let Some(generation) = self.preview_generation.checked_add(1) else {
                    self.message("preview_generation_exhausted");
                    return ProfileEditorEffect::None;
                };
                self.preview_generation = generation;
                self.pending_preview_generation = Some(generation);
                self.local_message = None;
                ProfileEditorEffect::PreviewEdit(PreviewEditRequest {
                    generation,
                    profile_id,
                    expected_active_version_id,
                    candidate: self.draft.clone(),
                })
            }
        }
    }

    fn activate(&mut self) -> ProfileEditorEffect {
        if self.step != ProfileEditorStep::Review {
            self.message("activation_unavailable");
            return ProfileEditorEffect::None;
        }
        if !self.is_valid() {
            return ProfileEditorEffect::None;
        }
        match &self.mode {
            ProfileEditorMode::Create { provenance } => ProfileEditorEffect::Execute(
                ApplicationCommand::CreateAgentProfile {
                    draft: self.draft.clone(),
                    template_provenance: Some(provenance.clone()),
                },
            ),
            ProfileEditorMode::Edit {
                profile_id,
                expected_active_version_id,
            } => {
                let Some(review) = &self.review else {
                    if self.local_message.is_none() {
                        self.message("preview_required");
                    }
                    return ProfileEditorEffect::None;
                };
                ProfileEditorEffect::Execute(ApplicationCommand::ActivateAgentProfileVersion {
                    profile_id: *profile_id,
                    expected_active_version_id: *expected_active_version_id,
                    candidate: self.draft.clone(),
                    review_token: review.preview.review_token,
                    review_digest: review.preview.review_digest.clone(),
                })
            }
        }
    }

    fn advance_after_validation(&mut self, next: ProfileEditorStep) {
        if self.is_valid() {
            self.step = next;
            self.local_message = None;
        }
    }

    fn is_valid(&mut self) -> bool {
        match AgentProfileDraft::new(
            self.draft.display_name.clone(),
            self.draft.description.clone(),
            self.draft.role,
            self.draft.primary_specialty.clone(),
            self.draft.specialty_tags.clone(),
            self.draft.personality.clone(),
            self.draft.instructions.clone(),
            self.draft.bindings.clone(),
            self.draft.skill_refs.clone(),
            self.draft.mcp_refs.clone(),
        ) {
            Ok(_) => true,
            Err(_) => {
                self.message("invalid_profile_field");
                false
            }
        }
    }

    fn invalidate_review(&mut self) {
        self.review = None;
        self.pending_preview_generation = None;
    }

    fn message(&mut self, code: &'static str) {
        self.local_message = Some(SafeUiMessage { code });
    }
}

fn replace_text(field: &mut String, value: &str) -> bool {
    if field == value {
        return false;
    }
    field.clear();
    field.push_str(value);
    true
}

fn append_line(field: &mut String, started: &mut bool, value: &str) -> bool {
    if !*started {
        *started = true;
        return replace_text(field, value);
    }
    if field.is_empty() {
        field.push_str(value);
    } else {
        field.push(' ');
        field.push_str(value);
    }
    true
}

trait BoolClear {
    fn not_then(self, clear: impl FnOnce()) -> bool;
}

impl BoolClear for bool {
    fn not_then(self, clear: impl FnOnce()) -> bool {
        if self {
            false
        } else {
            clear();
            true
        }
    }
}
