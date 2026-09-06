use std::io::{self, Write};

use crate::{
    agents::{
        AgentProfileDraft, AgentReadiness, McpRef, ProfileDiffField, ProfileFieldDiff,
        ProfileFieldValue, ProfileTemplate,
    },
    app::{
        AgentSkillAssignmentOperation, AgentSkillAssignmentPreview, AppError, ApplicationCommand,
        CommandOutcome, CommandView, InputRejectionCategory, SafeToken, ShutdownDisposition,
        ShutdownReason,
    },
    cli::CliError,
    config::StartupError,
    domain::Actor,
    runtime::RuntimeError,
    skills::{SkillDraft, SkillEditPreview, SkillVersionRef},
    setup::SetupStatus,
    ui::{
        profile_editor::{ProfileEditor, ProfileEditorMode, ProfileEditorStep},
        tui::TuiError,
    },
};

const MAX_PROFILE_LIST_ROWS: usize = 100;
const MAX_PROFILE_HISTORY_ROWS: usize = 100;

pub struct TextRenderer;

impl TextRenderer {
    pub fn render_outcome<W: Write>(outcome: &CommandOutcome, writer: &mut W) -> io::Result<()> {
        Self::render_view(&outcome.view, writer)
    }

    pub fn render_view<W: Write>(view: &CommandView, writer: &mut W) -> io::Result<()> {
        match view {
            CommandView::Help(_) => writer.write_all(
                b"Available commands:\n  /help\n  /status\n  /setup status\n  /audit tail [limit: 1-100]\n  /skill list\n  /skills\n  /skill add\n  /skill show <name-or-id> [version]\n  /skill assign <skill> <agent> [version]\n  /skill unassign <skill> <agent>\n  /quit\n",
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
                    if matches!(
                        view.rejection.safe_token.as_ref().map(SafeToken::as_str),
                        Some("/skill" | "/skills")
                    ) {
                        writer.write_all(b"Usage: /skill list | /skill add | /skill show <name-or-id> [version] | /skill assign <skill> <agent> [version] | /skill unassign <skill> <agent>\n")
                    } else {
                        Ok(())
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

    pub fn render_skill_editor<W: Write>(
        draft: &SkillDraft,
        writer: &mut W,
    ) -> io::Result<()> {
        writer.write_all(b"Create skill editor\n")?;
        writeln!(writer, "  Display name: {}", escaped_bounded(&draft.display_name, 64))?;
        writeln!(writer, "  Description: {}", escaped_bounded(&draft.description, 256))?;
        writeln!(writer, "  Use when: {}", escaped_bounded(&draft.use_when, 512))?;
        writeln!(writer, "  Tags: {}", escaped_list(&draft.tags, 32))?;
        writeln!(writer, "  Instructions: {}", escaped_bounded(&draft.instructions, 4_096))?;
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
            preview.profile_id,
            preview.expected_active_profile_version_id,
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
