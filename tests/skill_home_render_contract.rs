use ai_stock_forum::{
    app::{
        AgentProfilesView, DatabaseReadiness, PresentationSnapshot, ProcessGuardOwnership,
        SkillSummary, SkillView, SkillsView,
    },
    domain::{InstallationId, SessionId, SkillId, SkillVersionId},
    setup::SetupStatus,
    skills::{SkillDraft, SkillProvenance, SkillVersion},
    ui::{
        skill_editor::{SkillEditor, SkillEditorField},
        tui::{
            model::{Focus, InputMode, SkillEditorPage, SkillSection, SkillsPane, TuiModel},
            render,
            theme::Theme,
        },
    },
};
use ratatui::{Terminal, backend::TestBackend};
use uuid::Uuid;

fn fixture() -> TuiModel {
    let mut model = TuiModel::new(
        PresentationSnapshot {
            installation_id: InstallationId::from_uuid(Uuid::from_u128(1)),
            session_id: SessionId::from_uuid(Uuid::from_u128(2)),
            database_readiness: DatabaseReadiness::Ready,
            process_guard_ownership: ProcessGuardOwnership::Held,
            setup_status: SetupStatus::NotStarted,
            recent_audit: vec![],
            agent_profiles: AgentProfilesView {
                profiles: vec![],
                total_count: 0,
                returned_count: 0,
                truncated: false,
            },
            selected_agent_profile: None,
            selected_agent_profile_history: None,
        },
        false,
    );
    let version = SkillVersion::create(
        SkillId::from_uuid(Uuid::from_u128(10)),
        SkillVersionId::from_uuid(Uuid::from_u128(11)),
        100,
        SkillProvenance::User,
        SkillDraft::new(
            "Catalyst Mapping".into(),
            "Find upcoming events that change the thesis.".into(),
            "Before earnings and launches.".into(),
            vec!["research".into()],
            "List each catalyst and supporting evidence.".into(),
            vec![],
        )
        .unwrap(),
    )
    .unwrap();
    model.skills.active = true;
    model.skills.pane = SkillsPane::Detail;
    model.focus = Focus::Workspace;
    model.skills.library = SkillsView {
        skills: vec![SkillSummary {
            skill_ref: version.reference(),
            display_name: version.content().display_name.clone(),
            provenance: version.provenance().clone(),
        }],
        total_count: 1,
        returned_count: 1,
        truncated: false,
    };
    model.skills.detail = Some(SkillView {
        skill_ref: version.reference(),
        content: version.content().clone(),
        created_at_ms: 100,
        provenance: version.provenance().clone(),
        predecessor_version_id: None,
    });
    model
}

fn terminal(model: &TuiModel, width: u16, height: u16, no_color: bool) -> Terminal<TestBackend> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| render::render(frame, model, &Theme::from_no_color(no_color)))
        .unwrap();
    terminal
}

fn screen(model: &TuiModel, width: u16, height: u16) -> String {
    terminal(model, width, height, true)
        .backend()
        .buffer()
        .content()
        .chunks(usize::from(width))
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

// A missing Home action or guidance region prevents discovering the corresponding operation.
#[test]
fn loaded_home_exposes_actions_and_guidance_without_technical_identifiers() {
    let model = fixture();
    let text = screen(&model, 150, 42);
    for expected in [
        "SKILL HOME",
        "Assign to agent",
        "Edit skill",
        "Version history",
        "WHEN TO USE",
        "Before earnings and launches.",
        "List each catalyst and supporting evidence.",
    ] {
        assert!(text.contains(expected), "missing {expected}\n{text}");
    }
    let detail = model.skills.detail.as_ref().unwrap();
    for hidden in [
        detail.skill_ref.skill_id().to_string(),
        detail.skill_ref.skill_version_id().to_string(),
        detail.skill_ref.content_digest().to_string(),
    ] {
        assert!(
            !text.contains(&hidden),
            "everyday Home leaked technical identifier"
        );
    }
    assert!(!text.contains("Up/Down"));
    assert!(!text.contains("Left/Right"));
}

// An editor that shows the sequential wizard instead of section cards hides direct edits.
#[test]
fn editor_home_offers_independent_sections_for_a_partial_draft() {
    let mut model = fixture();
    model.skills.pane = SkillsPane::Editor;
    model.skills.editor = Some(SkillEditor::for_create(None));
    let text = screen(&model, 150, 42);
    for expected in [
        "Basics",
        "When to use",
        "Instructions",
        "Reference notes",
        "Review changes",
        "Discard draft",
        "DRAFT",
    ] {
        assert!(text.contains(expected), "missing {expected}\n{text}");
    }
    assert!(!text.contains("Step 1 of 5"));
}

// Fixed list prefixes used to leave a selected item below the viewport unreachable visually.
#[test]
fn compact_library_keeps_the_selected_skill_visible() {
    let mut model = fixture();
    let summary = model.skills.library.skills[0].clone();
    model.skills.library.skills = (0..30)
        .map(|index| {
            let mut item = summary.clone();
            item.display_name = format!("Research skill {index:02}");
            item
        })
        .collect();
    model.skills.library.total_count = 30;
    model.skills.library.returned_count = 30;
    model.skills.selected_skill = 29;
    model.skills.pane = SkillsPane::List;
    model.focus = Focus::List;
    let text = screen(&model, 60, 18);
    assert!(
        text.contains("Research skill 29"),
        "selected item lost\n{text}"
    );
    assert!(text.contains("New skill"));
}

// A copy picker must remain separate from the library and never preview another selection.
#[test]
fn new_skill_has_a_separate_starting_point_and_preview() {
    let mut model = fixture();
    model.skills.pane = SkillsPane::CreateSource;
    model.skills.selected_create_source = 0;
    let text = screen(&model, 150, 42);
    for expected in [
        "STARTING POINT",
        "STARTER PREVIEW",
        "Blank skill",
        "Copy Catalyst Mapping",
        "Start from scratch",
    ] {
        assert!(text.contains(expected), "missing {expected}\n{text}");
    }
    assert!(!text.contains("Skill library"));
    assert!(!text.contains("Before earnings and launches."));
}

#[test]
fn normal_starter_picker_keeps_blank_and_four_available_copies_visible() {
    let mut model = fixture();
    let summary = model.skills.library.skills[0].clone();
    model.skills.library.skills = [
        "Evidence Review",
        "Filing Analysis",
        "Catalyst Mapping",
        "Risk Checklist",
    ]
    .iter()
    .map(|name| {
        let mut item = summary.clone();
        item.display_name = (*name).to_owned();
        item
    })
    .collect();
    model.skills.pane = SkillsPane::CreateSource;
    model.skills.selected_create_source = 3;
    model.focus = Focus::List;
    let text = screen(&model, 120, 30);
    for label in [
        "Blank skill",
        "Evidence Review",
        "Filing Analysis",
        "Catalyst Mapping",
        "Risk Checklist",
    ] {
        assert!(text.contains(label), "missing {label}\n{text}");
    }
}

// Color-independent selection must survive NO_COLOR without introducing explicit colors.
#[test]
fn no_color_home_keeps_selection_and_never_sets_rgb_colors() {
    let model = fixture();
    let terminal = terminal(&model, 150, 42, true);
    assert!(
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .all(|cell| cell.fg == ratatui::style::Color::Reset
                && cell.bg == ratatui::style::Color::Reset)
    );
    assert!(screen(&model, 150, 42).contains("> Assign to agent"));
}

#[test]
fn compact_editor_keeps_each_selected_action_visible() {
    let mut model = fixture();
    model.skills.pane = SkillsPane::Editor;
    model.skills.editor = Some(SkillEditor::for_create(None));
    for (index, label) in [
        "Basics",
        "When to use",
        "Instructions",
        "Reference notes",
        "Review changes",
        "Discard draft",
    ]
    .iter()
    .enumerate()
    {
        model.skills.editor_home_selection = index;
        assert!(screen(&model, 60, 18).contains(&format!("> {label}")));
    }
}

#[test]
fn source_preview_rejects_loaded_content_from_another_exact_version() {
    let mut model = fixture();
    model.skills.pane = SkillsPane::CreateSource;
    model.skills.selected_create_source = 1;
    let mut unrelated = model.skills.detail.clone().unwrap();
    unrelated.skill_ref = SkillVersion::create(
        SkillId::from_uuid(Uuid::from_u128(20)),
        SkillVersionId::from_uuid(Uuid::from_u128(21)),
        101,
        SkillProvenance::User,
        unrelated.content.clone(),
    )
    .unwrap()
    .reference();
    unrelated.content.description = "STALE PREVIEW MUST NOT APPEAR".into();
    model.skills.create_source_detail = Some(unrelated);
    let text = screen(&model, 150, 42);
    assert!(text.contains("Loading selected starting point"));
    assert!(!text.contains("STALE PREVIEW MUST NOT APPEAR"));
    model.skills.create_source_detail = model.skills.detail.clone();
    assert!(screen(&model, 150, 42).contains("Before earnings and launches."));
}

#[test]
fn long_home_guidance_can_be_scrolled_to_its_end_while_action_stays_visible() {
    let mut model = fixture();
    model.skills.detail.as_mut().unwrap().content.instructions = format!(
        "{}END OF LONG GUIDANCE",
        "Read the supporting evidence. ".repeat(150)
    );
    assert!(!screen(&model, 100, 30).contains("END OF LONG GUIDANCE"));
    model.skills.content_scroll = u16::MAX;
    let text = screen(&model, 100, 30);
    assert!(text.contains("END OF LONG GUIDANCE"));
    assert!(text.contains("Assign to agent"));
    assert!(text.contains("Enter: open"));
}

#[test]
fn review_is_named_and_does_not_leak_default_candidate_digests() {
    let mut model = fixture();
    let mut editor =
        SkillEditor::for_create(Some(model.skills.detail.as_ref().unwrap().content.clone()));
    editor.go_to_review().unwrap();
    model.skills.editor = Some(editor);
    model.skills.editor_page = SkillEditorPage::Review;
    model.skills.pane = SkillsPane::Editor;
    let text = screen(&model, 150, 42);
    assert!(text.contains("REVIEW CHANGES"));
    assert!(text.contains("Catalyst Mapping"));
    assert!(text.contains("v1"));
    assert!(!text.contains("Step 5 of 5"));
}

#[test]
fn end_navigation_reaches_long_editor_instructions() {
    let mut model = fixture();
    model.set_terminal_size(80, 24);
    let mut draft = model.skills.detail.as_ref().unwrap().content.clone();
    draft.instructions = format!(
        "{}EDITOR-INSTRUCTION-END",
        "Inspect the evidence. ".repeat(150)
    );
    let mut editor = SkillEditor::for_create(Some(draft));
    editor.select_tui_field(SkillEditorField::Instructions);
    model.skills.editor = Some(editor);
    model.skills.editor_page = SkillEditorPage::Section(SkillSection::Instructions);
    model.skills.pane = SkillsPane::Editor;
    ai_stock_forum::ui::tui::handle_event(
        &mut model,
        ai_stock_forum::ui::tui::TuiEvent::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::End,
            crossterm::event::KeyModifiers::NONE,
        )),
    );
    let text = screen(&model, 80, 24);
    assert!(
        text.contains("EDITOR-INSTRUCTION-END"),
        "scroll={}\n{text}",
        model.skills.content_scroll
    );
}

#[test]
fn long_field_input_keeps_the_caret_and_recent_text_visible() {
    let mut model = fixture();
    model.skills.editor = Some(SkillEditor::for_create(None));
    model
        .skills
        .editor
        .as_mut()
        .unwrap()
        .select_tui_field(SkillEditorField::Instructions);
    model.skills.editor_page = SkillEditorPage::Section(SkillSection::Instructions);
    model.skills.pane = SkillsPane::Editor;
    model
        .skills
        .field_input
        .ingest(&format!("{}CARET END", "guidance ".repeat(100)));
    model.input_mode = InputMode::Type;
    assert!(screen(&model, 60, 18).contains("CARET END|"));
}

#[test]
fn long_reference_body_does_not_hide_selected_save_action() {
    let mut model = fixture();
    let mut editor = SkillEditor::for_create(None);
    editor.begin_add_reference();
    editor.set_tui_field(SkillEditorField::ReferenceName, "Supporting note");
    editor.set_tui_field(
        SkillEditorField::ReferenceBody,
        &"Evidence detail. ".repeat(200),
    );
    model.skills.editor = Some(editor);
    model.skills.editor_page = SkillEditorPage::ReferenceEdit;
    model.skills.reference_action = 2;
    model.skills.pane = SkillsPane::Editor;
    assert!(screen(&model, 60, 18).contains("> Save note"));
}

#[test]
fn review_end_reaches_all_guidance_before_confirmation() {
    let mut model = fixture();
    model.set_terminal_size(80, 24);
    let mut draft = model.skills.detail.as_ref().unwrap().content.clone();
    draft.instructions = format!("{}REVIEW-GUIDANCE-END", "Check the source. ".repeat(150));
    let mut editor = SkillEditor::for_create(Some(draft));
    editor.go_to_review().unwrap();
    model.skills.editor = Some(editor);
    model.skills.editor_page = SkillEditorPage::Review;
    model.skills.pane = SkillsPane::Editor;
    ai_stock_forum::ui::tui::handle_event(
        &mut model,
        ai_stock_forum::ui::tui::TuiEvent::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::End,
            crossterm::event::KeyModifiers::NONE,
        )),
    );
    let text = screen(&model, 80, 24);
    assert!(text.contains("REVIEW-GUIDANCE-END"), "{text}");
    assert!(text.contains("Enter: request validation review"));
}
