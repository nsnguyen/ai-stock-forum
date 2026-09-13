//! Synthetic production-render preview for the shared two-pane TUI shell.
//!
//! Usage:
//! `cargo run --example two_pane_preview -- [--width N] [--height N]
//!      [--scene agents|empty|history|editor|type|invalid-field|review|confirmation|chat|connections]
//!      [--no-color] [--svg]`

use std::{env, fmt::Write as _, process};

use ai_stock_forum::{
    agents::{
        AgentProfileVersion, AgentReadiness, AgentRole, ProfileDiffField, ProfileEditPreview,
        ProfileFieldDiff, ProfileFieldValue, builtin_profile_templates,
    },
    app::{
        AgentProfileHistoryEntry, AgentProfileHistoryView, AgentProfileSummary,
        AgentProfileVersionView, AgentProfileView, AgentProfilesView, DatabaseReadiness,
        PresentationSnapshot, ProcessGuardOwnership,
    },
    domain::{
        AgentProfileId, AgentProfileVersionId, InstallationId, MemoryNamespaceId,
        ProfileReviewToken, SessionId, sha256,
    },
    setup::SetupStatus,
    ui::{
        profile_editor::{ProfileEditor, ProfileEditorEffect, ProfileTuiField},
        tui::{
            ControllerEffect, TuiEvent, handle_event,
            model::{AgentsPane, TuiModel, View},
            render,
            theme::Theme,
        },
    },
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Terminal,
    backend::TestBackend,
    buffer::Buffer,
    style::{Color, Modifier},
};
use uuid::Uuid;

const USAGE: &str = "Usage: two_pane_preview [--width N] [--height N] \
    [--scene agents|empty|history|editor|type|invalid-field|review|confirmation|chat|connections] \
    [--no-color] [--svg]";
const CELL_WIDTH: u16 = 8;
const CELL_HEIGHT: u16 = 16;
const TEXT_BASELINE: u16 = 13;
const SVG_RASTER_SCALE: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scene {
    Agents,
    Empty,
    History,
    Editor,
    Type,
    InvalidField,
    Review,
    Confirmation,
    Chat,
    Connections,
}

impl Scene {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "agents" => Ok(Self::Agents),
            "empty" => Ok(Self::Empty),
            "history" => Ok(Self::History),
            "editor" => Ok(Self::Editor),
            "type" => Ok(Self::Type),
            "invalid-field" => Ok(Self::InvalidField),
            "review" => Ok(Self::Review),
            "confirmation" => Ok(Self::Confirmation),
            "chat" => Ok(Self::Chat),
            "connections" => Ok(Self::Connections),
            _ => Err(format!("unknown scene {value:?}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Config {
    width: u16,
    height: u16,
    scene: Scene,
    no_color: bool,
    svg: bool,
    help: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            width: 120,
            height: 30,
            scene: Scene::Agents,
            no_color: false,
            svg: false,
            help: false,
        }
    }
}

fn parse_args<I, S>(args: I) -> Result<Config, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args = args
        .into_iter()
        .map(|arg| arg.as_ref().to_owned())
        .collect::<Vec<_>>();
    let mut config = Config::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--width" => config.width = positive_dimension(&args, &mut index, "--width")?,
            "--height" => config.height = positive_dimension(&args, &mut index, "--height")?,
            "--scene" => {
                let value = option_value(&args, &mut index, "--scene")?;
                config.scene = Scene::parse(value)?;
            }
            "--no-color" => config.no_color = true,
            "--svg" => config.svg = true,
            "--help" | "-h" => config.help = true,
            value => return Err(format!("unknown option {value:?}")),
        }
        index += 1;
    }
    Ok(config)
}

fn option_value<'a>(
    args: &'a [String],
    index: &mut usize,
    option: &str,
) -> Result<&'a str, String> {
    *index += 1;
    args.get(*index)
        .map(String::as_str)
        .ok_or_else(|| format!("{option} requires a value"))
}

fn positive_dimension(args: &[String], index: &mut usize, option: &str) -> Result<u16, String> {
    let value = option_value(args, index, option)?;
    let dimension = value
        .parse::<u16>()
        .map_err(|_| format!("{option} requires a positive 16-bit integer"))?;
    if dimension == 0 {
        return Err(format!("{option} must be greater than zero"));
    }
    Ok(dimension)
}

#[derive(Clone)]
struct ProfileFixtures {
    long_horizon_predecessor: AgentProfileVersion,
    long_horizon: AgentProfileVersion,
    bear: AgentProfileVersion,
    engineering: AgentProfileVersion,
}

fn profile_fixtures() -> ProfileFixtures {
    let long_horizon_predecessor = profile_from_template(
        0,
        10,
        "Long Horizon Analyst",
        "Builds patient, evidence-led investment theses.",
        "fundamental compounders",
        &["quality", "long-duration"],
        "Patient, skeptical, and explicit about uncertainty.",
        "Separate facts from assumptions and cite primary evidence.",
    );
    let mut next_draft = long_horizon_predecessor.to_draft();
    next_draft.description =
        "Builds patient, evidence-led theses with explicit disconfirming evidence.".to_owned();
    let long_horizon = AgentProfileVersion::next_version(
        &long_horizon_predecessor,
        AgentProfileVersionId::from_uuid(Uuid::from_u128(13)),
        1_800_000_000_001,
        next_draft,
    )
    .expect("valid synthetic profile version");
    let bear = profile_from_template(
        1,
        21,
        "Bear Researcher",
        "Stress-tests valuation, leverage, and competitive assumptions.",
        "downside research",
        &["risk", "valuation"],
        "Direct, skeptical, and evidence-led.",
        "Find the strongest falsifiable downside case.",
    );
    let engineering = profile_from_template(
        3,
        32,
        "Engineering Partner",
        "Builds reliable systems for repeatable investment research.",
        "research systems",
        &["tooling", "data-quality"],
        "Methodical, practical, and quality-focused.",
        "Improve research tooling while preserving auditability.",
    );
    ProfileFixtures {
        long_horizon_predecessor,
        long_horizon,
        bear,
        engineering,
    }
}

#[allow(clippy::too_many_arguments)]
fn profile_from_template(
    template_index: usize,
    id_base: u128,
    display_name: &str,
    description: &str,
    primary_specialty: &str,
    specialty_tags: &[&str],
    personality: &str,
    instructions: &str,
) -> AgentProfileVersion {
    let template = &builtin_profile_templates()[template_index];
    let mut draft = template.copy_to_draft().expect("valid builtin template");
    draft.display_name = display_name.to_owned();
    draft.description = description.to_owned();
    draft.primary_specialty = primary_specialty.to_owned();
    draft.specialty_tags = specialty_tags.iter().map(|tag| (*tag).to_owned()).collect();
    draft.personality = personality.to_owned();
    draft.instructions = instructions.to_owned();
    AgentProfileVersion::create(
        AgentProfileId::from_uuid(Uuid::from_u128(id_base)),
        AgentProfileVersionId::from_uuid(Uuid::from_u128(id_base + 1)),
        MemoryNamespaceId::from_uuid(Uuid::from_u128(id_base + 2)),
        1_800_000_000_000 + i64::try_from(id_base).expect("small fixture identity"),
        draft,
        Some(template.provenance()),
    )
    .expect("valid synthetic profile")
}

fn summary(profile: &AgentProfileVersion) -> AgentProfileSummary {
    AgentProfileSummary {
        profile_id: profile.profile_id(),
        profile_version_id: profile.profile_version_id(),
        version: profile.version(),
        display_name: profile.display_name().to_owned(),
        role: profile.role(),
        primary_specialty: profile.primary_specialty().to_owned(),
        readiness: AgentReadiness::Unbound,
        content_digest: profile.content_digest().clone(),
    }
}

fn populated_snapshot(fixtures: &ProfileFixtures) -> PresentationSnapshot {
    PresentationSnapshot {
        installation_id: InstallationId::from_uuid(Uuid::from_u128(1)),
        session_id: SessionId::from_uuid(Uuid::from_u128(2)),
        database_readiness: DatabaseReadiness::Ready,
        process_guard_ownership: ProcessGuardOwnership::Held,
        setup_status: SetupStatus::NotStarted,
        recent_audit: Vec::new(),
        agent_profiles: AgentProfilesView {
            profiles: vec![
                summary(&fixtures.long_horizon),
                summary(&fixtures.bear),
                summary(&fixtures.engineering),
            ],
            total_count: 3,
            returned_count: 3,
            truncated: false,
        },
        selected_agent_profile: Some(AgentProfileView {
            profile: fixtures.long_horizon.clone(),
            readiness: AgentReadiness::Unbound,
        }),
        selected_agent_profile_history: Some(AgentProfileHistoryView {
            profile_id: fixtures.long_horizon.profile_id(),
            active_version_id: fixtures.long_horizon.profile_version_id(),
            versions: vec![
                history_entry(&fixtures.long_horizon),
                history_entry(&fixtures.long_horizon_predecessor),
            ],
            total_count: 2,
            returned_count: 2,
            truncated: false,
        }),
    }
}

fn empty_snapshot() -> PresentationSnapshot {
    let fixtures = profile_fixtures();
    let mut snapshot = populated_snapshot(&fixtures);
    snapshot.agent_profiles = AgentProfilesView {
        profiles: Vec::new(),
        total_count: 0,
        returned_count: 0,
        truncated: false,
    };
    snapshot.selected_agent_profile = None;
    snapshot.selected_agent_profile_history = None;
    snapshot
}

fn history_entry(profile: &AgentProfileVersion) -> AgentProfileHistoryEntry {
    AgentProfileHistoryEntry {
        profile_version_id: profile.profile_version_id(),
        version: profile.version(),
        supersedes: profile.supersedes(),
        created_at_ms: profile.created_at_ms(),
        readiness: AgentReadiness::Unbound,
        content_digest: profile.content_digest().clone(),
    }
}

fn model_for_scene(scene: Scene) -> TuiModel {
    if scene == Scene::Empty {
        let mut model = TuiModel::new(empty_snapshot(), false);
        model.active_view = View::Agents;
        model.agents.pane = AgentsPane::List;
        return model;
    }

    let fixtures = profile_fixtures();
    let mut model = TuiModel::new(populated_snapshot(&fixtures), false);
    match scene {
        Scene::Agents => {
            model.active_view = View::Agents;
            model.agents.pane = AgentsPane::Detail;
        }
        Scene::History => {
            model.active_view = View::Agents;
            model.agents.pane = AgentsPane::History;
            model.agents.selected_history_version = 1;
            model.agents.history_scroll = 1;
            model.agents.version_detail = Some(AgentProfileVersionView {
                profile: fixtures.long_horizon_predecessor.clone(),
                readiness: AgentReadiness::Unbound,
                predecessor_diff: Vec::new(),
            });
        }
        Scene::Editor => {
            configure_editor(&mut model, &fixtures.long_horizon);
        }
        Scene::Type => {
            configure_editor(&mut model, &fixtures.long_horizon);
            press(&mut model, KeyCode::Enter);
        }
        Scene::InvalidField => {
            configure_editor(&mut model, &fixtures.long_horizon);
            press(&mut model, KeyCode::Enter);
            model.agents.field_input.clear();
            press(&mut model, KeyCode::Char(' '));
            press(&mut model, KeyCode::Esc);
        }
        Scene::Review => configure_review(&mut model, &fixtures.long_horizon),
        Scene::Confirmation => {
            configure_review(&mut model, &fixtures.long_horizon);
            press(&mut model, KeyCode::Enter);
        }
        Scene::Chat => model.active_view = View::Chat,
        Scene::Connections => model.active_view = View::Connections,
        Scene::Empty => unreachable!("empty scene returned above"),
    }
    model
}

fn configure_editor(model: &mut TuiModel, profile: &AgentProfileVersion) {
    let mut editor = ProfileEditor::for_edit(
        profile.profile_id(),
        profile.profile_version_id(),
        profile.to_draft(),
    );
    move_editor_to(&mut editor, ProfileTuiField::DisplayName);
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Editor;
    model.agents.editor = Some(editor);
    model.agents.synchronize_field_input();
}

fn configure_review(model: &mut TuiModel, profile: &AgentProfileVersion) {
    let mut draft = profile.to_draft();
    draft.display_name = "Long Horizon Lead".to_owned();
    draft.role = AgentRole::Chief;
    let mut editor =
        ProfileEditor::for_edit(profile.profile_id(), profile.profile_version_id(), draft);
    move_editor_to(&mut editor, ProfileTuiField::Review);
    let ProfileEditorEffect::PreviewEdit(request) = editor.submit_line(":review") else {
        panic!("synthetic editor reached review")
    };
    editor.apply_preview(
        request.generation,
        ProfileEditPreview {
            profile_id: request.profile_id,
            expected_active_version_id: request.expected_active_version_id,
            diffs: vec![
                ProfileFieldDiff {
                    field: ProfileDiffField::DisplayName,
                    before: ProfileFieldValue::Text(profile.display_name().to_owned()),
                    after: ProfileFieldValue::Text("Long Horizon Lead".to_owned()),
                },
                ProfileFieldDiff {
                    field: ProfileDiffField::Role,
                    before: ProfileFieldValue::Role(profile.role()),
                    after: ProfileFieldValue::Role(AgentRole::Chief),
                },
            ],
            review_token: ProfileReviewToken::from_uuid(Uuid::from_u128(40)),
            review_digest: sha256(b"synthetic-two-pane-preview-review"),
        },
    );
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Editor;
    model.agents.editor = Some(editor);
    model.agents.synchronize_field_input();
}

fn move_editor_to(editor: &mut ProfileEditor, target: ProfileTuiField) {
    while editor.tui_field() != target {
        let previous = editor.tui_field();
        editor.move_tui_field(true);
        assert_ne!(editor.tui_field(), previous, "target field is reachable");
    }
}

fn press(model: &mut TuiModel, code: KeyCode) {
    assert_eq!(
        handle_event(
            model,
            TuiEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
        ),
        ControllerEffect::Redraw,
        "synthetic interaction remains local"
    );
}

fn render_scene(config: Config) -> Result<Terminal<TestBackend>, String> {
    let mut model = model_for_scene(config.scene);
    model.set_terminal_size(config.width, config.height);
    let backend = TestBackend::new(config.width, config.height);
    let mut terminal = Terminal::new(backend).map_err(|error| error.to_string())?;
    terminal
        .draw(|frame| render::render(frame, &model, &Theme::from_no_color(config.no_color)))
        .map_err(|error| error.to_string())?;
    Ok(terminal)
}

fn cells_to_text(buffer: &Buffer) -> String {
    let width = usize::from(buffer.area.width);
    let mut output = String::new();
    for row in buffer.content.chunks(width) {
        for cell in row {
            output.push_str(cell.symbol());
        }
        output.push('\n');
    }
    output
}

fn cells_to_svg(buffer: &Buffer) -> String {
    let logical_width = u32::from(buffer.area.width) * u32::from(CELL_WIDTH);
    let logical_height = u32::from(buffer.area.height) * u32::from(CELL_HEIGHT);
    let raster_width = logical_width * SVG_RASTER_SCALE;
    let raster_height = logical_height * SVG_RASTER_SCALE;
    let default_fg = "#f4f4f5";
    let default_bg = "#18181b";
    let mut output = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{raster_width}\" \
         height=\"{raster_height}\" viewBox=\"0 0 {logical_width} {logical_height}\">\n\
         <rect width=\"{logical_width}\" height=\"{logical_height}\" fill=\"{default_bg}\"/>\n\
         <g font-family=\"Menlo,monospace\" font-size=\"13\">\n"
    );
    for (index, cell) in buffer.content.iter().enumerate() {
        let x = u32::try_from(index % usize::from(buffer.area.width)).expect("cell x fits")
            * u32::from(CELL_WIDTH);
        let y = u32::try_from(index / usize::from(buffer.area.width)).expect("cell y fits")
            * u32::from(CELL_HEIGHT);
        let mut fg = color_css(cell.fg, default_fg);
        let mut bg = color_css(cell.bg, default_bg);
        if cell.modifier.contains(Modifier::REVERSED) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if bg != default_bg {
            let _ = writeln!(
                output,
                "<rect x=\"{x}\" y=\"{y}\" width=\"{CELL_WIDTH}\" height=\"{CELL_HEIGHT}\" fill=\"{bg}\"/>"
            );
        }
        if cell.symbol().trim().is_empty() {
            continue;
        }
        let weight = if cell.modifier.contains(Modifier::BOLD) {
            "700"
        } else {
            "400"
        };
        let opacity = if cell.modifier.contains(Modifier::DIM) {
            "0.62"
        } else {
            "1"
        };
        let _ = writeln!(
            output,
            "<text x=\"{x}\" y=\"{}\" fill=\"{fg}\" font-weight=\"{weight}\" opacity=\"{opacity}\">{}</text>",
            y + u32::from(TEXT_BASELINE),
            xml_escape(cell.symbol())
        );
    }
    output.push_str("</g>\n</svg>\n");
    output
}

fn color_css(color: Color, reset: &str) -> String {
    match color {
        Color::Reset => reset.to_owned(),
        Color::Black => "#000000".to_owned(),
        Color::Red => "#cd3131".to_owned(),
        Color::Green => "#0dbc79".to_owned(),
        Color::Yellow => "#e5e510".to_owned(),
        Color::Blue => "#2472c8".to_owned(),
        Color::Magenta => "#bc3fbc".to_owned(),
        Color::Cyan => "#11a8cd".to_owned(),
        Color::Gray => "#e5e5e5".to_owned(),
        Color::DarkGray => "#666666".to_owned(),
        Color::LightRed => "#f14c4c".to_owned(),
        Color::LightGreen => "#23d18b".to_owned(),
        Color::LightYellow => "#f5f543".to_owned(),
        Color::LightBlue => "#3b8eea".to_owned(),
        Color::LightMagenta => "#d670d6".to_owned(),
        Color::LightCyan => "#29b8db".to_owned(),
        Color::White => "#ffffff".to_owned(),
        Color::Rgb(red, green, blue) => format!("#{red:02x}{green:02x}{blue:02x}"),
        Color::Indexed(index) => indexed_color(index),
    }
}

fn indexed_color(index: u8) -> String {
    const ANSI: [&str; 16] = [
        "#000000", "#800000", "#008000", "#808000", "#000080", "#800080", "#008080", "#c0c0c0",
        "#808080", "#ff0000", "#00ff00", "#ffff00", "#0000ff", "#ff00ff", "#00ffff", "#ffffff",
    ];
    if index < 16 {
        return ANSI[usize::from(index)].to_owned();
    }
    if index < 232 {
        const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];
        let offset = index - 16;
        let red = CUBE[usize::from(offset / 36)];
        let green = CUBE[usize::from((offset % 36) / 6)];
        let blue = CUBE[usize::from(offset % 6)];
        return format!("#{red:02x}{green:02x}{blue:02x}");
    }
    let gray = 8 + (index - 232) * 10;
    format!("#{gray:02x}{gray:02x}{gray:02x}")
}

fn xml_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        escaped.push_str(match character {
            '&' => "&amp;",
            '<' => "&lt;",
            '>' => "&gt;",
            '\"' => "&quot;",
            '\'' => "&apos;",
            _ => {
                escaped.push(character);
                continue;
            }
        });
    }
    escaped
}

fn main() {
    let config = match parse_args(env::args().skip(1)) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("error: {error}\n{USAGE}");
            process::exit(2);
        }
    };
    if config.help {
        println!("{USAGE}");
        return;
    }
    let terminal = match render_scene(config) {
        Ok(terminal) => terminal,
        Err(error) => {
            eprintln!("render failed: {error}");
            process::exit(1);
        }
    };
    let buffer = terminal.backend().buffer();
    if config.svg {
        print!("{}", cells_to_svg(buffer));
    } else {
        print!("{}", cells_to_text(buffer));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    #[test]
    fn parses_all_documented_preview_options() {
        let args = [
            "--width",
            "96",
            "--height",
            "24",
            "--scene",
            "history",
            "--no-color",
            "--svg",
        ];

        let config = parse_args(args).expect("valid preview options");

        assert_eq!(config.width, 96);
        assert_eq!(config.height, 24);
        assert_eq!(config.scene, Scene::History);
        assert!(config.no_color);
        assert!(config.svg);
    }

    #[test]
    fn rejects_unknown_scenes_and_zero_dimensions() {
        assert!(parse_args(["--scene", "portfolio"]).is_err());
        assert!(parse_args(["--width", "0"]).is_err());
        assert!(parse_args(["--height", "0"]).is_err());
    }

    #[test]
    fn xml_escape_covers_text_and_attribute_metacharacters() {
        assert_eq!(xml_escape("<&\"'>"), "&lt;&amp;&quot;&apos;&gt;");
    }

    #[test]
    fn svg_uses_native_raster_scale_and_fixed_cell_geometry() {
        let buffer = Buffer::empty(Rect::new(0, 0, 2, 2));

        let svg = cells_to_svg(&buffer);

        assert!(svg.contains("width=\"32\" height=\"64\" viewBox=\"0 0 16 32\""));
        assert!(svg.contains("<rect width=\"16\" height=\"32\" fill=\"#18181b\"/>"));
        assert!(svg.contains("font-family=\"Menlo,monospace\" font-size=\"13\""));
    }

    #[test]
    fn synthetic_agents_use_three_distinct_stable_monogram_colors() {
        let fixtures = profile_fixtures();
        let theme = Theme::from_no_color(false);
        let long_horizon = theme.agent_monogram(fixtures.long_horizon.profile_id()).fg;
        let bear = theme.agent_monogram(fixtures.bear.profile_id()).fg;
        let engineering = theme.agent_monogram(fixtures.engineering.profile_id()).fg;

        assert_ne!(long_horizon, bear);
        assert_ne!(long_horizon, engineering);
        assert_ne!(bear, engineering);
    }

    #[test]
    fn scenes_match_task_two_editor_and_history_selection_state() {
        use ai_stock_forum::ui::{profile_editor::ProfileTuiField, tui::model::InputMode};

        let editor = model_for_scene(Scene::Editor);
        assert_eq!(
            editor.agents.editor.as_ref().expect("editor").tui_field(),
            ProfileTuiField::DisplayName
        );
        assert_eq!(editor.input_mode, InputMode::Nav);
        assert_eq!(editor.agents.field_input.text(), "Long Horizon Analyst");

        let review = model_for_scene(Scene::Review);
        let review_editor = review.agents.editor.as_ref().expect("review editor");
        assert_eq!(review_editor.tui_field(), ProfileTuiField::Review);
        assert!(review_editor.review().is_some());

        let history = model_for_scene(Scene::History);
        let selected = &history.agents.history.as_ref().expect("history").versions
            [history.agents.selected_history_version];
        let detail = history
            .agents
            .version_detail
            .as_ref()
            .expect("version detail");
        assert_ne!(
            selected.profile_version_id,
            history.agents.profiles.profiles[0].profile_version_id
        );
        assert_eq!(
            detail.profile.profile_version_id(),
            selected.profile_version_id
        );
    }

    #[test]
    fn interaction_scenes_use_real_controller_state() {
        use ai_stock_forum::ui::{
            profile_editor::ProfileTuiField,
            tui::model::{AgentsPane, InputMode},
        };

        for (name, scene) in [
            ("type", Scene::Type),
            ("invalid-field", Scene::InvalidField),
            ("confirmation", Scene::Confirmation),
        ] {
            assert_eq!(Scene::parse(name), Ok(scene));
        }

        let typing = model_for_scene(Scene::Type);
        assert_eq!(typing.input_mode, InputMode::Type);
        assert_eq!(
            typing
                .agents
                .editor
                .as_ref()
                .expect("typing editor")
                .tui_field(),
            ProfileTuiField::DisplayName
        );
        assert_eq!(
            typing.agents.field_input.cursor_byte(),
            "Long Horizon Analyst".len()
        );

        let invalid = model_for_scene(Scene::InvalidField);
        let invalid_editor = invalid.agents.editor.as_ref().expect("invalid editor");
        assert_eq!(invalid.input_mode, InputMode::Nav);
        assert_eq!(invalid_editor.tui_field(), ProfileTuiField::DisplayName);
        assert_eq!(
            invalid_editor.tui_field_text(ProfileTuiField::DisplayName),
            " "
        );
        assert!(
            invalid_editor
                .tui_field_error(ProfileTuiField::DisplayName)
                .is_some()
        );

        let confirmation = model_for_scene(Scene::Confirmation);
        assert_eq!(confirmation.agents.pane, AgentsPane::Confirmation);
        assert!(confirmation.agents.pending_confirmation.is_some());
    }
}
