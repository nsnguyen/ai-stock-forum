use std::io::{self, Write};

use crate::{
    agents::{
        AgentProfileDraft, AgentReadiness, McpRef, ProfileDiffField, ProfileFieldDiff,
        ProfileFieldValue, ProfileTemplate,
    },
    app::{
        AgentSkillAssignmentOperation, AgentSkillAssignmentPreview, AppError, ApplicationCommand,
        CommandOutcome, CommandView, EpisodicSummariesView, EpisodicSummaryView,
        InputRejectionCategory, MemoryEntriesView, MemoryEntryHistoryView, MemoryEntryMutationView,
        MemoryEntryVersionView, MemoryEntryView, MemoryProfileIdentityView,
        MemoryProposalCreatedView, MemoryProposalResolutionView, MemoryProposalView,
        MemoryProposalsView, SafeToken, ShutdownDisposition, ShutdownReason,
    },
    cli::CliError,
    config::StartupError,
    domain::Actor,
    memory::{
        EpisodicQualification, ExpectedMemoryEntryState, MemoryEntryDraft, MemoryEntryRef,
        MemoryProposalOperation, MemoryProposalRef,
    },
    runtime::RuntimeError,
    setup::SetupStatus,
    skills::{SkillDraft, SkillEditPreview, SkillVersionRef},
    ui::{
        profile_editor::{ProfileEditor, ProfileEditorMode, ProfileEditorStep},
        tui::TuiError,
    },
};

const MAX_PROFILE_LIST_ROWS: usize = 100;
const MAX_PROFILE_HISTORY_ROWS: usize = 100;
const MAX_MEMORY_LIST_ROWS: usize = 100;
const MAX_MEMORY_KEY_RENDER_BYTES: usize = 384;
const MAX_MEMORY_TAG_RENDER_BYTES: usize = 128;
const MAX_MEMORY_VALUE_RENDER_BYTES: usize = 16_384;
const MAX_MEMORY_RATIONALE_RENDER_BYTES: usize = 2_048;
const MAX_EPISODIC_LABEL_RENDER_BYTES: usize = 512;
const MAX_EPISODIC_BODY_RENDER_BYTES: usize = 32_768;
const MAX_EPISODIC_EVENT_TYPE_RENDER_BYTES: usize = 512;
const MEMORY_USAGE: &[u8] = b"Usage:\n  /memory list <agent>\n  /memory get <agent> <key>\n  /memory history <agent> <key> [positive-version]\n  /memory set <agent> <key>\n  /memory delete <agent> <key>\n  /memory proposals <agent> [pending|all]\n  /memory proposal <proposal-id>\n  /memory approve <proposal-id>\n  /memory reject <proposal-id>\n  /memory episodes <agent>\n  /memory episode <summary-id>\n";

pub struct TextRenderer;

impl TextRenderer {
    pub fn render_outcome<W: Write>(outcome: &CommandOutcome, writer: &mut W) -> io::Result<()> {
        Self::render_view(&outcome.view, writer)
    }

    pub fn render_view<W: Write>(view: &CommandView, writer: &mut W) -> io::Result<()> {
        if let Some(result) = render_memory_view(view, writer) {
            return result;
        }
        match view {
            CommandView::Help(_) => writer.write_all(
                b"Available commands:\n  /help\n  /status\n  /setup status\n  /audit tail [limit: 1-100]\n  /skill list\n  /skills\n  /skill add\n  /skill show <name-or-id> [version]\n  /skill assign <skill> <agent> [version]\n  /skill unassign <skill> <agent>\nMemory commands:\n    /memory list <agent>\n    /memory get <agent> <key>\n    /memory history <agent> <key> [version]\n    /memory set <agent> <key>\n    /memory delete <agent> <key>\n    /memory proposals <agent> [pending|all]\n    /memory proposal <proposal-id>\n    /memory approve <proposal-id>\n    /memory reject <proposal-id>\n    /memory episodes <agent>\n    /memory episode <summary-id>\n  /quit\nMemory internal producers are unavailable: proposal creation, summary mutation, and snapshot building.\n",
            ),
            CommandView::Status(_) => {
                writer.write_all(b"Installation: ready\nSession: active\n")
            }
            CommandView::SetupStatus(view) => match &view.status {
                SetupStatus::NotStarted => writer.write_all(
                    b"Setup: not started\nGuided setup is not implemented in Phase 0.\n",
                ),
                SetupStatus::DraftSaved { .. } => writer.write_all(b"Setup: draft saved\n"),
                SetupStatus::Applied { .. } => writer.write_all(b"Setup: applied\n"),
            },
            CommandView::AuditTail(view) => {
                writeln!(writer, "Audit tail (limit {}):", view.limit.get())?;
                if view.entries.is_empty() {
                    return writer.write_all(b"  No audit entries.\n");
                }
                for entry in &view.entries {
                    let actor = match entry.actor {
                        Actor::Human => "human",
                        Actor::System => "system",
                        Actor::Agent(_) => "agent",
                    };
                    let kind = escaped_bounded(&entry.kind, 64);
                    let summary = escaped_bounded(&entry.summary, 256);
                    writeln!(
                        writer,
                        "  #{} {} {} {} {} {}",
                        entry.sequence,
                        entry.occurred_at_ms,
                        actor,
                        kind,
                        entry.correlation_id,
                        summary,
                    )?;
                }
                Ok(())
            }
            CommandView::InputRejected(view) => match view.rejection.category {
                InputRejectionCategory::InvalidEncoding => {
                    writer.write_all(b"Input rejected: invalid encoding.\n")
                }
                InputRejectionCategory::Oversized => {
                    writer.write_all(b"Input rejected: input exceeds 4096 bytes.\n")
                }
                InputRejectionCategory::Malformed => {
                    writer.write_all(b"Input rejected: malformed command.\n")?;
                    match view.rejection.safe_token.as_ref().map(SafeToken::as_str) {
                        Some("/skill" | "/skills") => writer.write_all(b"Usage: /skill list | /skill add | /skill show <name-or-id> [version] | /skill assign <skill> <agent> [version] | /skill unassign <skill> <agent>\n"),
                        Some("/memory") => writer.write_all(MEMORY_USAGE),
                        _ => Ok(()),
                    }
                }
                InputRejectionCategory::Unknown => {
                    if let Some(token) = &view.rejection.safe_token {
                        writeln!(
                            writer,
                            "Input rejected: unknown command {}.",
                            escaped_bounded(token.as_str(), 64)
                        )
                    } else {
                        writer.write_all(b"Input rejected: unknown command.\n")
                    }
                }
            },
            CommandView::Shutdown(view) => match view.disposition {
                ShutdownDisposition::Continue => {
                    writer.write_all(b"Shutdown was not requested.\n")
                }
                ShutdownDisposition::Requested => writer.write_all(b"Shutting down.\n"),
            },
            CommandView::AgentProfileCreated(view) => writeln!(
                writer,
                "Agent profile created: {} version {} ({})",
                view.profile_id,
                view.version.get(),
                readiness(view.readiness)
            ),
            CommandView::AgentProfileVersionActivated(view) => writeln!(
                writer,
                "Agent profile version activated: {} version {} ({})",
                view.profile_id,
                view.version.get(),
                readiness(view.readiness)
            ),
            CommandView::AgentProfiles(view) => {
                writer.write_all(b"NAME | ROLE | SPECIALTY | READY | VERSION | ID\n")?;
                for profile in view.profiles.iter().take(MAX_PROFILE_LIST_ROWS) {
                    writeln!(
                        writer,
                        "{} | {} | {} | {} | {} | {}",
                        escaped_bounded(&profile.display_name, 64),
                        profile.role.as_str(),
                        escaped_bounded(&profile.primary_specialty, 64),
                        readiness(profile.readiness),
                        profile.version.get(),
                        profile.profile_id,
                    )?;
                }
                let omitted = view.profiles.len().saturating_sub(MAX_PROFILE_LIST_ROWS);
                if omitted > 0 {
                    writeln!(writer, "... {omitted} profiles omitted.")?;
                }
                Ok(())
            }
            CommandView::AgentProfile(view) => {
                let profile = &view.profile;
                writeln!(writer, "Agent profile: {}", profile.profile_id())?;
                writeln!(writer, "Display name: {}", escaped_bounded(profile.display_name(), 64))?;
                writeln!(writer, "Description: {}", escaped_bounded(profile.description(), 256))?;
                writeln!(writer, "Role: {}", profile.role().as_str())?;
                writeln!(
                    writer,
                    "Primary specialty: {}",
                    escaped_bounded(profile.primary_specialty(), 64)
                )?;
                writeln!(
                    writer,
                    "Specialty tags: {}",
                    escaped_list(profile.specialty_tags(), 48)
                )?;
                writeln!(writer, "Personality: {}", escaped_bounded(profile.personality(), 1_024))?;
                writeln!(writer, "Instructions: {}", escaped_bounded(profile.instructions(), 4_096))?;
                writeln!(
                    writer,
                    "Bindings: {}",
                    bindings_summary(profile.bindings())
                )?;
                writeln!(writer, "Skill refs: {}", escaped_skill_refs(profile.skill_refs()))?;
                writeln!(writer, "MCP refs: {}", escaped_mcp_refs(profile.mcp_refs()))?;
                match profile.template_provenance() {
                    Some(provenance) => writeln!(
                        writer,
                        "Template provenance: {}@{} {}",
                        escaped_bounded(provenance.template_id.as_str(), 64),
                        provenance.template_version.get(),
                        provenance.template_digest,
                    )?,
                    None => writer.write_all(b"Template provenance: none\n")?,
                }
                writeln!(writer, "Bindings readiness: {}", readiness(view.readiness))?;
                writeln!(writer, "Memory namespace ID: {}", profile.memory_namespace_id())?;
                writeln!(writer, "Policy reference: {}", escaped_bounded(profile.default_policy_ref(), 128))?;
                writeln!(writer, "Created time: {}", profile.created_at_ms())?;
                writeln!(writer, "Version: {}", profile.version().get())?;
                writeln!(writer, "Version ID: {}", profile.profile_version_id())?;
                writeln!(writer, "Digest: {}", profile.content_digest())
            }
            CommandView::AgentProfileHistory(view) => {
                writeln!(writer, "Agent profile history: {}", view.profile_id)?;
                writeln!(writer, "Active version ID: {}", view.active_version_id)?;
                writer.write_all(b"VERSION | VERSION ID | PREDECESSOR | CREATED | READY | DIGEST\n")?;
                for version in view.versions.iter().take(MAX_PROFILE_HISTORY_ROWS) {
                    writeln!(
                        writer,
                        "{} | {} | {} | {} | {} | {}",
                        version.version.get(),
                        version.profile_version_id,
                        version
                            .supersedes
                            .map(|id| id.to_string())
                            .unwrap_or_else(|| "none".to_owned()),
                        version.created_at_ms,
                        readiness(version.readiness),
                        version.content_digest,
                    )?;
                }
                let omitted = view.total_count.saturating_sub(view.returned_count);
                if omitted > 0 {
                    writeln!(writer, "... {omitted} versions omitted.")?;
                }
                Ok(())
            }
            CommandView::AgentProfileVersion(view) => {
                let profile = &view.profile;
                writeln!(
                    writer,
                    "Agent profile historical version: {} version {}",
                    profile.profile_id(),
                    profile.version().get()
                )?;
                writeln!(
                    writer,
                    "Display name: {}",
                    escaped_bounded(profile.display_name(), 64)
                )?;
                writeln!(
                    writer,
                    "Description: {}",
                    escaped_bounded(profile.description(), 256)
                )?;
                writeln!(writer, "Role: {}", profile.role().as_str())?;
                writeln!(
                    writer,
                    "Primary specialty: {}",
                    escaped_bounded(profile.primary_specialty(), 64)
                )?;
                writeln!(
                    writer,
                    "Specialty tags: {}",
                    escaped_list(profile.specialty_tags(), 48)
                )?;
                writeln!(
                    writer,
                    "Personality: {}",
                    escaped_bounded(profile.personality(), 1_024)
                )?;
                writeln!(
                    writer,
                    "Instructions: {}",
                    escaped_bounded(profile.instructions(), 4_096)
                )?;
                writeln!(
                    writer,
                    "Bindings: {}",
                    bindings_summary(profile.bindings())
                )?;
                writeln!(
                    writer,
                    "Skill refs: {}",
                    escaped_skill_refs(profile.skill_refs())
                )?;
                writeln!(
                    writer,
                    "MCP refs: {}",
                    escaped_mcp_refs(profile.mcp_refs())
                )?;
                match profile.template_provenance() {
                    Some(provenance) => writeln!(
                        writer,
                        "Template provenance: {}@{} {}",
                        escaped_bounded(provenance.template_id.as_str(), 64),
                        provenance.template_version.get(),
                        provenance.template_digest,
                    )?,
                    None => writer.write_all(b"Template provenance: none\n")?,
                }
                writeln!(writer, "Readiness: {}", readiness(view.readiness))?;
                writeln!(
                    writer,
                    "Memory namespace ID: {}",
                    profile.memory_namespace_id()
                )?;
                writeln!(
                    writer,
                    "Policy reference: {}",
                    escaped_bounded(profile.default_policy_ref(), 128)
                )?;
                writeln!(writer, "Created time: {}", profile.created_at_ms())?;
                writeln!(
                    writer,
                    "Predecessor version ID: {}",
                    profile
                        .supersedes()
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| "none".to_owned())
                )?;
                writeln!(writer, "Version: {}", profile.version().get())?;
                writeln!(writer, "Version ID: {}", profile.profile_version_id())?;
                writeln!(writer, "Digest: {}", profile.content_digest())?;
                writer.write_all(b"Predecessor diff:\n")?;
                render_profile_diffs(&view.predecessor_diff, writer)
            }
            CommandView::SkillCreated(view) => writeln!(
                writer,
                "Skill created: {} version {} digest {}",
                view.skill_id,
                view.version.get(),
                view.content_digest,
            ),
            CommandView::SkillVersionActivated(view) => writeln!(
                writer,
                "Skill version activated: {} version {} digest {}",
                view.skill_id,
                view.version.get(),
                view.content_digest,
            ),
            CommandView::Skills(view) => {
                writer.write_all(b"NAME | VERSION | ID | DIGEST\n")?;
                for skill in view.skills.iter().take(100) {
                    writeln!(
                        writer,
                        "{} | {} | {} | {}",
                        escaped_bounded(&skill.display_name, 64),
                        skill.skill_ref.version().get(),
                        skill.skill_ref.skill_id(),
                        skill.skill_ref.content_digest(),
                    )?;
                }
                let omitted = view.total_count.saturating_sub(view.returned_count);
                if omitted > 0 {
                    writeln!(writer, "... {omitted} skills omitted.")?;
                }
                Ok(())
            }
            CommandView::Skill(view) | CommandView::SkillVersion(view) => {
                writeln!(writer, "Skill: {}", view.skill_ref.skill_id())?;
                writeln!(
                    writer,
                    "Display name: {}",
                    escaped_bounded(&view.content.display_name, 64)
                )?;
                writeln!(writer, "Version: {}", view.skill_ref.version().get())?;
                writeln!(writer, "Version ID: {}", view.skill_ref.skill_version_id())?;
                writeln!(writer, "Digest: {}", view.skill_ref.content_digest())?;
                writeln!(writer, "Provenance: {:?}", view.provenance)?;
                writeln!(writer, "Resource count: {}", view.content.resources.len())
            }
            CommandView::SkillHistory(view) => {
                writeln!(writer, "Skill history: {}", view.skill_id)?;
                writeln!(writer, "Active version ID: {}", view.active_version_id)?;
                writer.write_all(b"VERSION | VERSION ID | PREDECESSOR | CREATED | DIGEST\n")?;
                for version in view.versions.iter().take(100) {
                    writeln!(
                        writer,
                        "{} | {} | {} | {} | {}",
                        version.skill_ref.version().get(),
                        version.skill_ref.skill_version_id(),
                        version
                            .predecessor_version_id
                            .map(|id| id.to_string())
                            .unwrap_or_else(|| "none".to_owned()),
                        version.created_at_ms,
                        version.skill_ref.content_digest(),
                    )?;
                }
                Ok(())
            }
            CommandView::AgentSkillAssigned(view) => writeln!(
                writer,
                "Agent skill assigned: profile {} version {}",
                view.profile_id,
                view.version.get(),
            ),
            CommandView::AgentSkillUpgraded(view) => writeln!(
                writer,
                "Agent skill upgraded: profile {} version {}",
                view.profile_id,
                view.version.get(),
            ),
            CommandView::AgentSkillUnassigned(view) => writeln!(
                writer,
                "Agent skill unassigned: profile {} version {}",
                view.profile_id,
                view.version.get(),
            ),
            CommandView::MemoryEntries(_)
            | CommandView::MemoryEntry(_)
            | CommandView::MemoryEntryVersion(_)
            | CommandView::MemoryEntryHistory(_)
            | CommandView::MemoryProposals(_)
            | CommandView::MemoryProposal(_)
            | CommandView::EpisodicSummaries(_)
            | CommandView::EpisodicSummary(_)
            | CommandView::MemoryEntryMutation(_)
            | CommandView::MemoryProposalCreated(_)
            | CommandView::MemoryProposalResolution(_) => {
                unreachable!("memory views are rendered before the non-memory view match")
            }
            CommandView::MemorySnapshot(view) => writeln!(
                writer,
                "Memory snapshot: entries {} summaries {} digest {}",
                view.snapshot.entries().len(),
                view.snapshot.summaries().len(),
                view.snapshot.snapshot_digest(),
            ),
        }
    }

    pub fn render_profile_templates<W: Write>(
        templates: &[ProfileTemplate],
        writer: &mut W,
    ) -> io::Result<()> {
        writer.write_all(b"Profile templates:\n")?;
        for template in templates {
            writeln!(
                writer,
                "  {} | {} | {} | version {}",
                escaped_bounded(template.id.as_str(), 64),
                template.role.as_str(),
                escaped_bounded(template.suggested_name, 64),
                template.version.get(),
            )?;
        }
        writer.write_all(b"Enter a template ID, or :cancel.\n")
    }

    pub fn render_skill_editor<W: Write>(draft: &SkillDraft, writer: &mut W) -> io::Result<()> {
        writer.write_all(b"Create skill editor\n")?;
        writeln!(
            writer,
            "  Display name: {}",
            escaped_bounded(&draft.display_name, 64)
        )?;
        writeln!(
            writer,
            "  Description: {}",
            escaped_bounded(&draft.description, 256)
        )?;
        writeln!(
            writer,
            "  Use when: {}",
            escaped_bounded(&draft.use_when, 512)
        )?;
        writeln!(writer, "  Tags: {}", escaped_list(&draft.tags, 32))?;
        writeln!(
            writer,
            "  Instructions: {}",
            escaped_bounded(&draft.instructions, 4_096)
        )?;
        writer.write_all(b"Controls: :name <text> :description <text> :use-when <text> :tag add <tag> :tag remove <tag> :instructions <text> :review :cancel\n")
    }

    pub fn render_skill_editor_message<W: Write>(message: &str, writer: &mut W) -> io::Result<()> {
        writeln!(writer, "Skill editor: {}", escaped_bounded(message, 128))
    }

    pub fn render_skill_creation_review<W: Write>(
        candidate: &SkillDraft,
        preview: &SkillEditPreview,
        writer: &mut W,
    ) -> io::Result<()> {
        writeln!(
            writer,
            "Skill creation review: {} version 1 digest {}",
            escaped_bounded(&candidate.display_name, 64),
            preview.candidate_digest,
        )?;
        writer.write_all(b"Type exactly: create\n")
    }

    pub fn render_skill_assignment_review<W: Write>(
        preview: &AgentSkillAssignmentPreview,
        writer: &mut W,
    ) -> io::Result<()> {
        let (action, reference) = match &preview.operation {
            AgentSkillAssignmentOperation::Assign { skill } => ("assign", skill),
            AgentSkillAssignmentOperation::Upgrade { replacement, .. } => ("upgrade", replacement),
            AgentSkillAssignmentOperation::Unassign { expected } => ("unassign", expected),
        };
        writeln!(writer, "Skill assignment review: {action}")?;
        writeln!(
            writer,
            "  Agent: {} base {}",
            preview.profile_id, preview.expected_active_profile_version_id,
        )?;
        writeln!(
            writer,
            "  Skill: {} exact version {} ({})",
            reference.skill_id(),
            reference.version().get(),
            reference.content_digest(),
        )?;
        writeln!(writer, "Type exactly: {action}")
    }

    pub fn render_skill_confirmation<W: Write>(
        action: &str,
        command: &ApplicationCommand,
        writer: &mut W,
    ) -> io::Result<()> {
        let reference = match command {
            ApplicationCommand::AssignAgentSkill { skill, .. } => Some(skill),
            ApplicationCommand::UpgradeAgentSkill { replacement, .. } => Some(replacement),
            ApplicationCommand::UnassignAgentSkill { expected, .. } => Some(expected),
            ApplicationCommand::CreateSkill { .. } => None,
            _ => return writer.write_all(b"Skill confirmation is unavailable.\n"),
        };
        if let Some(reference) = reference {
            writeln!(
                writer,
                "Skill assignment review: {action}\n  Skill: {} exact version {} ({})",
                reference.skill_id(),
                reference.version().get(),
                reference.content_digest(),
            )?;
        } else {
            writer.write_all(b"Skill creation review\n")?;
        }
        writeln!(writer, "Type exactly: {action}")
    }

    pub fn render_skill_confirmation_mismatch<W: Write>(writer: &mut W) -> io::Result<()> {
        writer.write_all(b"Confirmation did not match; skill review retained.\n")
    }

    pub fn render_fresh_skill_review_required<W: Write>(writer: &mut W) -> io::Result<()> {
        writer.write_all(b"Start a fresh /skill command to request a new review.\n")
    }

    pub fn render_skill_cancelled<W: Write>(writer: &mut W) -> io::Result<()> {
        writer.write_all(b"Skill review cancelled.\n")
    }

    pub fn render_profile_editor<W: Write>(
        editor: &ProfileEditor,
        writer: &mut W,
    ) -> io::Result<()> {
        let mode = match editor.mode() {
            ProfileEditorMode::Create { .. } => "Create",
            ProfileEditorMode::Edit { .. } => "Edit",
        };
        writeln!(writer, "{mode} profile editor [{}]", editor.step().as_str())?;
        render_draft(editor.draft(), writer)?;
        if editor.step() == ProfileEditorStep::Review {
            writeln!(writer, "{mode} profile review")?;
            match editor.mode() {
                ProfileEditorMode::Create { .. } => {
                    if let Some(baseline) = editor.create_baseline() {
                        render_profile_diffs(&draft_diffs(baseline, editor.draft()), writer)?;
                        writer.write_all(b"  Unchanged fields omitted.\n")?;
                    }
                    if let ProfileEditorMode::Create { provenance } = editor.mode() {
                        writeln!(
                            writer,
                            "  Template provenance: {}@{} {}",
                            escaped_bounded(provenance.template_id.as_str(), 128),
                            provenance.template_version.get(),
                            provenance.template_digest
                        )?;
                    }
                    writer.write_all(b"  Create action: :create\n")?;
                }
                ProfileEditorMode::Edit {
                    expected_active_version_id,
                    ..
                } => {
                    writeln!(writer, "  Base version ID: {expected_active_version_id}")?;
                    if let Some(review) = editor.review() {
                        render_profile_diffs(&review.preview().diffs, writer)?;
                        writeln!(
                            writer,
                            "  Review digest: {}",
                            review.preview().review_digest
                        )?;
                        writer.write_all(b"  Activate action: :activate\n")?;
                    } else {
                        writer.write_all(b"  Run :review to request a passive preview.\n")?;
                    }
                }
            }
        }
        if let Some(message) = editor.local_message() {
            writeln!(writer, "Editor message: {}", message.code())?;
        }
        writer.write_all(b"Limits: display name 1-64 bytes; description 0-256 bytes; primary specialty 1-64 bytes; tags 0-5 at 1-48 bytes each; personality 1-1024 cumulative bytes; instructions 1-4096 cumulative bytes.\n")?;
        match editor.mode() {
            ProfileEditorMode::Create { .. } => writer.write_all(
                b"Controls: :role <bull|bear|chief|engineering|custom> :tag add <tag> :tag remove <tag> :next :back :show :clear :review :create :cancel\n",
            ),
            ProfileEditorMode::Edit { .. } => writer.write_all(
                b"Controls: :role <bull|bear|chief|engineering|custom> :tag add <tag> :tag remove <tag> :next :back :show :clear :review :activate :cancel\n",
            ),
        }
    }

    pub fn render_profile_confirmation<W: Write>(
        editor: &ProfileEditor,
        command: &ApplicationCommand,
        expected_confirmation: &str,
        writer: &mut W,
    ) -> io::Result<()> {
        match (editor.mode(), command) {
            (
                ProfileEditorMode::Create { provenance },
                ApplicationCommand::CreateAgentProfile { .. },
            ) => writeln!(
                writer,
                "Create and activate from {}@{} {}. Type exactly: {expected_confirmation}",
                escaped_bounded(provenance.template_id.as_str(), 128),
                provenance.template_version.get(),
                provenance.template_digest,
            ),
            (
                ProfileEditorMode::Edit {
                    expected_active_version_id,
                    ..
                },
                ApplicationCommand::ActivateAgentProfileVersion { review_digest, .. },
            ) => writeln!(
                writer,
                "Activate from base {expected_active_version_id} with review digest {review_digest}. Type exactly: {expected_confirmation}",
            ),
            _ => writer.write_all(b"Profile confirmation is unavailable.\n"),
        }
    }

    pub fn render_activation_declined<W: Write>(writer: &mut W) -> io::Result<()> {
        writer.write_all(b"Confirmation cancelled; returned to review.\n")
    }

    pub fn render_confirmation_mismatch<W: Write>(writer: &mut W) -> io::Result<()> {
        writer.write_all(b"Confirmation did not match; draft retained.\n")
    }

    pub fn render_profile_cancelled<W: Write>(create: bool, writer: &mut W) -> io::Result<()> {
        if create {
            writer.write_all(b"Profile creation cancelled.\n")
        } else {
            writer.write_all(b"Profile edit cancelled.\n")
        }
    }

    pub fn render_startup_error<W: Write>(error: StartupError, writer: &mut W) -> io::Result<()> {
        writeln!(writer, "Startup failed [{}].", error.code())
    }

    pub fn render_cli_error<W: Write>(error: &CliError, writer: &mut W) -> io::Result<()> {
        match error {
            CliError::InvalidArguments => {
                writer.write_all(b"Command-line arguments are invalid.\n")
            }
        }
    }

    pub fn render_runtime_error<W: Write>(error: &RuntimeError, writer: &mut W) -> io::Result<()> {
        let message = match error {
            RuntimeError::InvalidCapacity => "Runtime configuration is invalid.",
            RuntimeError::Backpressure => "Command queue is busy; try again.",
            RuntimeError::Closed => "Application is shutting down.",
            RuntimeError::Application(error) => app_error_message(error),
            RuntimeError::WorkerStartup => "Application worker could not start.",
            RuntimeError::WorkerExited | RuntimeError::WorkerPanicked => {
                "Application worker stopped unexpectedly."
            }
            RuntimeError::TerminationTimedOut => "Application worker did not stop in time.",
        };
        writeln!(writer, "{message}")
    }

    pub fn render_shutdown_reason<W: Write>(
        reason: ShutdownReason,
        writer: &mut W,
    ) -> io::Result<()> {
        let message = match reason {
            ShutdownReason::UserQuit => "Shutting down.",
            ShutdownReason::InputClosed => "Input closed. Shutting down.",
            ShutdownReason::Interrupted => "Interrupted. Shutting down.",
            ShutdownReason::ApplicationError => "Stopping after an application error.",
        };
        writeln!(writer, "{message}")
    }

    pub fn render_previous_session_warning<W: Write>(writer: &mut W) -> io::Result<()> {
        writer.write_all(b"Warning: the previous session ended unexpectedly.\n")
    }

    pub fn render_ui_error<W: Write>(error: &super::UiError, writer: &mut W) -> io::Result<()> {
        match error {
            super::UiError::Read => writer.write_all(b"Input could not be read.\n"),
            super::UiError::Write => writer.write_all(b"Output could not be written.\n"),
            super::UiError::Runtime(error) => Self::render_runtime_error(error, writer),
            super::UiError::InterruptHandler => {
                writer.write_all(b"Interrupt handling could not be started.\n")
            }
            super::UiError::ReaderThread => {
                writer.write_all(b"Input worker stopped unexpectedly.\n")
            }
            super::UiError::LineSourceUnavailable => {
                writer.write_all(b"Terminal input is unavailable on this platform.\n")
            }
            super::UiError::Panicked => writer.write_all(b"Command host stopped unexpectedly.\n"),
        }
    }

    pub fn render_tui_error<W: Write>(error: &TuiError, writer: &mut W) -> io::Result<()> {
        match error {
            TuiError::TerminalInitialization => {
                writer.write_all(b"Terminal interface could not be started.\n")
            }
            TuiError::TerminalInput => writer.write_all(b"Terminal input could not be read.\n"),
            TuiError::TerminalOutput => {
                writer.write_all(b"Terminal output could not be written.\n")
            }
            TuiError::InterruptHandler => {
                writer.write_all(b"Interrupt handling could not be started.\n")
            }
            TuiError::Runtime(error) => Self::render_runtime_error(error, writer),
            TuiError::Panicked => writer.write_all(b"Terminal interface stopped unexpectedly.\n"),
        }
    }
}

fn render_memory_view<W: Write>(view: &CommandView, writer: &mut W) -> Option<io::Result<()>> {
    match view {
        CommandView::MemoryEntries(value) => Some(render_memory_entries(value, writer)),
        CommandView::MemoryEntry(value) => Some(render_memory_entry(value, writer)),
        CommandView::MemoryEntryHistory(value) => Some(render_memory_history(value, writer)),
        CommandView::MemoryEntryVersion(value) => Some(render_memory_version(value, writer)),
        CommandView::MemoryProposals(value) => Some(render_memory_proposals(value, writer)),
        CommandView::MemoryProposal(value) => Some(render_memory_proposal(value, writer)),
        CommandView::EpisodicSummaries(value) => Some(render_episodic_summaries(value, writer)),
        CommandView::EpisodicSummary(value) => Some(render_episodic_summary(value, writer)),
        CommandView::MemoryEntryMutation(value) => Some(render_memory_mutation(value, writer)),
        CommandView::MemoryProposalCreated(value) => Some(render_proposal_created(value, writer)),
        CommandView::MemoryProposalResolution(value) => {
            Some(render_proposal_resolution(value, writer))
        }
        CommandView::MemorySnapshot(_) => None,
        _ => None,
    }
}

fn render_memory_entries<W: Write>(view: &MemoryEntriesView, writer: &mut W) -> io::Result<()> {
    let (rendered_count, omitted_count) =
        rendered_list_counts(view.omitted_count, view.entries.len());
    writeln!(
        writer,
        "Memory entries: profile {} namespace {} returned {} of {} ({} omitted)",
        view.profile.profile_id(),
        view.namespace_id,
        rendered_count,
        view.total_count,
        omitted_count,
    )?;
    writer.write_all(b"KEY | STATE | VERSION | BYTES | TAGS | CREATED | VERSION ID | DIGEST\n")?;
    for item in view.entries.iter().take(MAX_MEMORY_LIST_ROWS) {
        writeln!(
            writer,
            "{} | {:?} | {} | {} | {} | {} | {} | {}",
            escaped_bounded_bytes(&item.display_key, MAX_MEMORY_KEY_RENDER_BYTES),
            item.entry.state(),
            item.entry.version().get(),
            item.value_bytes,
            escaped_memory_list(&item.purpose_tags, MAX_MEMORY_TAG_RENDER_BYTES),
            item.created_at_ms,
            item.entry.entry_version_id(),
            item.entry.content_digest(),
        )?;
    }
    write_memory_omitted(writer, "entries", omitted_count)
}

fn render_memory_entry<W: Write>(view: &MemoryEntryView, writer: &mut W) -> io::Result<()> {
    render_memory_entry_detail("Memory entry", &view.profile, &view.entry, writer)
}

fn render_memory_history<W: Write>(
    view: &MemoryEntryHistoryView,
    writer: &mut W,
) -> io::Result<()> {
    let (rendered_count, omitted_count) =
        rendered_list_counts(view.omitted_count, view.versions.len());
    writeln!(
        writer,
        "Memory entry history: profile {} current {} returned {} of {} ({} omitted)",
        view.profile.profile_id(),
        view.current.entry_version_id(),
        rendered_count,
        view.total_count,
        omitted_count,
    )?;
    writer.write_all(
        b"VERSION | KEY | STATE | CREATED | VERSION ID | ACCEPTED PROPOSAL | DIGEST\n",
    )?;
    for item in view.versions.iter().take(MAX_MEMORY_LIST_ROWS) {
        let accepted = item.accepted_proposal.as_ref().map_or_else(
            || "none".to_owned(),
            |proposal| {
                format!(
                    "{}@{}#{}",
                    proposal.proposal_id(),
                    proposal.version().get(),
                    proposal.content_digest()
                )
            },
        );
        writeln!(
            writer,
            "{} | {} | {:?} | {} | {} | {} | {}",
            item.entry.version().get(),
            escaped_bounded_bytes(&item.display_key, MAX_MEMORY_KEY_RENDER_BYTES),
            item.entry.state(),
            item.created_at_ms,
            item.entry.entry_version_id(),
            accepted,
            item.entry.content_digest(),
        )?;
    }
    write_memory_omitted(writer, "versions", omitted_count)
}

fn render_memory_version<W: Write>(
    view: &MemoryEntryVersionView,
    writer: &mut W,
) -> io::Result<()> {
    render_memory_entry_detail(
        "Memory entry historical version",
        &view.profile,
        &view.entry,
        writer,
    )
}

fn render_memory_proposals<W: Write>(view: &MemoryProposalsView, writer: &mut W) -> io::Result<()> {
    let (rendered_count, omitted_count) =
        rendered_list_counts(view.omitted_count, view.proposals.len());
    writeln!(
        writer,
        "Memory proposals: profile {} namespace {} filter {:?} returned {} of {} ({} omitted)",
        view.profile.profile_id(),
        view.namespace_id,
        view.filter,
        rendered_count,
        view.total_count,
        omitted_count,
    )?;
    writer.write_all(b"KEY | OPERATION | STATUS | PROPOSER | CREATED | PROPOSAL ID | DIGEST\n")?;
    for item in view.proposals.iter().take(MAX_MEMORY_LIST_ROWS) {
        writeln!(
            writer,
            "{} | {:?} | {:?} | {}@{} | {} | {} | {}",
            escaped_bounded_bytes(&item.display_key, MAX_MEMORY_KEY_RENDER_BYTES),
            item.operation,
            item.status,
            item.proposer.profile_id(),
            item.proposer.version().get(),
            item.created_at_ms,
            item.proposal.proposal_id(),
            item.proposal.content_digest(),
        )?;
    }
    write_memory_omitted(writer, "proposals", omitted_count)
}

fn render_memory_proposal<W: Write>(view: &MemoryProposalView, writer: &mut W) -> io::Result<()> {
    let proposal_ref = view.proposal.reference();
    writeln!(writer, "Memory proposal: {}", proposal_ref.proposal_id())?;
    writeln!(writer, "Version: {}", proposal_ref.version().get())?;
    writeln!(writer, "Status: {:?}", view.status)?;
    writeln!(writer, "Digest: {}", proposal_ref.content_digest())?;
    writeln!(writer, "Approval ID: {}", view.proposal.approval_id())?;
    writeln!(writer, "Namespace: {}", view.proposal.namespace_id())?;
    render_memory_profile_identity(
        "Proposer",
        &view.proposer_identity,
        view.proposer_is_historical,
        writer,
    )?;
    render_memory_profile_identity(
        "Namespace owner",
        &view.namespace_owner_identity,
        false,
        writer,
    )?;
    writeln!(
        writer,
        "Display key: {}",
        escaped_bounded_bytes(view.proposal.display_key(), MAX_MEMORY_KEY_RENDER_BYTES),
    )?;
    writeln!(
        writer,
        "Normalized key: {}",
        escaped_bounded_bytes(
            view.proposal.normalized_key().as_str(),
            MAX_MEMORY_KEY_RENDER_BYTES,
        ),
    )?;
    render_expected_entry("Expected entry", view.proposal.expected(), writer)?;
    render_expected_entry("Current entry", &view.current_entry, writer)?;
    match view.proposal.operation() {
        MemoryProposalOperation::Set { candidate } => {
            writer.write_all(b"Operation: Set\n")?;
            render_memory_candidate(candidate, writer)?;
        }
        MemoryProposalOperation::Delete => writer.write_all(b"Operation: Delete\n")?,
    }
    writeln!(
        writer,
        "Rationale: {}",
        escaped_bounded_bytes(view.proposal.rationale(), MAX_MEMORY_RATIONALE_RENDER_BYTES,),
    )?;
    writeln!(writer, "Created: {}", view.proposal.created_at_ms())?;
    if let Some(resolution) = &view.resolution {
        writeln!(
            writer,
            "Resolution: {:?} approval {} event {} at {}",
            resolution.status(),
            resolution.approval_id(),
            resolution.resolution_event_id(),
            resolution.resolved_at_ms(),
        )?;
    }
    Ok(())
}

fn render_episodic_summaries<W: Write>(
    view: &EpisodicSummariesView,
    writer: &mut W,
) -> io::Result<()> {
    let (rendered_count, omitted_count) =
        rendered_list_counts(view.omitted_count, view.summaries.len());
    writeln!(
        writer,
        "Episodic summaries: profile {} namespace {} returned {} of {} ({} omitted)",
        view.profile.profile_id(),
        view.namespace_id,
        rendered_count,
        view.total_count,
        omitted_count,
    )?;
    writer
        .write_all(b"QUALIFICATION | LABEL | TAGS | SOURCES | CREATED | SUMMARY ID | DIGEST\n")?;
    for item in view.summaries.iter().take(MAX_MEMORY_LIST_ROWS) {
        writeln!(
            writer,
            "{} | {} | {} | {} | {} | {} | {}",
            EpisodicQualification::SummaryVerifySources.label(),
            escaped_bounded_bytes(&item.label, MAX_EPISODIC_LABEL_RENDER_BYTES),
            escaped_memory_list(&item.purpose_tags, MAX_MEMORY_TAG_RENDER_BYTES),
            item.source_count,
            item.created_at_ms,
            item.summary.summary_id(),
            item.summary.content_digest(),
        )?;
    }
    write_memory_omitted(writer, "summaries", omitted_count)
}

fn render_episodic_summary<W: Write>(view: &EpisodicSummaryView, writer: &mut W) -> io::Result<()> {
    let reference = view.summary.reference();
    writeln!(writer, "Episodic summary: {}", reference.summary_id())?;
    writeln!(writer, "Qualification: {}", view.qualification.label())?;
    writeln!(writer, "Version: {}", reference.version().get())?;
    render_memory_profile_ref("Profile", reference.profile(), writer)?;
    writeln!(writer, "Namespace: {}", reference.namespace_id())?;
    writeln!(writer, "Digest: {}", reference.content_digest())?;
    writeln!(
        writer,
        "Creation event sequence: {}",
        reference.creation_event_sequence()
    )?;
    writeln!(
        writer,
        "Creation event ID: {}",
        reference.creation_event_id()
    )?;
    writeln!(
        writer,
        "Source set digest: {}",
        reference.source_set_digest()
    )?;
    writeln!(
        writer,
        "Label: {}",
        escaped_bounded_bytes(view.summary.label(), MAX_EPISODIC_LABEL_RENDER_BYTES),
    )?;
    writeln!(
        writer,
        "Body: {}",
        escaped_bounded_bytes(view.summary.body(), MAX_EPISODIC_BODY_RENDER_BYTES),
    )?;
    writeln!(
        writer,
        "Purpose tags: {}",
        escaped_memory_list(view.summary.purpose_tags(), MAX_MEMORY_TAG_RENDER_BYTES),
    )?;
    writer.write_all(b"Sources:\n")?;
    for source in view.summary.sources() {
        writeln!(
            writer,
            "  sequence {} event {} type {} digest {}",
            source.sequence(),
            source.event_id(),
            escaped_bounded_bytes(source.event_type(), MAX_EPISODIC_EVENT_TYPE_RENDER_BYTES,),
            source.event_digest(),
        )?;
    }
    writeln!(writer, "Created: {}", view.summary.created_at_ms())
}

fn render_memory_profile_identity<W: Write>(
    label: &str,
    identity: &MemoryProfileIdentityView,
    historical: bool,
    writer: &mut W,
) -> io::Result<()> {
    writeln!(
        writer,
        "{label}: {}",
        escaped_bounded_bytes(&identity.display_name, MAX_EPISODIC_LABEL_RENDER_BYTES,),
    )?;
    render_memory_profile_ref(&format!("{label} profile"), &identity.profile, writer)?;
    if historical {
        writeln!(writer, "{label} warning: historical profile version")?;
    }
    Ok(())
}

fn render_memory_profile_ref<W: Write>(
    label: &str,
    profile: &crate::agents::AgentProfileVersionRef,
    writer: &mut W,
) -> io::Result<()> {
    writeln!(
        writer,
        "{label}: {}@{}",
        profile.profile_id(),
        profile.version().get()
    )?;
    writeln!(
        writer,
        "{label} version ID: {}",
        profile.profile_version_id()
    )?;
    writeln!(writer, "{label} digest: {}", profile.content_digest())
}

fn render_memory_mutation<W: Write>(
    view: &MemoryEntryMutationView,
    writer: &mut W,
) -> io::Result<()> {
    writer.write_all(b"Memory entry mutation:\n")?;
    render_entry_ref("Entry", &view.entry, writer)?;
    render_expired_proposals(&view.expired_proposals, writer)
}

fn render_proposal_created<W: Write>(
    view: &MemoryProposalCreatedView,
    writer: &mut W,
) -> io::Result<()> {
    writeln!(
        writer,
        "Memory proposal created: {}",
        view.proposal.proposal_id()
    )?;
    writeln!(writer, "Version: {}", view.proposal.version().get())?;
    writeln!(writer, "Status: {:?}", view.status)?;
    writeln!(writer, "Approval ID: {}", view.approval_id)?;
    writeln!(writer, "Digest: {}", view.proposal.content_digest())
}

fn render_proposal_resolution<W: Write>(
    view: &MemoryProposalResolutionView,
    writer: &mut W,
) -> io::Result<()> {
    let proposal = view.resolution.proposal();
    writeln!(
        writer,
        "Memory proposal resolved: {}",
        proposal.proposal_id()
    )?;
    writeln!(writer, "Version: {}", proposal.version().get())?;
    writeln!(writer, "Status: {:?}", view.resolution.status())?;
    writeln!(writer, "Approval ID: {}", view.resolution.approval_id())?;
    writeln!(
        writer,
        "Resolution event: {}",
        view.resolution.resolution_event_id()
    )?;
    writeln!(writer, "Resolved: {}", view.resolution.resolved_at_ms())?;
    writeln!(writer, "Digest: {}", proposal.content_digest())?;
    match &view.entry {
        Some(entry) => render_entry_ref("Entry", entry, writer)?,
        None => writer.write_all(b"Entry: none\n")?,
    }
    render_expired_proposals(&view.expired_proposals, writer)
}

fn render_memory_entry_detail<W: Write>(
    title: &str,
    profile: &crate::agents::AgentProfileVersionRef,
    entry: &crate::memory::MemoryEntryVersion,
    writer: &mut W,
) -> io::Result<()> {
    let reference = entry.reference();
    writeln!(writer, "{title}: {}", reference.entry_id())?;
    writeln!(writer, "Profile: {}", profile.profile_id())?;
    writeln!(writer, "Namespace: {}", reference.namespace_id())?;
    writeln!(writer, "Version: {}", reference.version().get())?;
    writeln!(writer, "Version ID: {}", reference.entry_version_id())?;
    writeln!(writer, "State: {:?}", reference.state())?;
    writeln!(writer, "Digest: {}", reference.content_digest())?;
    writeln!(
        writer,
        "Display key: {}",
        escaped_bounded_bytes(entry.display_key(), MAX_MEMORY_KEY_RENDER_BYTES),
    )?;
    match entry.value() {
        Some(value) => writeln!(
            writer,
            "Value: {}",
            escaped_bounded_bytes(value, MAX_MEMORY_VALUE_RENDER_BYTES),
        )?,
        None => writer.write_all(b"Value: none (deleted)\n")?,
    }
    writeln!(
        writer,
        "Purpose tags: {}",
        escaped_memory_list(entry.purpose_tags(), MAX_MEMORY_TAG_RENDER_BYTES),
    )?;
    writeln!(writer, "Created by: {:?}", entry.created_by())?;
    writeln!(writer, "Created: {}", entry.created_at_ms())?;
    writeln!(writer, "Creation event: {}", entry.creation_event_id())?;
    if let Some(proposal) = entry.accepted_proposal() {
        writeln!(
            writer,
            "Accepted proposal: {}@{}#{}",
            proposal.proposal_id(),
            proposal.version().get(),
            proposal.content_digest(),
        )?;
    }
    Ok(())
}

fn render_memory_candidate<W: Write>(
    candidate: &MemoryEntryDraft,
    writer: &mut W,
) -> io::Result<()> {
    writeln!(
        writer,
        "Candidate key: {}",
        escaped_bounded_bytes(candidate.display_key(), MAX_MEMORY_KEY_RENDER_BYTES),
    )?;
    writeln!(
        writer,
        "Candidate value: {}",
        escaped_bounded_bytes(candidate.value(), MAX_MEMORY_VALUE_RENDER_BYTES),
    )?;
    writeln!(
        writer,
        "Candidate purpose tags: {}",
        escaped_memory_list(candidate.purpose_tags(), MAX_MEMORY_TAG_RENDER_BYTES),
    )
}

fn render_expected_entry<W: Write>(
    label: &str,
    expected: &ExpectedMemoryEntryState,
    writer: &mut W,
) -> io::Result<()> {
    match expected {
        ExpectedMemoryEntryState::Absent => writeln!(writer, "{label}: absent"),
        ExpectedMemoryEntryState::Present(entry) | ExpectedMemoryEntryState::Deleted(entry) => {
            write!(writer, "{label}: ")?;
            render_entry_ref_inline(entry, writer)
        }
    }
}

fn render_entry_ref<W: Write>(
    label: &str,
    entry: &MemoryEntryRef,
    writer: &mut W,
) -> io::Result<()> {
    write!(writer, "{label}: ")?;
    render_entry_ref_inline(entry, writer)
}

fn render_entry_ref_inline<W: Write>(entry: &MemoryEntryRef, writer: &mut W) -> io::Result<()> {
    writeln!(
        writer,
        "{} version {} version-id {} state {:?} digest {}",
        entry.entry_id(),
        entry.version().get(),
        entry.entry_version_id(),
        entry.state(),
        entry.content_digest(),
    )
}

fn render_expired_proposals<W: Write>(
    proposals: &[MemoryProposalRef],
    writer: &mut W,
) -> io::Result<()> {
    writeln!(writer, "Expired proposals: {}", proposals.len())?;
    for proposal in proposals.iter().take(MAX_MEMORY_LIST_ROWS) {
        writeln!(
            writer,
            "  {} version {} digest {}",
            proposal.proposal_id(),
            proposal.version().get(),
            proposal.content_digest(),
        )?;
    }
    write_memory_omitted(
        writer,
        "expired proposals",
        rendered_omitted(0, proposals.len()),
    )
}

fn rendered_omitted(server_omitted: u64, row_count: usize) -> u64 {
    server_omitted.saturating_add(
        u64::try_from(row_count.saturating_sub(MAX_MEMORY_LIST_ROWS)).unwrap_or(u64::MAX),
    )
}

fn rendered_list_counts(server_omitted: u64, row_count: usize) -> (u64, u64) {
    let rendered = u64::try_from(row_count.min(MAX_MEMORY_LIST_ROWS)).unwrap_or(u64::MAX);
    (rendered, rendered_omitted(server_omitted, row_count))
}

fn write_memory_omitted<W: Write>(writer: &mut W, label: &str, omitted: u64) -> io::Result<()> {
    if omitted == 0 {
        Ok(())
    } else {
        writeln!(writer, "... {omitted} {label} omitted.")
    }
}

fn escaped_memory_list(values: &[String], maximum_bytes: usize) -> String {
    let mut rendered = values
        .iter()
        .take(8)
        .map(|value| escaped_bounded_bytes(value, maximum_bytes))
        .collect::<Vec<_>>();
    if values.len() > 8 {
        rendered.push("...".to_owned());
    }
    if rendered.is_empty() {
        "none".to_owned()
    } else {
        rendered.join(", ")
    }
}

fn escaped_bounded_bytes(value: &str, maximum_bytes: usize) -> String {
    let mut escaped = String::new();
    for character in value.chars() {
        let fragment = character.escape_default().to_string();
        if escaped.len().saturating_add(fragment.len()) > maximum_bytes {
            escaped.push_str("...");
            break;
        }
        escaped.push_str(&fragment);
    }
    escaped
}

fn render_draft<W: Write>(draft: &AgentProfileDraft, writer: &mut W) -> io::Result<()> {
    writeln!(
        writer,
        "  Display name: {}",
        escaped_bounded(&draft.display_name, 64)
    )?;
    writeln!(
        writer,
        "  Description: {}",
        escaped_bounded(&draft.description, 256)
    )?;
    writeln!(writer, "  Role: {}", draft.role.as_str())?;
    writeln!(
        writer,
        "  Primary specialty: {}",
        escaped_bounded(&draft.primary_specialty, 64)
    )?;
    writeln!(
        writer,
        "  Specialty tags: {}",
        escaped_list(&draft.specialty_tags, 48)
    )?;
    writeln!(
        writer,
        "  Personality: {}",
        escaped_bounded(&draft.personality, 1_024)
    )?;
    writeln!(
        writer,
        "  Instructions: {}",
        escaped_bounded(&draft.instructions, 4_096)
    )?;
    writeln!(writer, "  Bindings: {}", bindings_summary(&draft.bindings),)
}

fn render_profile_diffs<W: Write>(diffs: &[ProfileFieldDiff], writer: &mut W) -> io::Result<()> {
    for diff in diffs {
        writeln!(
            writer,
            "  {}: {} -> {}",
            diff_field(diff.field),
            field_value(&diff.before),
            field_value(&diff.after),
        )?;
    }
    Ok(())
}

fn draft_diffs(before: &AgentProfileDraft, after: &AgentProfileDraft) -> Vec<ProfileFieldDiff> {
    let mut diffs = Vec::new();
    push_text_diff(
        &mut diffs,
        ProfileDiffField::DisplayName,
        &before.display_name,
        &after.display_name,
    );
    push_text_diff(
        &mut diffs,
        ProfileDiffField::Description,
        &before.description,
        &after.description,
    );
    if before.role != after.role {
        diffs.push(ProfileFieldDiff {
            field: ProfileDiffField::Role,
            before: ProfileFieldValue::Role(before.role),
            after: ProfileFieldValue::Role(after.role),
        });
    }
    push_text_diff(
        &mut diffs,
        ProfileDiffField::PrimarySpecialty,
        &before.primary_specialty,
        &after.primary_specialty,
    );
    if before.specialty_tags != after.specialty_tags {
        diffs.push(ProfileFieldDiff {
            field: ProfileDiffField::SpecialtyTags,
            before: ProfileFieldValue::SpecialtyTags(before.specialty_tags.clone()),
            after: ProfileFieldValue::SpecialtyTags(after.specialty_tags.clone()),
        });
    }
    push_text_diff(
        &mut diffs,
        ProfileDiffField::Personality,
        &before.personality,
        &after.personality,
    );
    push_text_diff(
        &mut diffs,
        ProfileDiffField::Instructions,
        &before.instructions,
        &after.instructions,
    );
    if before.bindings != after.bindings {
        diffs.push(ProfileFieldDiff {
            field: ProfileDiffField::Bindings,
            before: ProfileFieldValue::Bindings(before.bindings.clone()),
            after: ProfileFieldValue::Bindings(after.bindings.clone()),
        });
    }
    diffs
}

fn push_text_diff(
    diffs: &mut Vec<ProfileFieldDiff>,
    field: ProfileDiffField,
    before: &str,
    after: &str,
) {
    if before != after {
        diffs.push(ProfileFieldDiff {
            field,
            before: ProfileFieldValue::Text(before.to_owned()),
            after: ProfileFieldValue::Text(after.to_owned()),
        });
    }
}

fn diff_field(field: ProfileDiffField) -> &'static str {
    match field {
        ProfileDiffField::DisplayName => "display_name",
        ProfileDiffField::Description => "description",
        ProfileDiffField::Role => "role",
        ProfileDiffField::PrimarySpecialty => "primary_specialty",
        ProfileDiffField::SpecialtyTags => "specialty_tags",
        ProfileDiffField::Personality => "personality",
        ProfileDiffField::Instructions => "instructions",
        ProfileDiffField::Bindings => "bindings",
        ProfileDiffField::SkillRefsAdded => "skill_refs_added",
        ProfileDiffField::SkillRefsUpgraded => "skill_refs_upgraded",
        ProfileDiffField::SkillRefsRemoved => "skill_refs_removed",
    }
}

fn field_value(value: &ProfileFieldValue) -> String {
    match value {
        ProfileFieldValue::Text(value) => escaped_bounded(value, 4_096),
        ProfileFieldValue::Role(value) => value.as_str().to_owned(),
        ProfileFieldValue::SpecialtyTags(values) => escaped_list(values, 48),
        ProfileFieldValue::Bindings(bindings) => bindings_summary(bindings),
    }
}

fn escaped_list(values: &[String], maximum_scalars: usize) -> String {
    if values.is_empty() {
        return "none".to_owned();
    }
    values
        .iter()
        .map(|value| escaped_bounded(value, maximum_scalars))
        .collect::<Vec<_>>()
        .join(", ")
}

fn escaped_skill_refs(values: &[SkillVersionRef]) -> String {
    let labels = values.iter().map(skill_ref_label).collect::<Vec<_>>();
    escaped_refs(labels.iter().map(String::as_str))
}

fn skill_ref_label(reference: &SkillVersionRef) -> String {
    format!(
        "{}@{}#{}",
        reference.skill_id(),
        reference.skill_version_id(),
        reference.version().get(),
    )
}

fn escaped_mcp_refs(values: &[McpRef]) -> String {
    escaped_refs(values.iter().map(McpRef::as_str))
}

fn escaped_refs<'a>(values: impl Iterator<Item = &'a str>) -> String {
    let values = values
        .map(|value| escaped_bounded(value, 256))
        .collect::<Vec<_>>();
    if values.is_empty() {
        "none".to_owned()
    } else {
        values.join(", ")
    }
}

fn readiness(readiness: AgentReadiness) -> &'static str {
    match readiness {
        AgentReadiness::Unbound => "unbound",
        AgentReadiness::BindingUnavailable => "binding_unavailable",
        AgentReadiness::Ready => "ready",
    }
}

fn bindings_summary(bindings: &AgentBindings) -> String {
    let inference = bindings.inference.as_ref().map_or_else(
        || "none".to_owned(),
        |binding| {
            format!(
                "{}/{}",
                escaped_bounded(binding.connection_id().as_str(), 128),
                escaped_bounded(binding.model_id().as_str(), 128)
            )
        },
    );
    let engineering = bindings.engineering.as_ref().map_or_else(
        || "none".to_owned(),
        |binding| escaped_bounded(binding.runtime_id().as_str(), 128),
    );
    format!("inference={inference} engineering={engineering}")
}

fn app_error_message(error: &AppError) -> &'static str {
    match error {
        AppError::Persistence(_) | AppError::Recovery(_) => "Application command failed.",
        AppError::CapabilityDenied { .. } | AppError::ApprovalRequired { .. } => {
            "Command is unavailable."
        }
        AppError::CommandConflict => "Command could not be completed.",
        AppError::Domain(_)
        | AppError::AgentProfileNotFound
        | AppError::DuplicateProfileName
        | AppError::AgentProfileHistoryMismatch
        | AppError::BindingReferenceUnavailable
        | AppError::StaleAgentProfileVersion
        | AppError::ProfileReviewUnavailable
        | AppError::ProfileReviewMismatch
        | AppError::ReviewDigestMismatch => "Agent profile operation could not be completed.",
        AppError::SkillNotFound
        | AppError::DuplicateSkillName
        | AppError::StaleSkillVersion
        | AppError::SkillReviewUnavailable
        | AppError::SkillReviewMismatch
        | AppError::SkillAlreadyAssigned
        | AppError::SkillNotAssigned
        | AppError::AgentSkillLimitExceeded => "Skill operation could not be completed.",
        AppError::MemoryEntryNotFound
        | AppError::MemoryProposalNotFound
        | AppError::EpisodicSummaryNotFound
        | AppError::WrongMemoryCommandDispatcher => "Memory operation could not be completed.",
        AppError::LifecycleFinished => "Application is shutting down.",
    }
}

fn escaped_bounded(value: &str, maximum_scalars: usize) -> String {
    let mut escaped = String::new();
    let mut scalars = 0;
    for character in value.chars() {
        let fragment = character.escape_default().to_string();
        let fragment_scalars = fragment.chars().count();
        if scalars + fragment_scalars > maximum_scalars {
            escaped.push_str("...");
            break;
        }
        escaped.push_str(&fragment);
        scalars += fragment_scalars;
    }
    escaped
}
use crate::agents::AgentBindings;
