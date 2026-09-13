//! Presentation-local guided editor for agent profile drafts.
//!
//! This module owns only a draft and its navigation state. Hosts submit the
//! typed effects it emits to the application boundary and return previews via
//! [`ProfileEditor::apply_preview`]. It has no persistence or terminal
//! dependencies, and its safe summaries intentionally exclude profile prose.

use std::collections::BTreeSet;

use crate::{
    agents::{
        AgentBindings, AgentProfileDraft, AgentRole, DESCRIPTION_MAX_BYTES, DISPLAY_NAME_MAX_BYTES,
        INSTRUCTIONS_MAX_BYTES, PERSONALITY_MAX_BYTES, PRIMARY_SPECIALTY_MAX_BYTES,
        ProfileEditPreview, ProfileField, ProfileTemplate, ProfileTemplateProvenance,
        SPECIALTY_TAG_MAX_BYTES, canonicalize_visible_text, normalize_profile_name_key,
        normalize_tag_key,
    },
    app::ApplicationCommand,
    domain::{AgentProfileId, AgentProfileVersionId, DomainError},
};

const MAX_SPECIALTY_TAGS: usize = crate::agents::MAX_SPECIALTY_TAGS;

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

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ProfileTuiField {
    Template,
    DisplayName,
    Role,
    Description,
    PrimarySpecialty,
    Tags,
    Personality,
    Instructions,
    Bindings,
    Review,
    Discard,
}

impl ProfileTuiField {
    const ALL: [Self; 11] = [
        Self::Template,
        Self::DisplayName,
        Self::Role,
        Self::Description,
        Self::PrimarySpecialty,
        Self::Tags,
        Self::Personality,
        Self::Instructions,
        Self::Bindings,
        Self::Review,
        Self::Discard,
    ];

    const fn index(self) -> usize {
        match self {
            Self::Template => 0,
            Self::DisplayName => 1,
            Self::Role => 2,
            Self::Description => 3,
            Self::PrimarySpecialty => 4,
            Self::Tags => 5,
            Self::Personality => 6,
            Self::Instructions => 7,
            Self::Bindings => 8,
            Self::Review => 9,
            Self::Discard => 10,
        }
    }
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct RawProfileTuiDraft {
    template: String,
    display_name: String,
    role: String,
    description: String,
    primary_specialty: String,
    tags: String,
    personality: String,
    instructions: String,
}

impl From<&AgentProfileDraft> for RawProfileTuiDraft {
    fn from(draft: &AgentProfileDraft) -> Self {
        Self {
            template: draft
                .template_provenance()
                .map(|provenance| provenance.template_id.as_str().to_owned())
                .unwrap_or_default(),
            display_name: draft.display_name.clone(),
            role: draft.role.as_str().to_owned(),
            description: draft.description.clone(),
            primary_specialty: draft.primary_specialty.clone(),
            tags: draft.specialty_tags.join(", "),
            personality: draft.personality.clone(),
            instructions: draft.instructions.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileEditor {
    mode: ProfileEditorMode,
    step: ProfileEditorStep,
    draft: AgentProfileDraft,
    create_baseline: Option<AgentProfileDraft>,
    review: Option<ProfileEditorReview>,
    local_message: Option<SafeUiMessage>,
    identity_field: IdentityField,
    personality_started: bool,
    instructions_started: bool,
    preview_generation: u64,
    pending_preview_generation: Option<u64>,
    tui_field: ProfileTuiField,
    tui_raw: RawProfileTuiDraft,
    tui_errors: [Option<&'static str>; ProfileTuiField::ALL.len()],
}

impl ProfileEditor {
    pub fn for_create(template: &ProfileTemplate) -> Result<Self, DomainError> {
        let draft = template.copy_to_draft()?;
        let tui_raw = RawProfileTuiDraft::from(&draft);
        Ok(Self {
            mode: ProfileEditorMode::Create {
                provenance: template.provenance(),
            },
            step: ProfileEditorStep::Template,
            create_baseline: Some(draft.clone()),
            draft,
            review: None,
            local_message: None,
            identity_field: IdentityField::DisplayName,
            personality_started: false,
            instructions_started: false,
            preview_generation: 0,
            pending_preview_generation: None,
            tui_field: ProfileTuiField::Template,
            tui_raw,
            tui_errors: [None; ProfileTuiField::ALL.len()],
        })
    }

    pub fn for_edit(
        profile_id: AgentProfileId,
        expected_active_version_id: AgentProfileVersionId,
        draft: AgentProfileDraft,
    ) -> Self {
        let tui_raw = RawProfileTuiDraft::from(&draft);
        Self {
            mode: ProfileEditorMode::Edit {
                profile_id,
                expected_active_version_id,
            },
            step: ProfileEditorStep::Template,
            draft,
            create_baseline: None,
            review: None,
            local_message: None,
            identity_field: IdentityField::DisplayName,
            personality_started: false,
            instructions_started: false,
            preview_generation: 0,
            pending_preview_generation: None,
            tui_field: ProfileTuiField::Template,
            tui_raw,
            tui_errors: [None; ProfileTuiField::ALL.len()],
        }
    }

    pub fn mode(&self) -> &ProfileEditorMode {
        &self.mode
    }

    pub const fn step(&self) -> ProfileEditorStep {
        self.step
    }

    pub const fn current_field_label(&self) -> &'static str {
        match self.step {
            ProfileEditorStep::Template => "Template",
            ProfileEditorStep::Identity => match self.identity_field {
                IdentityField::DisplayName => "Display name",
                IdentityField::Description => "Description",
            },
            ProfileEditorStep::Specialty => "Primary specialty",
            ProfileEditorStep::Personality => "Personality",
            ProfileEditorStep::Instructions => "Instructions",
            ProfileEditorStep::OptionalBindings => "Optional bindings",
            ProfileEditorStep::Review => "Review",
        }
    }

    pub fn draft(&self) -> &AgentProfileDraft {
        &self.draft
    }

    pub fn create_baseline(&self) -> Option<&AgentProfileDraft> {
        self.create_baseline.as_ref()
    }

    pub fn review(&self) -> Option<&ProfileEditorReview> {
        self.review.as_ref()
    }

    pub fn local_message(&self) -> Option<&SafeUiMessage> {
        self.local_message.as_ref()
    }

    pub const fn tui_field(&self) -> ProfileTuiField {
        self.tui_field
    }

    pub fn move_tui_field(&mut self, forward: bool) -> ProfileTuiField {
        let index = self.tui_field.index();
        let next = if forward {
            index.saturating_add(1).min(ProfileTuiField::ALL.len() - 1)
        } else {
            index.saturating_sub(1)
        };
        self.set_tui_navigation_field(ProfileTuiField::ALL[next]);
        self.tui_field
    }

    pub fn tui_field_text(&self, field: ProfileTuiField) -> &str {
        match field {
            ProfileTuiField::Template => &self.tui_raw.template,
            ProfileTuiField::DisplayName => &self.tui_raw.display_name,
            ProfileTuiField::Role => &self.tui_raw.role,
            ProfileTuiField::Description => &self.tui_raw.description,
            ProfileTuiField::PrimarySpecialty => &self.tui_raw.primary_specialty,
            ProfileTuiField::Tags => &self.tui_raw.tags,
            ProfileTuiField::Personality => &self.tui_raw.personality,
            ProfileTuiField::Instructions => &self.tui_raw.instructions,
            ProfileTuiField::Bindings | ProfileTuiField::Review | ProfileTuiField::Discard => "",
        }
    }

    pub fn tui_field_error(&self, field: ProfileTuiField) -> Option<&'static str> {
        self.tui_errors[field.index()]
    }

    pub fn set_tui_field(&mut self, field: ProfileTuiField, text: &str) -> bool {
        let result = match field {
            ProfileTuiField::DisplayName => {
                self.tui_raw.display_name = text.to_owned();
                canonicalize_tui_text(
                    text,
                    ProfileField::DisplayName,
                    DISPLAY_NAME_MAX_BYTES,
                    false,
                )
                .map(|canonical| self.draft.display_name = canonical)
            }
            ProfileTuiField::Description => {
                self.tui_raw.description = text.to_owned();
                canonicalize_tui_text(text, ProfileField::Description, DESCRIPTION_MAX_BYTES, true)
                    .map(|canonical| self.draft.description = canonical)
            }
            ProfileTuiField::PrimarySpecialty => {
                self.tui_raw.primary_specialty = text.to_owned();
                canonicalize_tui_primary_specialty(text, &self.draft.specialty_tags)
                    .map(|canonical| self.draft.primary_specialty = canonical)
            }
            ProfileTuiField::Tags => {
                self.tui_raw.tags = text.to_owned();
                canonicalize_tui_tags(text, &self.draft.primary_specialty)
                    .map(|tags| self.draft.specialty_tags = tags)
            }
            ProfileTuiField::Personality => {
                self.tui_raw.personality = text.to_owned();
                canonicalize_tui_text(
                    text,
                    ProfileField::Personality,
                    PERSONALITY_MAX_BYTES,
                    false,
                )
                .map(|canonical| self.draft.personality = canonical)
            }
            ProfileTuiField::Instructions => {
                self.tui_raw.instructions = text.to_owned();
                canonicalize_tui_text(
                    text,
                    ProfileField::Instructions,
                    INSTRUCTIONS_MAX_BYTES,
                    false,
                )
                .map(|canonical| self.draft.instructions = canonical)
            }
            ProfileTuiField::Template
            | ProfileTuiField::Role
            | ProfileTuiField::Bindings
            | ProfileTuiField::Review
            | ProfileTuiField::Discard => return false,
        };

        self.invalidate_review();
        match result {
            Ok(()) => {
                self.tui_errors[field.index()] = None;
                self.local_message = None;
                true
            }
            Err(code) => {
                self.tui_errors[field.index()] = Some(code);
                self.message(code);
                false
            }
        }
    }

    pub fn select_tui_role(&mut self, role: AgentRole) -> bool {
        self.tui_raw.role = role.as_str().to_owned();
        self.tui_errors[ProfileTuiField::Role.index()] = None;
        if self.draft.role != role {
            self.draft.role = role;
            self.invalidate_review();
        }
        self.local_message = None;
        true
    }

    pub fn report_error(&mut self, code: &'static str) {
        self.message(code);
    }

    pub fn select_template(&mut self, template: &ProfileTemplate) -> bool {
        if !matches!(self.mode, ProfileEditorMode::Create { .. }) {
            self.message("template_selection_create_only");
            return false;
        }
        if self.step != ProfileEditorStep::Template {
            self.message("editor_field_unavailable");
            return false;
        }
        let Ok(draft) = template.copy_to_draft() else {
            self.message("invalid_profile_field");
            return false;
        };

        self.draft = draft;
        if let ProfileEditorMode::Create { provenance } = &mut self.mode {
            *provenance = template.provenance();
            self.create_baseline = Some(self.draft.clone());
        }
        self.identity_field = IdentityField::DisplayName;
        self.personality_started = false;
        self.instructions_started = false;
        self.tui_field = ProfileTuiField::Template;
        self.tui_raw = RawProfileTuiDraft::from(&self.draft);
        self.tui_errors = [None; ProfileTuiField::ALL.len()];
        self.invalidate_review();
        self.local_message = None;
        true
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
            if self.review.is_some() {
                "ready"
            } else {
                "none"
            }
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

    /// Accepts one keyboard-editor submission. Empty input keeps the current
    /// value, while valid replacement text is applied before advancing.
    /// Colon controls retain their original explicit behavior.
    pub fn submit_keyboard_line(&mut self, line: &str) -> ProfileEditorEffect {
        if line.starts_with(':') {
            return self.submit_line(line);
        }
        if line.is_empty() {
            return match self.step {
                ProfileEditorStep::Template
                | ProfileEditorStep::Identity
                | ProfileEditorStep::Specialty
                | ProfileEditorStep::Personality
                | ProfileEditorStep::Instructions
                | ProfileEditorStep::OptionalBindings => self.next(),
                ProfileEditorStep::Review => {
                    self.message("editor_last_step");
                    ProfileEditorEffect::None
                }
            };
        }

        match self.step {
            ProfileEditorStep::Identity
            | ProfileEditorStep::Specialty
            | ProfileEditorStep::Personality
            | ProfileEditorStep::Instructions => {
                self.local_message = None;
                self.submit_text(line);
                if self.local_message.is_none() {
                    self.next()
                } else {
                    ProfileEditorEffect::None
                }
            }
            ProfileEditorStep::Template
            | ProfileEditorStep::OptionalBindings
            | ProfileEditorStep::Review => self.submit_line(line),
        }
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
            "create" => self.create(),
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
            _ if control.starts_with("role ") => {
                self.set_role(&control[5..]);
                ProfileEditorEffect::None
            }
            _ => {
                self.message("unknown_editor_control");
                ProfileEditorEffect::None
            }
        }
    }

    fn submit_text(&mut self, line: &str) {
        let tui_field = match self.step {
            ProfileEditorStep::Identity => Some(match self.identity_field {
                IdentityField::DisplayName => ProfileTuiField::DisplayName,
                IdentityField::Description => ProfileTuiField::Description,
            }),
            ProfileEditorStep::Specialty => Some(ProfileTuiField::PrimarySpecialty),
            ProfileEditorStep::Personality => Some(ProfileTuiField::Personality),
            ProfileEditorStep::Instructions => Some(ProfileTuiField::Instructions),
            ProfileEditorStep::Template
            | ProfileEditorStep::OptionalBindings
            | ProfileEditorStep::Review => None,
        };
        let changed = match self.step {
            ProfileEditorStep::Identity => match self.identity_field {
                IdentityField::DisplayName => replace_text(
                    &mut self.draft.display_name,
                    line,
                    ProfileField::DisplayName,
                    DISPLAY_NAME_MAX_BYTES,
                    false,
                ),
                IdentityField::Description => replace_text(
                    &mut self.draft.description,
                    line,
                    ProfileField::Description,
                    DESCRIPTION_MAX_BYTES,
                    true,
                ),
            },
            ProfileEditorStep::Specialty => replace_text(
                &mut self.draft.primary_specialty,
                line,
                ProfileField::PrimarySpecialty,
                PRIMARY_SPECIALTY_MAX_BYTES,
                false,
            ),
            ProfileEditorStep::Personality => append_line(
                &mut self.draft.personality,
                &mut self.personality_started,
                line,
                ProfileField::Personality,
                PERSONALITY_MAX_BYTES,
            ),
            ProfileEditorStep::Instructions => append_line(
                &mut self.draft.instructions,
                &mut self.instructions_started,
                line,
                ProfileField::Instructions,
                INSTRUCTIONS_MAX_BYTES,
            ),
            ProfileEditorStep::Template
            | ProfileEditorStep::OptionalBindings
            | ProfileEditorStep::Review => {
                self.message("editor_field_unavailable");
                Ok(false)
            }
        };
        match changed {
            Ok(changed) => {
                let repaired = tui_field.is_some_and(|field| self.sync_tui_field_from_draft(field));
                if changed {
                    self.invalidate_review();
                }
                if changed || repaired {
                    self.local_message = None;
                }
            }
            Err(TextEditError::Limit) => self.message("profile_field_limit"),
            Err(TextEditError::Invalid) => self.message("invalid_profile_field"),
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
                    if self.validate_current() {
                        self.identity_field = IdentityField::Description;
                        self.local_message = None;
                    }
                }
                IdentityField::Description => {
                    self.advance_after_validation(ProfileEditorStep::Specialty)
                }
            },
            ProfileEditorStep::Specialty => {
                self.advance_after_validation(ProfileEditorStep::Personality)
            }
            ProfileEditorStep::Personality => {
                self.advance_after_validation(ProfileEditorStep::Instructions)
            }
            ProfileEditorStep::Instructions => {
                self.advance_after_validation(ProfileEditorStep::OptionalBindings)
            }
            ProfileEditorStep::OptionalBindings => {
                self.advance_after_validation(ProfileEditorStep::Review)
            }
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
        let tui_field = match self.step {
            ProfileEditorStep::Identity => Some(match self.identity_field {
                IdentityField::DisplayName => ProfileTuiField::DisplayName,
                IdentityField::Description => ProfileTuiField::Description,
            }),
            ProfileEditorStep::Specialty => Some(ProfileTuiField::PrimarySpecialty),
            ProfileEditorStep::Personality => Some(ProfileTuiField::Personality),
            ProfileEditorStep::Instructions => Some(ProfileTuiField::Instructions),
            ProfileEditorStep::Template
            | ProfileEditorStep::OptionalBindings
            | ProfileEditorStep::Review => None,
        };
        let changed = match self.step {
            ProfileEditorStep::Identity => match self.identity_field {
                IdentityField::DisplayName => self.draft.display_name.is_empty().not_then(|| {
                    self.draft.display_name.clear();
                }),
                IdentityField::Description => self.draft.description.is_empty().not_then(|| {
                    self.draft.description.clear();
                }),
            },
            ProfileEditorStep::Specialty => {
                self.draft.primary_specialty.is_empty().not_then(|| {
                    self.draft.primary_specialty.clear();
                })
            }
            ProfileEditorStep::Personality => self.draft.personality.is_empty().not_then(|| {
                self.draft.personality.clear();
                self.personality_started = true;
            }),
            ProfileEditorStep::Instructions => self.draft.instructions.is_empty().not_then(|| {
                self.draft.instructions.clear();
                self.instructions_started = true;
            }),
            ProfileEditorStep::OptionalBindings => {
                let changed = self.draft.bindings != AgentBindings::default();
                self.draft.bindings = AgentBindings::default();
                changed
            }
            ProfileEditorStep::Template | ProfileEditorStep::Review => {
                self.message("editor_field_unavailable");
                false
            }
        };
        let repaired = tui_field.is_some_and(|field| self.sync_tui_field_from_draft(field));
        if changed {
            self.invalidate_review();
        }
        if changed || repaired {
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
        let tag = match canonicalize_visible_text(
            ProfileField::SpecialtyTag,
            value,
            SPECIALTY_TAG_MAX_BYTES,
            false,
        ) {
            Ok(tag) => tag,
            Err(_) if value.len() > SPECIALTY_TAG_MAX_BYTES => {
                self.message("profile_field_limit");
                return;
            }
            Err(_) => {
                self.message("invalid_profile_field");
                return;
            }
        };
        let Ok(key) = normalize_tag_key(&tag) else {
            self.message("invalid_profile_field");
            return;
        };
        if normalize_profile_name_key(&self.draft.primary_specialty)
            .is_ok_and(|primary| primary.as_str() == key)
            || self
                .draft
                .specialty_tags
                .iter()
                .any(|existing| normalize_tag_key(existing).is_ok_and(|existing| existing == key))
        {
            self.message("invalid_profile_field");
            return;
        }
        self.draft.specialty_tags.push(tag);
        self.draft
            .specialty_tags
            .sort_by_cached_key(|tag| normalize_tag_key(tag).unwrap_or_else(|_| tag.clone()));
        self.sync_tui_field_from_draft(ProfileTuiField::Tags);
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
        self.sync_tui_field_from_draft(ProfileTuiField::Tags);
        self.invalidate_review();
        self.local_message = None;
    }

    fn set_provider(&mut self, value: &str) {
        if self.step != ProfileEditorStep::OptionalBindings {
            self.message("editor_field_unavailable");
            return;
        }
        let _ = value;
        self.message("binding_reference_unavailable");
    }

    fn set_role(&mut self, value: &str) {
        if self.step != ProfileEditorStep::Template {
            self.message("editor_field_unavailable");
            return;
        }
        let role = match value {
            "bull" => AgentRole::Bull,
            "bear" => AgentRole::Bear,
            "chief" => AgentRole::Chief,
            "engineering" => AgentRole::Engineering,
            "custom" => AgentRole::Custom,
            _ => {
                self.message("invalid_profile_field");
                return;
            }
        };
        if self.draft.role != role {
            self.draft.role = role;
            self.invalidate_review();
        }
        self.sync_tui_field_from_draft(ProfileTuiField::Role);
        self.local_message = None;
    }

    fn set_model(&mut self, value: &str) {
        if self.step != ProfileEditorStep::OptionalBindings {
            self.message("editor_field_unavailable");
            return;
        }
        let _ = value;
        self.message("binding_reference_unavailable");
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

    fn create(&mut self) -> ProfileEditorEffect {
        if !matches!(self.mode, ProfileEditorMode::Create { .. }) {
            self.message("activation_action_required");
            return ProfileEditorEffect::None;
        }
        self.execute_create()
    }

    fn activate(&mut self) -> ProfileEditorEffect {
        if !matches!(self.mode, ProfileEditorMode::Edit { .. }) {
            self.message("create_action_required");
            return ProfileEditorEffect::None;
        }
        self.execute_activation()
    }

    fn execute_create(&mut self) -> ProfileEditorEffect {
        if self.step != ProfileEditorStep::Review {
            self.message("create_unavailable");
            return ProfileEditorEffect::None;
        }
        if !self.is_valid() {
            return ProfileEditorEffect::None;
        }
        match &self.mode {
            ProfileEditorMode::Create { provenance } => {
                ProfileEditorEffect::Execute(ApplicationCommand::CreateAgentProfile {
                    draft: self.draft.clone(),
                    template_provenance: Some(provenance.clone()),
                })
            }
            ProfileEditorMode::Edit { .. } => ProfileEditorEffect::None,
        }
    }

    fn execute_activation(&mut self) -> ProfileEditorEffect {
        if self.step != ProfileEditorStep::Review {
            self.message("activation_unavailable");
            return ProfileEditorEffect::None;
        }
        if !self.is_valid() {
            return ProfileEditorEffect::None;
        }
        match &self.mode {
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
            ProfileEditorMode::Create { .. } => ProfileEditorEffect::None,
        }
    }

    fn advance_after_validation(&mut self, next: ProfileEditorStep) {
        if self.validate_current() {
            self.step = next;
            self.local_message = None;
        }
    }

    fn is_valid(&mut self) -> bool {
        if self.tui_errors.iter().any(Option::is_some) {
            self.message("invalid_profile_field");
            return false;
        }
        match self.draft.canonicalized() {
            Ok(canonical) => {
                self.draft = canonical;
                true
            }
            Err(_) => {
                self.message("invalid_profile_field");
                false
            }
        }
    }

    fn validate_current(&mut self) -> bool {
        let result = match self.step {
            ProfileEditorStep::Template | ProfileEditorStep::OptionalBindings => Ok(()),
            ProfileEditorStep::Identity => match self.identity_field {
                IdentityField::DisplayName => canonicalize_visible_text(
                    ProfileField::DisplayName,
                    &self.draft.display_name,
                    DISPLAY_NAME_MAX_BYTES,
                    false,
                )
                .map(|value| self.draft.display_name = value),
                IdentityField::Description => canonicalize_visible_text(
                    ProfileField::Description,
                    &self.draft.description,
                    DESCRIPTION_MAX_BYTES,
                    true,
                )
                .map(|value| self.draft.description = value),
            },
            ProfileEditorStep::Specialty => canonicalize_visible_text(
                ProfileField::PrimarySpecialty,
                &self.draft.primary_specialty,
                PRIMARY_SPECIALTY_MAX_BYTES,
                false,
            )
            .and_then(|value| {
                self.draft.primary_specialty = value;
                for tag in &self.draft.specialty_tags {
                    normalize_tag_key(tag)?;
                }
                Ok(())
            }),
            ProfileEditorStep::Personality => canonicalize_visible_text(
                ProfileField::Personality,
                &self.draft.personality,
                PERSONALITY_MAX_BYTES,
                false,
            )
            .map(|value| self.draft.personality = value),
            ProfileEditorStep::Instructions => canonicalize_visible_text(
                ProfileField::Instructions,
                &self.draft.instructions,
                INSTRUCTIONS_MAX_BYTES,
                false,
            )
            .map(|value| self.draft.instructions = value),
            ProfileEditorStep::Review => self
                .draft
                .canonicalized()
                .map(|canonical| self.draft = canonical),
        };
        match result {
            Ok(()) => true,
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

    fn set_tui_navigation_field(&mut self, field: ProfileTuiField) {
        self.tui_field = field;
        match field {
            ProfileTuiField::Template | ProfileTuiField::Role => {
                self.step = ProfileEditorStep::Template;
            }
            ProfileTuiField::DisplayName => {
                self.step = ProfileEditorStep::Identity;
                self.identity_field = IdentityField::DisplayName;
            }
            ProfileTuiField::Description => {
                self.step = ProfileEditorStep::Identity;
                self.identity_field = IdentityField::Description;
            }
            ProfileTuiField::PrimarySpecialty | ProfileTuiField::Tags => {
                self.step = ProfileEditorStep::Specialty;
            }
            ProfileTuiField::Personality => self.step = ProfileEditorStep::Personality,
            ProfileTuiField::Instructions => self.step = ProfileEditorStep::Instructions,
            ProfileTuiField::Bindings => self.step = ProfileEditorStep::OptionalBindings,
            ProfileTuiField::Review | ProfileTuiField::Discard => {
                self.step = ProfileEditorStep::Review;
            }
        }
    }

    fn sync_tui_field_from_draft(&mut self, field: ProfileTuiField) -> bool {
        let canonical = match field {
            ProfileTuiField::Template => self
                .draft
                .template_provenance()
                .map(|provenance| provenance.template_id.as_str().to_owned())
                .unwrap_or_default(),
            ProfileTuiField::DisplayName => self.draft.display_name.clone(),
            ProfileTuiField::Role => self.draft.role.as_str().to_owned(),
            ProfileTuiField::Description => self.draft.description.clone(),
            ProfileTuiField::PrimarySpecialty => self.draft.primary_specialty.clone(),
            ProfileTuiField::Tags => self.draft.specialty_tags.join(", "),
            ProfileTuiField::Personality => self.draft.personality.clone(),
            ProfileTuiField::Instructions => self.draft.instructions.clone(),
            ProfileTuiField::Bindings | ProfileTuiField::Review | ProfileTuiField::Discard => {
                return false;
            }
        };
        let raw = match field {
            ProfileTuiField::Template => &mut self.tui_raw.template,
            ProfileTuiField::DisplayName => &mut self.tui_raw.display_name,
            ProfileTuiField::Role => &mut self.tui_raw.role,
            ProfileTuiField::Description => &mut self.tui_raw.description,
            ProfileTuiField::PrimarySpecialty => &mut self.tui_raw.primary_specialty,
            ProfileTuiField::Tags => &mut self.tui_raw.tags,
            ProfileTuiField::Personality => &mut self.tui_raw.personality,
            ProfileTuiField::Instructions => &mut self.tui_raw.instructions,
            ProfileTuiField::Bindings | ProfileTuiField::Review | ProfileTuiField::Discard => {
                return false;
            }
        };
        let repaired = *raw != canonical || self.tui_errors[field.index()].is_some();
        *raw = canonical;
        self.tui_errors[field.index()] = None;
        repaired
    }
}

fn canonicalize_tui_text(
    value: &str,
    field: ProfileField,
    max_bytes: usize,
    allow_empty: bool,
) -> Result<String, &'static str> {
    canonicalize_visible_text(field, value, max_bytes, allow_empty).map_err(|_| {
        if value.len() > max_bytes {
            "profile_field_limit"
        } else {
            "invalid_profile_field"
        }
    })
}

fn canonicalize_tui_tags(
    value: &str,
    primary_specialty: &str,
) -> Result<Vec<String>, &'static str> {
    let values: Vec<_> = value
        .split(',')
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .collect();
    if values.len() > MAX_SPECIALTY_TAGS {
        return Err("specialty_tag_limit");
    }
    if values.is_empty() {
        return Ok(Vec::new());
    }

    let primary_key =
        normalize_profile_name_key(primary_specialty).map_err(|_| "invalid_profile_field")?;
    let mut keys = BTreeSet::new();
    let mut tags = Vec::with_capacity(values.len());
    for value in values {
        let tag = canonicalize_tui_text(
            value,
            ProfileField::SpecialtyTag,
            SPECIALTY_TAG_MAX_BYTES,
            false,
        )?;
        let key = normalize_tag_key(&tag).map_err(|_| "invalid_profile_field")?;
        if key == primary_key.as_str() || !keys.insert(key.clone()) {
            return Err("invalid_profile_field");
        }
        tags.push((key, tag));
    }
    tags.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(tags.into_iter().map(|(_, tag)| tag).collect())
}

fn canonicalize_tui_primary_specialty(
    value: &str,
    tags: &[String],
) -> Result<String, &'static str> {
    let specialty = canonicalize_tui_text(
        value,
        ProfileField::PrimarySpecialty,
        PRIMARY_SPECIALTY_MAX_BYTES,
        false,
    )?;
    let specialty_key =
        normalize_profile_name_key(&specialty).map_err(|_| "invalid_profile_field")?;
    if tags
        .iter()
        .any(|tag| normalize_tag_key(tag).is_ok_and(|tag_key| tag_key == specialty_key.as_str()))
    {
        return Err("invalid_profile_field");
    }
    Ok(specialty)
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum TextEditError {
    Invalid,
    Limit,
}

fn replace_text(
    field: &mut String,
    value: &str,
    profile_field: ProfileField,
    max_bytes: usize,
    allow_empty: bool,
) -> Result<bool, TextEditError> {
    let canonical = match canonicalize_visible_text(profile_field, value, max_bytes, allow_empty) {
        Ok(canonical) => canonical,
        Err(_)
            if !allow_empty
                && !value.is_empty()
                && value.chars().all(char::is_whitespace)
                && !value.chars().any(char::is_control)
                && value.len() <= max_bytes =>
        {
            value.to_owned()
        }
        Err(_) if value.len() > max_bytes => return Err(TextEditError::Limit),
        Err(_) => return Err(TextEditError::Invalid),
    };
    if field == &canonical {
        return Ok(false);
    }
    *field = canonical;
    Ok(true)
}

fn append_line(
    field: &mut String,
    started: &mut bool,
    value: &str,
    profile_field: ProfileField,
    max_bytes: usize,
) -> Result<bool, TextEditError> {
    let value =
        canonicalize_visible_text(profile_field, value, max_bytes, false).map_err(|_| {
            if value.len() > max_bytes {
                TextEditError::Limit
            } else {
                TextEditError::Invalid
            }
        })?;
    if !*started {
        *started = true;
        if field == &value {
            return Ok(false);
        }
        *field = value;
        return Ok(true);
    }
    let separator_bytes = usize::from(!field.is_empty());
    if field.len() + separator_bytes + value.len() > max_bytes {
        return Err(TextEditError::Limit);
    }
    if field.is_empty() {
        field.push_str(&value);
    } else {
        field.push(' ');
        field.push_str(&value);
    }
    Ok(true)
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
