//! Presentation-local state machine for reviewed memory set operations.

use crate::{
    app::{AgentProfileSelector, ApplicationCommand, MemoryEditPreview},
    domain::DomainError,
    memory::{MemoryEntryDraft, MemoryEntryState, MemoryEntryVersion, MemoryMutationKind},
};

pub use crate::memory::MEMORY_PLAINTEXT_WARNING;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryEditorStep {
    Key,
    Value,
    PurposeTags,
    Review,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryPreviewRequest {
    pub generation: u64,
    pub selector: AgentProfileSelector,
    pub candidate: MemoryEntryDraft,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[expect(
    clippy::large_enum_variant,
    reason = "the Task 14 effect contract intentionally carries the exact application command"
)]
pub enum MemoryEditorEffect {
    None,
    Preview(MemoryPreviewRequest),
    Confirm(ApplicationCommand),
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryEditor {
    selector: AgentProfileSelector,
    seed: Option<MemoryEntryVersion>,
    key: String,
    value: String,
    tags: String,
    step: MemoryEditorStep,
    generation: u64,
    preview: Option<MemoryEditPreview>,
    safe_error_code: Option<&'static str>,
}

impl MemoryEditor {
    pub fn for_create(selector: AgentProfileSelector) -> Self {
        Self {
            selector,
            seed: None,
            key: String::new(),
            value: String::new(),
            tags: String::new(),
            step: MemoryEditorStep::Key,
            generation: 0,
            preview: None,
            safe_error_code: None,
        }
    }

    pub fn for_set(
        selector: AgentProfileSelector,
        entry: MemoryEntryVersion,
    ) -> Result<Self, DomainError> {
        if entry.reference().state() != MemoryEntryState::Present {
            return Err(DomainError::InvalidMemoryEditorSeed);
        }
        let Some(value) = entry.value() else {
            return Err(DomainError::InvalidMemoryEditorSeed);
        };

        Ok(Self {
            selector,
            key: entry.display_key().to_owned(),
            value: value.to_owned(),
            tags: format_tags(entry.purpose_tags()),
            seed: Some(entry),
            step: MemoryEditorStep::Value,
            generation: 0,
            preview: None,
            safe_error_code: None,
        })
    }

    pub fn submit_keyboard_line(&mut self, line: &str) -> Result<MemoryEditorEffect, DomainError> {
        if line.chars().any(char::is_control) {
            return Err(DomainError::InvalidMemoryEditorTransition);
        }
        self.submit_line(line.to_owned())
    }

    pub fn submit_line(&mut self, line: String) -> Result<MemoryEditorEffect, DomainError> {
        match self.step {
            MemoryEditorStep::Key => {
                let canonical = MemoryEntryDraft::new(line, "editor value".to_owned(), Vec::new())?;
                self.key = canonical.display_key().to_owned();
                self.invalidate_draft();
                self.step = MemoryEditorStep::Value;
                Ok(MemoryEditorEffect::None)
            }
            MemoryEditorStep::Value => {
                let canonical = MemoryEntryDraft::new(self.key.clone(), line, Vec::new())?;
                self.value = canonical.value().to_owned();
                self.invalidate_draft();
                self.step = MemoryEditorStep::PurposeTags;
                Ok(MemoryEditorEffect::None)
            }
            MemoryEditorStep::PurposeTags => {
                let canonical = MemoryEntryDraft::new(
                    self.key.clone(),
                    self.value.clone(),
                    parse_tags(&line)?,
                )?;
                self.key = canonical.display_key().to_owned();
                self.value = canonical.value().to_owned();
                self.tags = format_tags(canonical.purpose_tags());
                self.invalidate_draft();
                Ok(MemoryEditorEffect::Preview(self.preview_request()?))
            }
            MemoryEditorStep::Review => Err(DomainError::InvalidMemoryEditorTransition),
        }
    }

    pub fn back(&mut self) -> MemoryEditorEffect {
        match self.step {
            MemoryEditorStep::Key => MemoryEditorEffect::Cancelled,
            MemoryEditorStep::Value if self.seed.is_some() => MemoryEditorEffect::Cancelled,
            MemoryEditorStep::Value => {
                self.step = MemoryEditorStep::Key;
                MemoryEditorEffect::None
            }
            MemoryEditorStep::PurposeTags => {
                self.step = MemoryEditorStep::Value;
                MemoryEditorEffect::None
            }
            MemoryEditorStep::Review => {
                self.clear_review();
                MemoryEditorEffect::None
            }
        }
    }

    pub fn preview_request(&self) -> Result<MemoryPreviewRequest, DomainError> {
        if self.key.is_empty() || self.value.is_empty() || self.step == MemoryEditorStep::Key {
            return Err(DomainError::InvalidMemoryEditorTransition);
        }
        Ok(MemoryPreviewRequest {
            generation: self.generation,
            selector: self.selector.clone(),
            candidate: self
                .draft()
                .map_err(|_| DomainError::InvalidMemoryEditorTransition)?,
        })
    }

    pub fn replace_value(&mut self, value: String) -> Result<(), DomainError> {
        let canonical = MemoryEntryDraft::new(self.key.clone(), value, parse_tags(&self.tags)?)?;
        self.value = canonical.value().to_owned();
        self.tags = format_tags(canonical.purpose_tags());
        self.invalidate_draft();
        self.step = MemoryEditorStep::PurposeTags;
        Ok(())
    }

    pub fn apply_preview(&mut self, generation: u64, preview: MemoryEditPreview) -> bool {
        if generation != self.generation {
            return false;
        }
        self.preview = Some(preview);
        self.step = MemoryEditorStep::Review;
        self.safe_error_code = None;
        true
    }

    pub fn clear_review(&mut self) {
        self.preview = None;
        self.safe_error_code = None;
        self.advance_generation();
        if self.step == MemoryEditorStep::Review {
            self.step = MemoryEditorStep::PurposeTags;
        }
    }

    pub fn report_error(&mut self, code: &'static str) {
        self.safe_error_code = Some(code);
        self.preview = None;
        self.advance_generation();
    }

    pub fn draft(&self) -> Result<MemoryEntryDraft, DomainError> {
        MemoryEntryDraft::new(
            self.key.clone(),
            self.value.clone(),
            parse_tags(&self.tags)?,
        )
    }

    pub fn confirm(&self) -> Result<MemoryEditorEffect, DomainError> {
        if self.step != MemoryEditorStep::Review {
            return Err(DomainError::InvalidMemoryEditorTransition);
        }
        let Some(preview) = self.preview.as_ref() else {
            return Err(DomainError::InvalidMemoryEditorTransition);
        };
        let MemoryEditPreview::Review(review) = preview else {
            return Ok(MemoryEditorEffect::None);
        };
        if review.operation != MemoryMutationKind::Set {
            return Err(DomainError::InvalidMemoryEditorTransition);
        }
        let Some(candidate) = review.candidate.clone() else {
            return Err(DomainError::InvalidMemoryEditorTransition);
        };
        Ok(MemoryEditorEffect::Confirm(
            ApplicationCommand::SetMemoryEntry {
                profile: review.profile.clone(),
                expected: review.expected.clone(),
                candidate,
                review_token: review.review_token,
                review_digest: review.review_digest.clone(),
            },
        ))
    }

    pub fn step(&self) -> MemoryEditorStep {
        self.step
    }

    pub fn selector(&self) -> &AgentProfileSelector {
        &self.selector
    }

    pub(crate) fn seed(&self) -> Option<&MemoryEntryVersion> {
        self.seed.as_ref()
    }

    pub fn key_input(&self) -> &str {
        &self.key
    }

    pub fn value_input(&self) -> &str {
        &self.value
    }

    pub fn tags_input(&self) -> &str {
        &self.tags
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn preview(&self) -> Option<&MemoryEditPreview> {
        self.preview.as_ref()
    }

    pub fn safe_error_code(&self) -> Option<&'static str> {
        self.safe_error_code
    }

    fn invalidate_draft(&mut self) {
        self.preview = None;
        self.safe_error_code = None;
        self.advance_generation();
    }

    fn advance_generation(&mut self) {
        self.generation = self.generation.saturating_add(1);
    }
}

fn parse_tags(input: &str) -> Result<Vec<String>, DomainError> {
    let mut tags = Vec::new();
    let mut tag = String::new();
    let mut escaped = false;

    for character in input.chars() {
        if escaped {
            if !matches!(character, ',' | '\\') {
                return Err(DomainError::InvalidMemoryEditorTransition);
            }
            tag.push(character);
            escaped = false;
        } else {
            match character {
                '\\' => escaped = true,
                ',' => push_tag(&mut tags, &mut tag),
                _ => tag.push(character),
            }
        }
    }
    if escaped {
        return Err(DomainError::InvalidMemoryEditorTransition);
    }
    push_tag(&mut tags, &mut tag);
    Ok(tags)
}

fn push_tag(tags: &mut Vec<String>, tag: &mut String) {
    let trimmed = tag.trim();
    if !trimmed.is_empty() {
        tags.push(trimmed.to_owned());
    }
    tag.clear();
}

fn format_tags(tags: &[String]) -> String {
    let mut output = String::new();
    for (index, tag) in tags.iter().enumerate() {
        if index != 0 {
            output.push_str(", ");
        }
        for character in tag.chars() {
            if matches!(character, ',' | '\\') {
                output.push('\\');
            }
            output.push(character);
        }
    }
    output
}
