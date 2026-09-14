use ai_stock_forum::{
    agents::builtin_profile_templates,
    app::{AgentProfilesView, DatabaseReadiness, PresentationSnapshot, ProcessGuardOwnership},
    domain::{InstallationId, SessionId},
    setup::SetupStatus,
    ui::{
        profile_editor::{ProfileEditor, ProfileTuiField},
        tui::{
            model::{AgentsPane, Focus, ProfileEditorPage, ProfileSection, TuiModel, View},
            render,
            theme::Theme,
        },
    },
};
use ratatui::{Terminal, backend::TestBackend};
use uuid::Uuid;

fn model() -> TuiModel {
    let snapshot = PresentationSnapshot {
        installation_id: InstallationId::from_uuid(Uuid::from_u128(1)),
        session_id: SessionId::from_uuid(Uuid::from_u128(2)),
        database_readiness: DatabaseReadiness::Ready,
        process_guard_ownership: ProcessGuardOwnership::Held,
        setup_status: SetupStatus::NotStarted,
        recent_audit: Vec::new(),
        agent_profiles: AgentProfilesView {
            profiles: Vec::new(),
            total_count: 0,
            returned_count: 0,
            truncated: false,
        },
        selected_agent_profile: None,
        selected_agent_profile_history: None,
    };
    let mut model = TuiModel::new(snapshot, false);
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Editor;
    model.agents.editor = Some(ProfileEditor::for_create(&builtin_profile_templates()[0]).unwrap());
    model.agents.editor_page = ProfileEditorPage::Home;
    model.focus = Focus::Workspace;
    model
}

fn screen(model: &mut TuiModel, width: u16, height: u16, no_color: bool) -> String {
    model.set_terminal_size(width, height);
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| render::render(frame, model, &Theme::from_no_color(no_color)))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[test]
fn profile_home_replaces_the_long_field_list_with_four_sections_and_two_actions() {
    // Regression caught: routing Home through the legacy 11-field editor list.
    let mut model = model();
    let rendered = screen(&mut model, 120, 30, false);

    for label in [
        "Make this agent yours.",
        "New agent / Bull Researcher · DRAFT NOT APPLIED",
        "Identity",
        "Focus",
        "Personality",
        "Instructions",
        "Review changes",
        "Discard draft",
        "Template used: Bull Researcher",
        "reference only",
        "bull · Develops evidence-led",
        "upside research",
        "catalysts, growth",
        "Constructive,",
        "Develop the strongest",
    ] {
        assert!(rendered.contains(label), "missing home label: {label}");
    }
    assert!(!rendered.contains("› Template"));
    assert!(!rendered.contains("› Bindings"));
    for icon in ["(o)", "-(o)-", ".---.", "[===]"] {
        assert!(
            rendered.contains(icon),
            "missing ASCII section icon: {icon}"
        );
    }
    assert!(!rendered.contains('✓'));
}

#[test]
fn new_agent_templates_replace_the_agent_list_and_preview_without_creating_anything() {
    // Regression caught: showing the saved-agent list beside a new-agent draft.
    let mut model = model();
    model.agents.editor_page = ProfileEditorPage::Templates;
    let rendered = screen(&mut model, 120, 30, false);

    for label in [
        "NEW AGENT / TEMPLATES",
        "Bull Researcher",
        "Bear Researcher",
        "Chief Moderator",
        "TEMPLATE PREVIEW",
        "Use this template",
        "No agent is created until you review and confirm.",
        "\\       //",
    ] {
        assert!(rendered.contains(label), "missing template label: {label}");
    }
    assert!(!rendered.contains("Agent list"));
    assert!(!rendered.contains("No agent profiles yet"));
}

#[test]
fn compact_template_picker_respects_the_current_one_pane_focus() {
    // Regression caught: painting both template panes into an unreadable compact viewport.
    let mut model = model();
    model.agents.editor_page = ProfileEditorPage::Templates;
    model.focus = Focus::List;
    let list = screen(&mut model, 60, 18, true);
    assert!(list.contains("NEW AGENT / TEMPLATES"));
    assert!(list.contains("Bull Researcher"));
    assert!(!list.contains("TEMPLATE PREVIEW"));

    model.focus = Focus::Workspace;
    let preview = screen(&mut model, 60, 18, true);
    assert!(preview.contains("TEMPLATE PREVIEW"));
    assert!(preview.contains("Use this template"));
    assert!(!preview.contains("NEW AGENT / TEMPLATES"));
}

#[test]
fn section_page_renders_only_its_owned_fields_and_keeps_source_context() {
    // Regression caught: opening a section but retaining unrelated fields from the old editor.
    let mut model = model();
    model.agents.editor_page = ProfileEditorPage::Section(ProfileSection::Focus);
    model
        .agents
        .editor
        .as_mut()
        .unwrap()
        .select_tui_field(ProfileTuiField::PrimarySpecialty);
    let rendered = screen(&mut model, 80, 24, false);

    for label in [
        "FOCUS",
        "Focus / Bull Researcher · DRAFT NOT APPLIED",
        "Primary specialty",
        "Tags",
        "Template used: Bull Researcher",
        "Bindings",
        "read-only",
    ] {
        assert!(rendered.contains(label), "missing section label: {label}");
    }
    for unrelated in ["Display name", "Personality", "Instructions"] {
        assert!(
            !rendered.contains(unrelated),
            "unrelated section field leaked: {unrelated}"
        );
    }
}

#[test]
fn compact_home_auto_reveals_every_selected_action_in_color_and_no_color_modes() {
    // Regression caught: lower home actions becoming unreachable below the first card row.
    let mut model = model();
    for no_color in [false, true] {
        for (selection, label) in [
            (0, "Identity"),
            (1, "Focus"),
            (2, "Personality"),
            (3, "Instructions"),
            (4, "Review changes"),
            (5, "Discard draft"),
        ] {
            model.agents.profile_home_selection = selection;
            let rendered = screen(&mut model, 60, 18, no_color);
            assert!(
                rendered.contains(label),
                "selection {selection} hidden with no_color={no_color}"
            );
        }
    }
}

#[test]
fn compact_selected_field_prioritizes_its_validation_error_over_generic_guidance() {
    // Regression caught: the two-line compact card silently clipping its validation error.
    let mut model = model();
    model.agents.editor_page = ProfileEditorPage::Section(ProfileSection::Identity);
    let editor = model.agents.editor.as_mut().unwrap();
    editor.select_tui_field(ProfileTuiField::DisplayName);
    assert!(!editor.set_tui_field(ProfileTuiField::DisplayName, "   "));

    let rendered = screen(&mut model, 60, 18, true);
    assert!(rendered.contains("Display name: invalid"));
    assert!(rendered.contains("Please revise this field before reviewing."));
    assert!(!rendered.contains("Display name · Up to"));
}
