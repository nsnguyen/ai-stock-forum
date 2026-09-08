use std::ops::Deref;

use serde::{Deserialize, Deserializer, Serialize};

use crate::policy::Capability;
use crate::{
    agents::{
        AgentProfileDraft, NormalizedProfileName, ProfileField, ProfileTemplateProvenance,
        canonicalize_visible_text, normalize_profile_name_key,
    },
    domain::{
        Actor, AgentProfileId, AgentProfileVersionId, ApprovalId, CommandId, CorrelationId, Digest,
        DomainError, EpisodicSummaryId, MemoryProposalId, MemoryReviewToken, ObjectVersion,
        ProfileReviewToken, Sha256Digest, SkillId, SkillReviewToken, SkillVersionId, sha256,
    },
    memory::{
        ExpectedMemoryEntryState, MemoryEntryDraft, MemoryEntryRef, MemoryProposalFilter,
        MemoryProposalOperation, MemoryProposalRef, MemoryRetrievalRequest,
    },
    skills::{SkillDraft, SkillVersionRef},
};

pub const MAX_INPUT_BYTES: usize = 4096;
pub const DEFAULT_AUDIT_LIMIT: u16 = 20;
pub const MAX_AUDIT_LIMIT: u16 = 100;
pub const MAX_AGENT_PROFILE_LIST_RESULTS: usize = 100;
pub const MAX_AGENT_PROFILE_HISTORY_RESULTS: usize = 100;
pub const MAX_SKILL_LIST_RESULTS: usize = 100;
pub const MAX_SKILL_HISTORY_RESULTS: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "selector_type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum SkillSelector {
    Id(SkillId),
    Name(String),
}

impl From<SkillId> for SkillSelector {
    fn from(value: SkillId) -> Self {
        Self::Id(value)
    }
}

impl SkillSelector {
    pub fn from_input(value: &str) -> Result<Self, DomainError> {
        if let Ok(id) = uuid::Uuid::parse_str(value) {
            return Ok(Self::Id(SkillId::from_uuid(id)));
        }
        let probe = SkillDraft::new(
            value.to_owned(),
            String::new(),
            "selector".to_owned(),
            Vec::new(),
            "selector".to_owned(),
            Vec::new(),
        )?;
        Ok(Self::Name(probe.display_name))
    }

    pub(crate) fn normalized_name(&self) -> Option<crate::skills::NormalizedSkillName> {
        match self {
            Self::Id(_) => None,
            Self::Name(name) => SkillDraft::new(
                name.clone(),
                String::new(),
                "selector".to_owned(),
                Vec::new(),
                "selector".to_owned(),
                Vec::new(),
            )
            .ok()
            .and_then(|draft| draft.normalized_name().ok()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "data",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum AgentSkillAssignmentOperation {
    Assign {
        skill: SkillVersionRef,
    },
    Upgrade {
        expected: SkillVersionRef,
        replacement: SkillVersionRef,
    },
    Unassign {
        expected: SkillVersionRef,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentProfileSelector {
    Id(AgentProfileId),
    Name(String),
}

#[derive(Serialize, Deserialize)]
#[serde(
    tag = "selector_type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum AgentProfileSelectorWire {
    Id(AgentProfileId),
    Name(String),
}

impl AgentProfileSelector {
    pub fn from_input(value: &str) -> Result<Self, DomainError> {
        if let Ok(id) = uuid::Uuid::parse_str(value) {
            return Ok(Self::Id(AgentProfileId::from_uuid(id)));
        }
        Ok(Self::Name(canonicalize_visible_text(
            ProfileField::DisplayName,
            value,
            64,
            false,
        )?))
    }

    pub fn display_name(&self) -> Option<&str> {
        match self {
            Self::Id(_) => None,
            Self::Name(display_name) => Some(display_name),
        }
    }

    pub fn normalized_name(&self) -> Option<NormalizedProfileName> {
        match self {
            Self::Id(_) => None,
            Self::Name(display_name) => normalize_profile_name_key(display_name).ok(),
        }
    }
}

impl From<AgentProfileId> for AgentProfileSelector {
    fn from(value: AgentProfileId) -> Self {
        Self::Id(value)
    }
}

impl Serialize for AgentProfileSelector {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Id(id) => AgentProfileSelectorWire::Id(*id),
            Self::Name(name) => AgentProfileSelectorWire::Name(name.clone()),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for AgentProfileSelector {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match AgentProfileSelectorWire::deserialize(deserializer)? {
            AgentProfileSelectorWire::Id(id) => Ok(Self::Id(id)),
            AgentProfileSelectorWire::Name(name) => {
                Self::from_input(&name).map_err(serde::de::Error::custom)
            }
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct AuditLimit(u16);

impl AuditLimit {
    pub fn new(value: u16) -> Result<Self, AuditLimitError> {
        Self::try_from(value)
    }

    pub fn get(self) -> u16 {
        self.0
    }
}

impl TryFrom<u16> for AuditLimit {
    type Error = AuditLimitError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        if (1..=MAX_AUDIT_LIMIT).contains(&value) {
            Ok(Self(value))
        } else {
            Err(AuditLimitError::OutOfRange)
        }
    }
}

impl<'de> Deserialize<'de> for AuditLimit {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = u16::deserialize(deserializer)?;
        Self::try_from(value).map_err(|_| serde::de::Error::custom("invalid audit limit"))
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum AuditLimitError {
    OutOfRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "data",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ApplicationCommand {
    ShowHelp,
    ShowStatus,
    ShowSetupStatus,
    ShowAuditTail {
        limit: AuditLimit,
    },
    CreateAgentProfile {
        draft: AgentProfileDraft,
        template_provenance: Option<ProfileTemplateProvenance>,
    },
    ActivateAgentProfileVersion {
        profile_id: AgentProfileId,
        expected_active_version_id: AgentProfileVersionId,
        candidate: AgentProfileDraft,
        review_token: ProfileReviewToken,
        review_digest: Digest,
    },
    ListAgentProfiles,
    ShowAgentProfile {
        selector: AgentProfileSelector,
    },
    ShowAgentProfileHistory {
        selector: AgentProfileSelector,
    },
    ShowAgentProfileVersion {
        selector: AgentProfileSelector,
        version: ObjectVersion,
    },
    CreateSkill {
        skill_id: SkillId,
        candidate: SkillDraft,
        review_token: SkillReviewToken,
        review_digest: Digest,
    },
    ActivateSkillVersion {
        skill_id: SkillId,
        expected_active_version_id: SkillVersionId,
        candidate: SkillDraft,
        review_token: SkillReviewToken,
        review_digest: Digest,
    },
    ListSkills,
    ShowSkill {
        selector: SkillSelector,
    },
    ShowSkillHistory {
        selector: SkillSelector,
    },
    ShowSkillVersion {
        selector: SkillSelector,
        version: ObjectVersion,
    },
    AssignAgentSkill {
        profile_id: AgentProfileId,
        expected_active_profile_version_id: AgentProfileVersionId,
        skill: SkillVersionRef,
        review_token: SkillReviewToken,
        review_digest: Digest,
    },
    UpgradeAgentSkill {
        profile_id: AgentProfileId,
        expected_active_profile_version_id: AgentProfileVersionId,
        expected: SkillVersionRef,
        replacement: SkillVersionRef,
        review_token: SkillReviewToken,
        review_digest: Digest,
    },
    UnassignAgentSkill {
        profile_id: AgentProfileId,
        expected_active_profile_version_id: AgentProfileVersionId,
        expected: SkillVersionRef,
        review_token: SkillReviewToken,
        review_digest: Digest,
    },
    SetMemoryEntry {
        profile: crate::agents::AgentProfileVersionRef,
        expected: ExpectedMemoryEntryState,
        candidate: MemoryEntryDraft,
        review_token: MemoryReviewToken,
        review_digest: Digest,
    },
    DeleteMemoryEntry {
        profile: crate::agents::AgentProfileVersionRef,
        expected: MemoryEntryRef,
        review_token: MemoryReviewToken,
        review_digest: Digest,
    },
    ProposeMemoryMutation {
        proposer: crate::agents::AgentProfileVersionRef,
        expected: ExpectedMemoryEntryState,
        operation: MemoryProposalOperation,
        rationale: String,
    },
    ApproveMemoryProposal {
        proposal: MemoryProposalRef,
        approval_id: ApprovalId,
        expected_approval_status: crate::policy::ApprovalStatus,
        expected_entry: ExpectedMemoryEntryState,
        review_token: MemoryReviewToken,
        review_digest: Digest,
    },
    RejectMemoryProposal {
        proposal: MemoryProposalRef,
        approval_id: ApprovalId,
        expected_approval_status: crate::policy::ApprovalStatus,
        expected_entry: ExpectedMemoryEntryState,
        review_token: MemoryReviewToken,
        review_digest: Digest,
    },
    ListMemoryEntries {
        selector: AgentProfileSelector,
    },
    ShowMemoryEntry {
        selector: AgentProfileSelector,
        display_key: String,
    },
    ShowMemoryEntryHistory {
        selector: AgentProfileSelector,
        display_key: String,
    },
    ShowMemoryEntryVersion {
        selector: AgentProfileSelector,
        display_key: String,
        version: ObjectVersion,
    },
    ListMemoryProposals {
        selector: AgentProfileSelector,
        filter: MemoryProposalFilter,
    },
    ShowMemoryProposal {
        proposal_id: MemoryProposalId,
    },
    ListEpisodicSummaries {
        selector: AgentProfileSelector,
    },
    ShowEpisodicSummary {
        summary_id: EpisodicSummaryId,
    },
    BuildMemorySnapshot {
        request: MemoryRetrievalRequest,
    },
    RejectInput(InputRejection),
    RequestShutdown,
}

impl ApplicationCommand {
    pub(crate) fn canonicalize_profile_payloads(&mut self) -> Result<(), DomainError> {
        match self {
            Self::CreateAgentProfile { draft, .. } => *draft = draft.canonicalized()?,
            Self::ActivateAgentProfileVersion { candidate, .. } => {
                *candidate = candidate.canonicalized()?;
            }
            Self::CreateSkill { candidate, .. } | Self::ActivateSkillVersion { candidate, .. } => {
                *candidate = candidate.canonicalized()?;
            }
            Self::SetMemoryEntry { candidate, .. } => {
                *candidate = canonicalize_memory_draft(candidate)?;
            }
            Self::ProposeMemoryMutation {
                operation: MemoryProposalOperation::Set { candidate },
                ..
            } => {
                *candidate = canonicalize_memory_draft(candidate)?;
            }
            _ => {}
        }
        Ok(())
    }

    pub fn audit_tail(limit: u16) -> Result<Self, AuditLimitError> {
        Ok(Self::ShowAuditTail {
            limit: AuditLimit::new(limit)?,
        })
    }

    pub fn required_capability(&self) -> Capability {
        match self {
            Self::ShowHelp | Self::RejectInput(_) => Capability::HelpRead,
            Self::ShowStatus => Capability::StatusRead,
            Self::ShowSetupStatus => Capability::SetupStatusRead,
            Self::ShowAuditTail { .. } => Capability::AuditRead,
            Self::CreateAgentProfile { .. } => Capability::AgentProfileCreate,
            Self::ActivateAgentProfileVersion { .. } => Capability::AgentProfileActivate,
            Self::ListAgentProfiles
            | Self::ShowAgentProfile { .. }
            | Self::ShowAgentProfileHistory { .. }
            | Self::ShowAgentProfileVersion { .. } => Capability::AgentProfileRead,
            Self::CreateSkill { .. } => Capability::SkillCreate,
            Self::ActivateSkillVersion { .. } => Capability::SkillVersion,
            Self::ListSkills
            | Self::ShowSkill { .. }
            | Self::ShowSkillHistory { .. }
            | Self::ShowSkillVersion { .. } => Capability::SkillRead,
            Self::AssignAgentSkill { .. } | Self::UpgradeAgentSkill { .. } => {
                Capability::AgentSkillAssign
            }
            Self::UnassignAgentSkill { .. } => Capability::AgentSkillUnassign,
            Self::SetMemoryEntry { .. } | Self::DeleteMemoryEntry { .. } => {
                Capability::MemoryMutate
            }
            Self::ProposeMemoryMutation { .. } => Capability::MemoryPropose,
            Self::ApproveMemoryProposal { .. } | Self::RejectMemoryProposal { .. } => {
                Capability::MemoryResolve
            }
            Self::ListMemoryEntries { .. }
            | Self::ShowMemoryEntry { .. }
            | Self::ShowMemoryEntryHistory { .. }
            | Self::ShowMemoryEntryVersion { .. }
            | Self::ListMemoryProposals { .. }
            | Self::ShowMemoryProposal { .. }
            | Self::ListEpisodicSummaries { .. }
            | Self::ShowEpisodicSummary { .. }
            | Self::BuildMemorySnapshot { .. } => Capability::MemoryRead,
            Self::RequestShutdown => Capability::Shutdown,
        }
    }
}

fn canonicalize_memory_draft(
    candidate: &MemoryEntryDraft,
) -> Result<MemoryEntryDraft, DomainError> {
    MemoryEntryDraft::new(
        candidate.display_key().to_owned(),
        candidate.value().to_owned(),
        candidate.purpose_tags().to_vec(),
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandEnvelope {
    pub command_id: CommandId,
    pub correlation_id: CorrelationId,
    pub actor: Actor,
    pub command: ApplicationCommand,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputRejectionCategory {
    InvalidEncoding,
    Oversized,
    Malformed,
    Unknown,
}

pub const MAX_SAFE_TOKEN_CHARS: usize = 64;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SafeTokenError {
    Empty,
    TooLong,
    UnsafeCharacter,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct SafeToken(String);

impl SafeToken {
    pub fn new(value: impl Into<String>) -> Result<Self, SafeTokenError> {
        let value = value.into();
        if value.is_empty() {
            return Err(SafeTokenError::Empty);
        }
        if value.chars().count() > MAX_SAFE_TOKEN_CHARS {
            return Err(SafeTokenError::TooLong);
        }
        if value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
        {
            return Err(SafeTokenError::UnsafeCharacter);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Deref for SafeToken {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl<'de> Deserialize<'de> for SafeToken {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(|_| serde::de::Error::custom("invalid safe token"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputRejection {
    pub category: InputRejectionCategory,
    pub safe_token: Option<SafeToken>,
    pub byte_length: usize,
    pub input_digest: Sha256Digest,
}

impl InputRejection {
    pub fn from_input(
        category: InputRejectionCategory,
        safe_token: Option<SafeToken>,
        input: &[u8],
    ) -> Self {
        Self {
            category,
            safe_token,
            byte_length: input.len(),
            input_digest: sha256(input),
        }
    }
}
