use std::io::{self, Write};

use crate::{
    agents::{
        AgentProfileDraft, AgentReadiness, McpRef, ProfileDiffField, ProfileFieldDiff,
        ProfileFieldValue, ProfileTemplate, SkillRef,
    },
    app::{
        AppError, CommandOutcome, CommandView, InputRejectionCategory, ShutdownDisposition,
        ShutdownReason,
    },
    cli::CliError,
    config::StartupError,
    domain::Actor,
    runtime::RuntimeError,
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
                b"Available commands:\n  /help\n  /status\n  /setup status\n  /audit tail [limit: 1-100]\n  /quit\n",
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
                    writer.write_all(b"Input rejected: malformed command.\n")
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
                    "Bindings: provider={} model={}",
                    escaped_option(profile.bindings().model_provider.as_deref(), 256),
                    escaped_option(profile.bindings().model_name.as_deref(), 256)
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
                let omitted = view.versions.len().saturating_sub(MAX_PROFILE_HISTORY_ROWS);
                if omitted > 0 {
                    writeln!(writer, "... {omitted} versions omitted.")?;
                }
                Ok(())
            }
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
                }
                ProfileEditorMode::Edit { .. } => {
                    if let Some(review) = editor.review() {
                        render_profile_diffs(&review.preview().diffs, writer)?;
                    } else {
                        writer.write_all(b"  Run :review to request a passive preview.\n")?;
                    }
                }
            }
        }
        if let Some(message) = editor.local_message() {
            writeln!(writer, "Editor message: {}", message.code())?;
        }
        writer.write_all(
            b"Controls: :role <bull|bear|chief|engineering|custom> :next :back :show :clear :review :activate :cancel\n",
        )
    }

    pub fn render_activation_confirmation<W: Write>(writer: &mut W) -> io::Result<()> {
        writer.write_all(b"Confirm activation? [y/yes or n/no]\n")
    }

    pub fn render_activation_declined<W: Write>(writer: &mut W) -> io::Result<()> {
        writer.write_all(b"Activation declined; returned to review.\n")
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
    writeln!(writer, "  Display name: {}", escaped_bounded(&draft.display_name, 64))?;
    writeln!(writer, "  Description: {}", escaped_bounded(&draft.description, 256))?;
    writeln!(writer, "  Role: {}", draft.role.as_str())?;
    writeln!(writer, "  Primary specialty: {}", escaped_bounded(&draft.primary_specialty, 64))?;
    writeln!(writer, "  Specialty tags: {}", escaped_list(&draft.specialty_tags, 48))?;
    writeln!(writer, "  Personality: {}", escaped_bounded(&draft.personality, 1_024))?;
    writeln!(writer, "  Instructions: {}", escaped_bounded(&draft.instructions, 4_096))?;
    writeln!(
        writer,
        "  Bindings: provider={} model={}",
        escaped_option(draft.bindings.model_provider.as_deref(), 256),
        escaped_option(draft.bindings.model_name.as_deref(), 256),
    )
}

fn render_profile_diffs<W: Write>(
    diffs: &[ProfileFieldDiff],
    writer: &mut W,
) -> io::Result<()> {
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
    }
}

fn field_value(value: &ProfileFieldValue) -> String {
    match value {
        ProfileFieldValue::Text(value) => escaped_bounded(value, 4_096),
        ProfileFieldValue::Role(value) => value.as_str().to_owned(),
        ProfileFieldValue::SpecialtyTags(values) => escaped_list(values, 48),
        ProfileFieldValue::Bindings(bindings) => format!(
            "provider={} model={}",
            escaped_option(bindings.model_provider.as_deref(), 256),
            escaped_option(bindings.model_name.as_deref(), 256),
        ),
    }
}

fn escaped_option(value: Option<&str>, maximum_scalars: usize) -> String {
    value
        .map(|value| escaped_bounded(value, maximum_scalars))
        .unwrap_or_else(|| "none".to_owned())
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

fn escaped_skill_refs(values: &[SkillRef]) -> String {
    escaped_refs(values.iter().map(SkillRef::as_str))
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
        AgentReadiness::Ready => "ready",
        AgentReadiness::NotReady => "not_ready",
    }
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
        | AppError::StaleAgentProfileVersion
        | AppError::ProfileReviewUnavailable
        | AppError::ProfileReviewMismatch
        | AppError::ReviewDigestMismatch => "Agent profile operation could not be completed.",
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
