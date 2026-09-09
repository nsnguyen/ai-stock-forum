use crate::{
    agents::{AgentProfileVersion, AgentReadiness, AgentRole, ProfileFieldDiff},
    app::{AuditLimit, EventEnvelope, InputRejection},
    audit::AuditEntry,
    domain::{
        AgentProfileId, AgentProfileVersionId, ApprovalId, CommandId, CorrelationId, Digest,
        InstallationId, MemoryNamespaceId, ObjectVersion, SessionId,
    },
    memory::{
        EpisodicQualification, EpisodicSummary, EpisodicSummaryRef, ExpectedMemoryEntryState,
        MemoryEntryRef, MemoryEntryVersion, MemoryProposal, MemoryProposalFilter,
        MemoryProposalOperationKind, MemoryProposalRef, MemoryProposalResolution,
        MemoryProposalStatus, MemorySnapshot,
    },
    setup::SetupStatus,
    skills::{ContentDigest, SkillDraft, SkillProvenance, SkillVersionRef},
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

impl Serialize for AgentReadiness {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(match self {
            Self::Unbound => "unbound",
            Self::BindingUnavailable => "binding_unavailable",
            Self::Ready => "ready",
        })
    }
}

impl<'de> Deserialize<'de> for AgentReadiness {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match String::deserialize(deserializer)?.as_str() {
            "unbound" => Ok(Self::Unbound),
            "binding_unavailable" => Ok(Self::BindingUnavailable),
            "ready" => Ok(Self::Ready),
            _ => Err(serde::de::Error::custom("invalid agent readiness")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandOutcome {
    pub command_id: CommandId,
    pub correlation_id: CorrelationId,
    pub committed_events: Vec<EventEnvelope>,
    pub view: CommandView,
    pub shutdown: ShutdownDisposition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelpView;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusView {
    pub installation_id: InstallationId,
    pub session_id: SessionId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetupStatusView {
    pub status: SetupStatus,
}

impl SetupStatusView {
    pub fn is_not_started(&self) -> bool {
        self.status == SetupStatus::NotStarted
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditTailView {
    pub limit: AuditLimit,
    pub entries: Vec<AuditEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputRejectedView {
    pub rejection: InputRejection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShutdownView {
    pub disposition: ShutdownDisposition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProfileCreatedView {
    pub profile_id: AgentProfileId,
    pub profile_version_id: AgentProfileVersionId,
    pub version: ObjectVersion,
    pub readiness: AgentReadiness,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProfileVersionActivatedView {
    pub profile_id: AgentProfileId,
    pub profile_version_id: AgentProfileVersionId,
    pub previous_version_id: AgentProfileVersionId,
    pub version: ObjectVersion,
    pub readiness: AgentReadiness,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProfileSummary {
    pub profile_id: AgentProfileId,
    pub profile_version_id: AgentProfileVersionId,
    pub version: ObjectVersion,
    pub display_name: String,
    pub role: AgentRole,
    pub primary_specialty: String,
    pub readiness: AgentReadiness,
    pub content_digest: Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProfilesView {
    pub profiles: Vec<AgentProfileSummary>,
    pub total_count: u32,
    pub returned_count: u32,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProfileView {
    pub profile: AgentProfileVersion,
    pub readiness: AgentReadiness,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProfileHistoryEntry {
    pub profile_version_id: AgentProfileVersionId,
    pub version: ObjectVersion,
    pub supersedes: Option<AgentProfileVersionId>,
    pub created_at_ms: i64,
    pub readiness: AgentReadiness,
    pub content_digest: Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProfileHistoryView {
    pub profile_id: AgentProfileId,
    pub active_version_id: AgentProfileVersionId,
    pub versions: Vec<AgentProfileHistoryEntry>,
    pub total_count: u32,
    pub returned_count: u32,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProfileVersionView {
    pub profile: AgentProfileVersion,
    pub readiness: AgentReadiness,
    pub predecessor_diff: Vec<ProfileFieldDiff>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillCreatedView {
    pub skill_id: crate::domain::SkillId,
    pub skill_version_id: crate::domain::SkillVersionId,
    pub version: ObjectVersion,
    pub content_digest: Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillVersionActivatedView {
    pub skill_id: crate::domain::SkillId,
    pub skill_version_id: crate::domain::SkillVersionId,
    pub previous_version_id: crate::domain::SkillVersionId,
    pub version: ObjectVersion,
    pub content_digest: Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillSummary {
    pub skill_ref: SkillVersionRef,
    pub display_name: String,
    pub provenance: SkillProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillsView {
    pub skills: Vec<SkillSummary>,
    pub total_count: u32,
    pub returned_count: u32,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillView {
    pub skill_ref: SkillVersionRef,
    pub content: SkillDraft,
    pub created_at_ms: i64,
    pub provenance: SkillProvenance,
    pub predecessor_version_id: Option<crate::domain::SkillVersionId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillHistoryEntry {
    pub skill_ref: SkillVersionRef,
    pub created_at_ms: i64,
    pub predecessor_version_id: Option<crate::domain::SkillVersionId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillHistoryView {
    pub skill_id: crate::domain::SkillId,
    pub active_version_id: crate::domain::SkillVersionId,
    pub versions: Vec<SkillHistoryEntry>,
    pub total_count: u32,
    pub returned_count: u32,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSkillMutationView {
    pub profile_id: AgentProfileId,
    pub profile_version_id: AgentProfileVersionId,
    pub previous_profile_version_id: AgentProfileVersionId,
    pub version: ObjectVersion,
    pub profile_content_digest: Digest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSkillAssignmentPreview {
    pub profile_id: AgentProfileId,
    pub expected_active_profile_version_id: AgentProfileVersionId,
    pub operation: crate::app::AgentSkillAssignmentOperation,
    pub review_token: crate::domain::SkillReviewToken,
    pub review_digest: ContentDigest,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryEntrySummary {
    pub entry: MemoryEntryRef,
    pub display_key: String,
    pub purpose_tags: Vec<String>,
    pub value_bytes: u64,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryEntriesView {
    pub profile: crate::agents::AgentProfileVersionRef,
    pub namespace_id: MemoryNamespaceId,
    pub entries: Vec<MemoryEntrySummary>,
    pub total_count: u64,
    pub returned_count: u64,
    pub omitted_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryEntryView {
    pub profile: crate::agents::AgentProfileVersionRef,
    pub entry: MemoryEntryVersion,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryEntryHistorySummary {
    pub entry: MemoryEntryRef,
    pub display_key: String,
    pub created_at_ms: i64,
    pub accepted_proposal: Option<MemoryProposalRef>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryEntryHistoryView {
    pub profile: crate::agents::AgentProfileVersionRef,
    pub current: MemoryEntryRef,
    pub versions: Vec<MemoryEntryHistorySummary>,
    pub total_count: u64,
    pub returned_count: u64,
    pub omitted_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryEntryVersionView {
    pub profile: crate::agents::AgentProfileVersionRef,
    pub entry: MemoryEntryVersion,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryProposalSummary {
    pub proposal: MemoryProposalRef,
    pub namespace_id: MemoryNamespaceId,
    pub proposer: crate::agents::AgentProfileVersionRef,
    pub operation: MemoryProposalOperationKind,
    pub display_key: String,
    pub status: MemoryProposalStatus,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryProposalsView {
    pub profile: crate::agents::AgentProfileVersionRef,
    pub namespace_id: MemoryNamespaceId,
    pub filter: MemoryProposalFilter,
    pub proposals: Vec<MemoryProposalSummary>,
    pub total_count: u64,
    pub returned_count: u64,
    pub omitted_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryProfileIdentityView {
    pub profile: crate::agents::AgentProfileVersionRef,
    pub display_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryProposalView {
    pub proposal: MemoryProposal,
    pub status: MemoryProposalStatus,
    pub resolution: Option<MemoryProposalResolution>,
    pub current_entry: ExpectedMemoryEntryState,
    pub proposer_is_historical: bool,
    pub proposer_identity: MemoryProfileIdentityView,
    pub namespace_owner_identity: MemoryProfileIdentityView,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodicSummaryListItem {
    pub summary: EpisodicSummaryRef,
    pub label: String,
    pub purpose_tags: Vec<String>,
    pub source_count: u64,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodicSummariesView {
    pub profile: crate::agents::AgentProfileVersionRef,
    pub namespace_id: MemoryNamespaceId,
    pub summaries: Vec<EpisodicSummaryListItem>,
    pub total_count: u64,
    pub returned_count: u64,
    pub omitted_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodicSummaryView {
    pub summary: EpisodicSummary,
    pub qualification: EpisodicQualification,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryEntryMutationView {
    pub entry: MemoryEntryRef,
    pub expired_proposals: Vec<MemoryProposalRef>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryProposalCreatedView {
    pub proposal: MemoryProposalRef,
    pub approval_id: ApprovalId,
    pub status: MemoryProposalStatus,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryProposalResolutionView {
    pub resolution: MemoryProposalResolution,
    pub entry: Option<MemoryEntryRef>,
    pub expired_proposals: Vec<MemoryProposalRef>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemorySnapshotView {
    pub snapshot: MemorySnapshot,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShutdownDisposition {
    Continue,
    Requested,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "data",
    rename_all = "snake_case",
    deny_unknown_fields
)]
#[allow(clippy::large_enum_variant)] // Contract variants intentionally retain exact typed payloads.
pub enum CommandView {
    Help(HelpView),
    Status(StatusView),
    SetupStatus(SetupStatusView),
    AuditTail(AuditTailView),
    AgentProfileCreated(AgentProfileCreatedView),
    AgentProfileVersionActivated(AgentProfileVersionActivatedView),
    AgentProfiles(AgentProfilesView),
    AgentProfile(AgentProfileView),
    AgentProfileHistory(AgentProfileHistoryView),
    AgentProfileVersion(AgentProfileVersionView),
    SkillCreated(SkillCreatedView),
    SkillVersionActivated(SkillVersionActivatedView),
    Skills(SkillsView),
    Skill(SkillView),
    SkillHistory(SkillHistoryView),
    SkillVersion(SkillView),
    AgentSkillAssigned(AgentSkillMutationView),
    AgentSkillUpgraded(AgentSkillMutationView),
    AgentSkillUnassigned(AgentSkillMutationView),
    MemoryEntries(MemoryEntriesView),
    MemoryEntry(MemoryEntryView),
    MemoryEntryHistory(MemoryEntryHistoryView),
    MemoryEntryVersion(MemoryEntryVersionView),
    MemoryProposals(MemoryProposalsView),
    MemoryProposal(MemoryProposalView),
    EpisodicSummaries(EpisodicSummariesView),
    EpisodicSummary(EpisodicSummaryView),
    MemoryEntryMutation(MemoryEntryMutationView),
    MemoryProposalCreated(MemoryProposalCreatedView),
    MemoryProposalResolution(MemoryProposalResolutionView),
    MemorySnapshot(MemorySnapshotView),
    InputRejected(InputRejectedView),
    Shutdown(ShutdownView),
}
