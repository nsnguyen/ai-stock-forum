use ai_stock_forum::{
    agents::{
        AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentReadiness, AgentRole,
        DESCRIPTION_MAX_BYTES, DISPLAY_NAME_MAX_BYTES, INSTRUCTIONS_MAX_BYTES,
        PERSONALITY_MAX_BYTES, PRIMARY_SPECIALTY_MAX_BYTES, ProfileDiffField, ProfileEditPreview,
        ProfileFieldDiff, ProfileFieldValue, builtin_profile_templates,
    },
    app::{
        AgentProfileHistoryEntry, AgentProfileHistoryView, AgentProfileSummary,
        AgentProfileVersionView, AgentProfileView, AgentProfilesView, ApplicationCommand,
        DatabaseReadiness, PresentationSnapshot, ProcessGuardOwnership,
    },
    domain::{
        AgentProfileId, AgentProfileVersionId, InstallationId, MemoryNamespaceId,
        ProfileReviewToken, SessionId, sha256,
    },
    setup::SetupStatus,
    ui::{
        profile_editor::{ProfileEditor, ProfileEditorEffect},
        tui::{
            ControllerEffect, ProfileConfirmation, TuiEvent, handle_event,
            layout::view_geometry,
            model::{AgentsPane, SkillsPane, TuiModel, View},
            render,
            theme::Theme,
        },
    },
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend, layout::Rect};
use uuid::Uuid;

fn profile() -> AgentProfileVersion {
    let template = &builtin_profile_templates()[0];
    let mut draft = template.copy_to_draft().expect("valid builtin template");
    draft.display_name = "Long Horizon Analyst".to_owned();
    draft.description = "Builds patient, evidence-led theses.".to_owned();
    draft.primary_specialty = "fundamental compounders".to_owned();
    draft.specialty_tags = vec!["quality".to_owned(), "long-duration".to_owned()];
    draft.personality = "Patient, skeptical, and explicit about uncertainty.".to_owned();
    draft.instructions = "Separate facts from assumptions and cite primary evidence.".to_owned();
    AgentProfileVersion::create(
        AgentProfileId::from_uuid(Uuid::from_u128(10)),
        AgentProfileVersionId::from_uuid(Uuid::from_u128(11)),
        MemoryNamespaceId::from_uuid(Uuid::from_u128(12)),
        1_800_000_000_000,
        draft,
        Some(template.provenance()),
    )
    .expect("valid profile")
}

fn snapshot(with_profile: bool) -> PresentationSnapshot {
    let (agent_profiles, selected_agent_profile, selected_agent_profile_history) = if with_profile {
        let profile = profile();
        let readiness = AgentReadiness::Unbound;
        (
            AgentProfilesView {
                profiles: vec![AgentProfileSummary {
                    profile_id: profile.profile_id(),
                    profile_version_id: profile.profile_version_id(),
                    version: profile.version(),
                    display_name: profile.display_name().to_owned(),
                    role: profile.role(),
                    primary_specialty: profile.primary_specialty().to_owned(),
                    readiness,
                    content_digest: profile.content_digest().clone(),
                }],
                total_count: 1,
                returned_count: 1,
                truncated: false,
            },
            Some(AgentProfileView {
                profile: profile.clone(),
                readiness,
            }),
            Some(AgentProfileHistoryView {
                profile_id: profile.profile_id(),
                active_version_id: profile.profile_version_id(),
                versions: vec![AgentProfileHistoryEntry {
                    profile_version_id: profile.profile_version_id(),
                    version: profile.version(),
                    supersedes: profile.supersedes(),
                    created_at_ms: profile.created_at_ms(),
                    readiness,
                    content_digest: profile.content_digest().clone(),
                }],
                total_count: 1,
                returned_count: 1,
                truncated: false,
            }),
        )
    } else {
        (
            AgentProfilesView {
                profiles: Vec::new(),
                total_count: 0,
                returned_count: 0,
                truncated: false,
            },
            None,
            None,
        )
    };

    PresentationSnapshot {
        installation_id: InstallationId::from_uuid(Uuid::from_u128(1)),
        session_id: SessionId::from_uuid(Uuid::from_u128(2)),
        database_readiness: DatabaseReadiness::Ready,
        process_guard_ownership: ProcessGuardOwnership::Held,
        setup_status: SetupStatus::NotStarted,
        recent_audit: Vec::new(),
        agent_profiles,
        selected_agent_profile,
        selected_agent_profile_history,
    }
}

fn model(with_profile: bool, pane: AgentsPane) -> TuiModel {
    let mut model = TuiModel::new(snapshot(with_profile), false);
    model.active_view = View::Agents;
    model.agents.pane = pane;
    model
}

fn rendered(model: &TuiModel, width: u16, height: u16) -> Terminal<TestBackend> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| render::render(frame, model, &Theme::from_no_color(false)))
        .expect("agents render remains total");
    terminal
}

fn render_text(model: &TuiModel, width: u16, height: u16) -> String {
    rendered(model, width, height)
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

fn final_key(model: &mut TuiModel, code: KeyCode) -> ControllerEffect {
    handle_event(
        model,
        TuiEvent::Key(KeyEvent::new(code, KeyModifiers::NONE)),
    )
}

fn final_backtab(model: &mut TuiModel) {
    handle_event(
        model,
        TuiEvent::Key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)),
    );
}

fn final_footer(model: &TuiModel, width: u16, height: u16) -> String {
    let terminal = rendered(model, width, height);
    let buffer = terminal.backend().buffer();
    (height - 2..height)
        .flat_map(|y| (0..width).map(move |x| buffer[(x, y)].symbol()))
        .collect()
}

#[test]
fn final_profile_scroll_is_bounded_and_new_selection_keeps_pinned_loading_context() {
    use ai_stock_forum::ui::tui::model::Focus;
    for (width, height) in [(60, 18), (120, 30)] {
        let mut model = model(true, AgentsPane::Detail);
        let first = profile();
        let mut draft = first.to_draft();
        draft.instructions = format!("{} PROFILE-END", "long evidence ".repeat(220));
        let long = AgentProfileVersion::create(
            first.profile_id(),
            first.profile_version_id(),
            first.memory_namespace_id(),
            first.created_at_ms(),
            draft,
            None,
        )
        .unwrap();
        model.agents.profiles.profiles[0].content_digest = long.content_digest().clone();
        model.agents.detail.as_mut().unwrap().profile = long;
        let mut next = model.agents.profiles.profiles[0].clone();
        next.profile_id = AgentProfileId::from_uuid(Uuid::from_u128(99));
        next.display_name = "Next Agent".into();
        model.agents.profiles.profiles.push(next);
        handle_event(&mut model, TuiEvent::Resize(width, height));
        final_key(&mut model, KeyCode::End);
        assert!(
            model.agents.detail_scroll > 0,
            "End must move the actual profile offset"
        );
        assert!(render_text(&model, width, height).contains("PROFILE-END"));
        let end = model.agents.detail_scroll;
        for _ in 0..50 {
            final_key(&mut model, KeyCode::Char('s'));
        }
        assert_eq!(model.agents.detail_scroll, end);
        final_key(&mut model, KeyCode::Char('9'));
        final_key(&mut model, KeyCode::Char('3'));
        assert_eq!(model.agents.detail_scroll, end);
        let text = render_text(&model, width, height);
        for label in [
            "Long Horizon Analyst",
            "Profile",
            "Memory",
            "Skills",
            "History",
        ] {
            assert!(text.contains(label), "missing {label}");
        }
        final_key(&mut model, KeyCode::Home);
        assert_eq!(model.agents.detail_scroll, 0);
        final_key(&mut model, KeyCode::PageDown);
        assert!(model.agents.detail_scroll > 0);
        let retained = model.agents.detail_scroll;
        final_backtab(&mut model);
        final_key(&mut model, KeyCode::Tab);
        assert_eq!(
            model.agents.detail_scroll, retained,
            "same-agent compact list round-trip keeps its body position"
        );
        final_backtab(&mut model);
        assert_eq!(model.focus, Focus::List);
        final_key(&mut model, KeyCode::Char('s'));
        final_key(&mut model, KeyCode::Tab);
        assert_eq!(model.agents.detail_scroll, 0);
        let text = render_text(&model, width, height);
        assert!(text.contains("Loading Next Agent"));
        for label in ["Profile", "Memory", "Skills", "History"] {
            assert!(text.contains(label));
        }
    }
}

#[test]
fn final_history_keeps_twelve_wrapped_cards_visible_through_resize_and_opens_at_top() {
    let mut model = model(true, AgentsPane::Detail);
    handle_event(&mut model, TuiEvent::Resize(60, 18));
    final_key(&mut model, KeyCode::Char('h'));
    let first = profile();
    let mut versions = vec![first.clone()];
    for index in 2..=12 {
        let previous = versions.last().unwrap();
        let mut draft = previous.to_draft();
        draft.description = format!("History description {index}");
        versions.push(
            AgentProfileVersion::next_version(
                previous,
                AgentProfileVersionId::from_uuid(Uuid::from_u128(100 + index)),
                previous.created_at_ms() + 1,
                draft,
            )
            .unwrap(),
        );
    }
    let history = model.agents.history.as_mut().unwrap();
    history.versions = versions
        .iter()
        .map(|profile| AgentProfileHistoryEntry {
            profile_version_id: profile.profile_version_id(),
            version: profile.version(),
            supersedes: profile.supersedes(),
            created_at_ms: profile.created_at_ms(),
            readiness: AgentReadiness::Unbound,
            content_digest: profile.content_digest().clone(),
        })
        .collect();
    for index in 0..12 {
        if index > 0 {
            final_key(&mut model, KeyCode::Char('s'));
        }
        let text = render_text(&model, 60, 18);
        assert!(
            text.contains(&format!("› Version {}", index + 1)),
            "selected version {} hidden",
            index + 1
        );
    }
    for (width, height) in [(120, 30), (60, 18)] {
        handle_event(&mut model, TuiEvent::Resize(width, height));
        assert!(render_text(&model, width, height).contains("› Version 12"));
    }
    let effect = final_key(&mut model, KeyCode::Enter);
    assert!(
        matches!(effect, ControllerEffect::LoadSelectedAgentProfile { read: ai_stock_forum::ui::tui::model::AgentProfileRead::Version(version), .. } if version.get() == 12)
    );
    let target = model.agents.profile_target().unwrap();
    assert!(model.agents.install_profile_result(
        &target,
        ai_stock_forum::app::CommandView::AgentProfileVersion(AgentProfileVersionView {
            profile: versions.last().unwrap().clone(),
            readiness: AgentReadiness::Unbound,
            predecessor_diff: vec![]
        })
    ));
    assert!(render_text(&model, 60, 18).contains("Version 12"));
    final_key(&mut model, KeyCode::Esc);
    assert!(render_text(&model, 60, 18).contains("› Version 12"));
}

#[test]
fn final_editor_boundary_tabs_leave_and_return_to_retained_invalid_fields() {
    use ai_stock_forum::ui::{
        profile_editor::ProfileTuiField,
        tui::model::{Focus, InputMode},
    };
    for (width, height) in [(60, 18), (120, 30)] {
        let mut model = model(true, AgentsPane::Editor);
        model.agents.editor =
            Some(ProfileEditor::for_create(&builtin_profile_templates()[0]).unwrap());
        model.set_terminal_size(width, height);
        final_backtab(&mut model);
        assert_eq!(model.focus, Focus::List);
        assert!(!final_footer(&model, width, height).contains("next field"));
        final_key(&mut model, KeyCode::Char('w'));
        assert_eq!(
            model.agents.editor.as_ref().unwrap().tui_field(),
            ProfileTuiField::Template
        );
        handle_event(&mut model, TuiEvent::Paste("not editor input".into()));
        assert_eq!(
            model.agents.editor.as_ref().unwrap().tui_field(),
            ProfileTuiField::Template
        );
        final_key(&mut model, KeyCode::Tab);
        assert_eq!(model.focus, Focus::Workspace);
        assert_eq!(
            model.agents.editor.as_ref().unwrap().tui_field(),
            ProfileTuiField::Template
        );
        final_key(&mut model, KeyCode::Tab);
        final_key(&mut model, KeyCode::Enter);
        model.agents.field_input.clear();
        let raw = "invalid".repeat(50);
        handle_event(&mut model, TuiEvent::Paste(raw.clone()));
        final_key(&mut model, KeyCode::Tab);
        for _ in 0..20 {
            if model.focus != Focus::Workspace {
                break;
            }
            final_key(&mut model, KeyCode::Tab);
        }
        assert_eq!(model.focus, Focus::Navigation);
        assert_eq!(model.input_mode, InputMode::Nav);
        let editor = model.agents.editor.as_ref().unwrap();
        assert_eq!(editor.tui_field(), ProfileTuiField::Discard);
        assert_eq!(editor.tui_field_text(ProfileTuiField::DisplayName), raw);
        assert!(
            editor
                .tui_field_error(ProfileTuiField::DisplayName)
                .is_some()
        );
        final_backtab(&mut model);
        assert_eq!(model.focus, Focus::Workspace);
        assert_eq!(
            model.agents.editor.as_ref().unwrap().tui_field(),
            ProfileTuiField::Discard
        );
        final_key(&mut model, KeyCode::Esc);
        assert!(final_footer(&model, width, height).contains("Resume"));
    }
}

#[test]
fn final_footer_advertises_only_available_actions_for_the_logical_owner() {
    use ai_stock_forum::ui::tui::model::Focus;
    for (pane, focus, loaded, skills, draft, expected) in [
        (
            AgentsPane::List,
            Focus::List,
            true,
            false,
            false,
            vec!["N new", "E edit", "H history"],
        ),
        (
            AgentsPane::Detail,
            Focus::Workspace,
            true,
            false,
            false,
            vec!["N new", "E edit", "H history"],
        ),
        (
            AgentsPane::History,
            Focus::Workspace,
            true,
            false,
            false,
            vec!["E edit"],
        ),
        (
            AgentsPane::Detail,
            Focus::Navigation,
            true,
            false,
            false,
            vec![],
        ),
        (
            AgentsPane::Detail,
            Focus::Workspace,
            true,
            true,
            false,
            vec![],
        ),
        (
            AgentsPane::Detail,
            Focus::Workspace,
            false,
            false,
            false,
            vec!["N new"],
        ),
        (
            AgentsPane::Detail,
            Focus::Workspace,
            true,
            false,
            true,
            vec!["N Resume", "E Resume", "H history"],
        ),
        (
            AgentsPane::Detail,
            Focus::Workspace,
            false,
            false,
            true,
            vec!["N Resume", "E Resume"],
        ),
        (
            AgentsPane::Detail,
            Focus::List,
            true,
            true,
            false,
            vec!["N new", "E edit", "H history"],
        ),
        (
            AgentsPane::Editor,
            Focus::Navigation,
            true,
            false,
            true,
            vec![],
        ),
    ] {
        let mut model = model(true, pane);
        model.set_focus(focus);
        model.agents.skill_panel_open = skills;
        if !loaded {
            model.agents.detail = None;
        }
        if draft {
            model.agents.editor =
                Some(ProfileEditor::for_create(&builtin_profile_templates()[0]).unwrap());
        }
        let footer = final_footer(&model, 60, 18);
        for (ch, needle) in [('n', "N "), ('e', "E "), ('h', "H ")] {
            let advertised = expected.iter().any(|label| label.starts_with(needle));
            assert_eq!(
                footer.contains(needle),
                advertised,
                "{pane:?} {focus:?}: {footer}"
            );
            if advertised {
                let mut copy = model.clone();
                let effect = final_key(&mut copy, KeyCode::Char(ch));
                assert!(effect != ControllerEffect::None);
                if ch == 'n' && !draft {
                    assert!(matches!(
                        effect,
                        ControllerEffect::StartProfileCreate { .. }
                    ));
                }
                if ch == 'e' && !draft {
                    assert!(matches!(
                        effect,
                        ControllerEffect::StartSelectedProfileEdit { .. }
                    ));
                }
                if draft && ch != 'h' {
                    assert_eq!(copy.agents.pane, AgentsPane::Editor);
                }
            }
        }
        for label in expected {
            assert!(footer.contains(label), "missing {label}: {footer}");
        }
    }
}

#[test]
fn final_shifted_actions_are_nav_shortcuts_but_modifiers_and_type_stay_literal() {
    for letter in ['N', 'E', 'H'] {
        for modifiers in [KeyModifiers::NONE, KeyModifiers::SHIFT] {
            let mut model = model(true, AgentsPane::Detail);
            let effect = handle_event(
                &mut model,
                TuiEvent::Key(KeyEvent::new(KeyCode::Char(letter), modifiers)),
            );
            assert!(
                matches!(
                    effect,
                    ControllerEffect::StartProfileCreate { .. }
                        | ControllerEffect::StartSelectedProfileEdit { .. }
                        | ControllerEffect::LoadSelectedAgentProfile { .. }
                ),
                "{letter} {modifiers:?}: {effect:?}"
            );
        }
        for modifiers in [
            KeyModifiers::ALT,
            KeyModifiers::CONTROL,
            KeyModifiers::SUPER,
        ] {
            let mut model = model(true, AgentsPane::Detail);
            let before = model.clone();
            handle_event(
                &mut model,
                TuiEvent::Key(KeyEvent::new(KeyCode::Char(letter), modifiers)),
            );
            assert_eq!(model, before);
        }
    }
    let mut model = model(true, AgentsPane::Editor);
    model.agents.editor = Some(ProfileEditor::for_create(&builtin_profile_templates()[0]).unwrap());
    final_key(&mut model, KeyCode::Tab);
    final_key(&mut model, KeyCode::Enter);
    model.agents.field_input.clear();
    for letter in ['N', 'E', 'H'] {
        handle_event(
            &mut model,
            TuiEvent::Key(KeyEvent::new(KeyCode::Char(letter), KeyModifiers::SHIFT)),
        );
    }
    assert_eq!(model.agents.field_input.text(), "NEH");
}

#[test]
fn final_type_cursor_stays_visible_after_safe_wide_and_combining_text() {
    for raw in ["界".repeat(60), "e\u{301}界".repeat(40)] {
        let mut model = model(false, AgentsPane::Editor);
        model.agents.editor =
            Some(ProfileEditor::for_create(&builtin_profile_templates()[0]).unwrap());
        model.set_terminal_size(60, 18);
        final_key(&mut model, KeyCode::Tab);
        final_key(&mut model, KeyCode::Enter);
        model.agents.field_input.clear();
        handle_event(&mut model, TuiEvent::Paste(raw.clone()));
        let terminal = rendered(&model, 60, 18);
        let buffer = terminal.backend().buffer();
        // Exclude the panel borders; the only inner vertical marker is the TYPE cursor.
        assert!(
            (1..59).any(|x| (0..18).any(|y| buffer[(x, y)].symbol() == "│")),
            "cursor clipped for {raw}"
        );
        assert_eq!(model.agents.field_input.text(), raw);
    }
}

#[test]
fn final_help_explains_current_profile_controls() {
    let mut model = model(false, AgentsPane::List);
    model.select_view(View::Help);
    let text = render_text(&model, 180, 70);
    for label in [
        "Profile / Memory / Skills / History",
        "Esc keeps",
        "Resume",
        "literal",
        "Memory and Skills",
    ] {
        assert!(text.contains(label), "missing {label}");
    }
    assert!(!text.contains("Profile, Memory, and Skills keep"));
}

#[test]
fn final_guide_uses_a_portable_repository_root_launch() {
    let guide = include_str!("../docs/testing/two-pane-shell-agents.md");
    assert!(guide.contains("repository root"));
    assert!(!guide.contains("/Users/nguyen-mini"));
}

#[test]
fn fresh_agents_list_owns_the_only_focused_panel_and_actions_wait_for_tab() {
    let theme = Theme::from_no_color(false);
    for (width, height) in [(60, 18), (120, 30)] {
        let mut model = TuiModel::new(snapshot(true), false);
        handle_event(&mut model, TuiEvent::Resize(width, height));
        handle_event(
            &mut model,
            TuiEvent::Key(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE)),
        );
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render::render(frame, &model, &theme))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let style_for = |needle: &str| {
            (0..height).find_map(|y| {
                let row = (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>();
                row.find(needle).map(|byte| {
                    let x = row[..byte].chars().count() as u16;
                    buffer[(x, y)].style()
                })
            })
        };
        assert_eq!(style_for("Agent list").unwrap().fg, theme.focus.fg);
        if width == 120 {
            assert_ne!(style_for("Agent workspace").unwrap().fg, theme.focus.fg);
            assert_ne!(style_for("Profile ").unwrap().fg, theme.focus.fg);
        }
    }
}

fn render_rows(model: &TuiModel, width: u16, height: u16) -> Vec<String> {
    let terminal = rendered(model, width, height);
    terminal
        .backend()
        .buffer()
        .content()
        .chunks(usize::from(width))
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
        .collect()
}

#[test]
fn friendly_profile_and_history_hide_internal_identity_metadata() {
    for pane in [AgentsPane::Detail, AgentsPane::History] {
        let model = model(true, pane);
        let screen = render_text(&model, 160, 80);
        for secret in [
            profile().profile_id().to_string(),
            profile().profile_version_id().to_string(),
            profile().memory_namespace_id().to_string(),
            profile().content_digest().to_string(),
            profile()
                .template_provenance()
                .unwrap()
                .template_digest
                .to_string(),
        ] {
            assert!(
                !screen.contains(&secret),
                "internal identity leaked in {pane:?}"
            );
        }
        assert!(screen.contains("Long Horizon Analyst"));
        assert!(screen.contains("Version 1"));
        assert!(screen.contains("Current"));
    }
}

#[test]
fn selected_identity_and_four_choices_render_while_detail_is_loading() {
    let mut model = model(true, AgentsPane::Detail);
    model.agents.profiles.profiles[0].display_name = "Selected New Agent".to_owned();
    model.agents.profiles.profiles[0].profile_id = AgentProfileId::from_uuid(Uuid::from_u128(90));
    let screen = render_text(&model, 120, 30);
    assert!(screen.contains("Loading Selected New Agent"));
    assert!(!screen.contains("Long Horizon Analyst"));
    for label in ["Profile", "Memory", "Skills", "History"] {
        assert!(screen.contains(label));
    }
}

#[test]
fn minimum_agents_and_editor_hints_remain_complete_and_review_scrolls_to_last_diff() {
    use ai_stock_forum::ui::profile_editor::ProfileTuiField;
    let mut model = model(true, AgentsPane::Detail);
    let text = render_text(&model, 60, 18);
    for hint in ["Enter", "Esc", "N new", "E edit", "H history"] {
        assert!(text.contains(hint), "missing {hint}");
    }
    let mut editor = ProfileEditor::for_create(&builtin_profile_templates()[0]).unwrap();
    editor.set_tui_field(
        ProfileTuiField::Description,
        &"Long description. ".repeat(30),
    );
    editor.set_tui_field(
        ProfileTuiField::Instructions,
        &format!("{} FINAL REVIEW LINE", "Long instructions. ".repeat(120)),
    );
    while editor.tui_field() != ProfileTuiField::Review {
        editor.move_tui_field(true);
    }
    model.agents.editor = Some(editor);
    model.agents.pane = AgentsPane::Editor;
    model.set_terminal_size(60, 18);
    let text = render_text(&model, 60, 18);
    assert!(!text.contains("FINAL REVIEW LINE"));
    handle_event(
        &mut model,
        TuiEvent::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE)),
    );
    let text = render_text(&model, 60, 18);
    assert!(text.contains("FINAL REVIEW LINE"));
    assert!(text.contains("Esc keep"));
    assert!(text.contains("Tab"));
    assert_eq!(
        model.agents.editor.as_ref().unwrap().tui_field(),
        ProfileTuiField::Review
    );
}

#[test]
fn identity_monogram_color_survives_reordering_and_no_color_has_no_palette() {
    let mut model = model(true, AgentsPane::List);
    model.agents.profiles.profiles[0].display_name = "Alpha One".to_owned();
    let mut second = model.agents.profiles.profiles[0].clone();
    second.profile_id = AgentProfileId::from_uuid(Uuid::from_u128(91));
    second.display_name = "Beta Two".to_owned();
    model.agents.profiles.profiles.push(second);
    let color = |model: &TuiModel, initials: &str| {
        let terminal = rendered(model, 120, 30);
        let buffer = terminal.backend().buffer();
        for y in 4..24 {
            for x in 1..35 {
                if format!("{}{}", buffer[(x, y)].symbol(), buffer[(x + 1, y)].symbol()) == initials
                {
                    return buffer[(x, y)].fg;
                }
            }
        }
        panic!("initials not visible");
    };
    let first_color = color(&model, "AO");
    let second_color = color(&model, "BT");
    assert_ne!(first_color, second_color);
    model.agents.profiles.profiles.reverse();
    assert_eq!(color(&model, "AO"), first_color);
    assert_eq!(color(&model, "BT"), second_color);
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal
        .draw(|frame| render::render(frame, &model, &Theme::from_no_color(true)))
        .unwrap();
    assert!(
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .all(|cell| cell.fg == ratatui::style::Color::Reset
                && cell.bg == ratatui::style::Color::Reset)
    );
}

#[test]
fn long_invalid_profile_field_error_stays_visible_in_type_and_nav_at_minimum_size() {
    use ai_stock_forum::ui::profile_editor::ProfileTuiField;
    let mut model = model(false, AgentsPane::Editor);
    model.agents.editor = Some(ProfileEditor::for_create(&builtin_profile_templates()[0]).unwrap());
    model.set_terminal_size(60, 18);
    handle_event(
        &mut model,
        TuiEvent::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
    );
    handle_event(
        &mut model,
        TuiEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    );
    model.agents.field_input.clear();
    let raw = "x".repeat(300);
    handle_event(&mut model, TuiEvent::Paste(raw.clone()));
    assert!(render_text(&model, 60, 18).contains("Display name: invalid"));
    handle_event(
        &mut model,
        TuiEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
    );
    assert!(render_text(&model, 60, 18).contains("Display name: invalid"));
    assert_eq!(
        model
            .agents
            .editor
            .as_ref()
            .unwrap()
            .tui_field_text(ProfileTuiField::DisplayName),
        raw
    );
    assert!(
        model
            .agents
            .editor
            .as_ref()
            .unwrap()
            .tui_field_error(ProfileTuiField::DisplayName)
            .is_some()
    );
    while model.agents.editor.as_ref().unwrap().tui_field() != ProfileTuiField::Review {
        handle_event(
            &mut model,
            TuiEvent::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
        );
    }
    handle_event(
        &mut model,
        TuiEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    );
    assert!(model.agents.pending_confirmation.is_none());
}

#[test]
fn hidden_profile_editor_does_not_claim_the_skills_command_bar() {
    let mut model = model(false, AgentsPane::List);
    assert!(
        model
            .agents
            .start_profile_create(0, builtin_profile_templates())
    );
    model.skills.active = true;
    model.skills.pane = SkillsPane::List;

    let text = render_text(&model, 100, 30);

    assert!(!text.contains(" Command "));
    assert!(!text.contains("Profile input"));
}

#[test]
fn profile_editor_footer_matches_nav_and_literal_type_controls() {
    let mut model = model(false, AgentsPane::Editor);
    model.agents.editor = Some(ProfileEditor::for_create(&builtin_profile_templates()[0]).unwrap());
    let text = render_text(&model, 100, 30);
    assert!(text.contains("NAV"));
    assert!(text.contains("Tab next field"));
    handle_event(
        &mut model,
        TuiEvent::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
    );
    handle_event(
        &mut model,
        TuiEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    );
    let text = render_text(&model, 100, 30);
    for hint in ["TYPE", "WASD text", "Enter accept", "Esc keep"] {
        assert!(text.contains(hint), "missing {hint}");
    }
    assert!(!text.contains("Profile input"));
}

#[test]
fn agents_layout_uses_one_or_two_panes_at_exact_width_breakpoints() {
    let list = model(true, AgentsPane::List);
    let narrow_list = render_text(&list, 79, 24);
    assert!(narrow_list.contains("Agent list"));
    assert!(!narrow_list.contains("Agent workspace"));

    let detail = model(true, AgentsPane::Detail);
    let narrow_detail = render_text(&detail, 79, 24);
    assert!(!narrow_detail.contains("Agent list"));
    assert!(narrow_detail.contains("Agent workspace"));

    let compact = render_text(&detail, 99, 24);
    assert!(!compact.contains("Agent list"));
    assert!(compact.contains("Agent workspace"));

    let medium = render_text(&detail, 100, 24);
    assert!(medium.contains("Agent list"));
    assert!(medium.contains("Agent workspace"));
    assert!(!medium.contains("Readiness & history"));

    let wide = render_text(&detail, 120, 30);
    assert!(wide.contains("Agent list"));
    assert!(wide.contains("Agent workspace"));
    assert!(!wide.contains("Readiness & history"));

    let medium_low = render_text(&detail, 100, 18);
    assert!(medium_low.contains("Agent list"));
    assert!(medium_low.contains("Agent workspace"));
    assert!(!medium_low.contains("Readiness & history"));

    let wide_low = render_text(&detail, 120, 18);
    assert!(wide_low.contains("Agent list"));
    assert!(wide_low.contains("Agent workspace"));
    assert!(!wide_low.contains("Readiness & history"));
}

#[test]
fn legacy_views_keep_height_aware_modes_at_low_supported_heights() {
    for view in [View::Overview, View::Setup, View::Audit, View::Help] {
        for (width, mode) in [(80, "Narrow"), (120, "Wide")] {
            let mut legacy = model(false, AgentsPane::List);
            legacy.active_view = view;
            let text = render_text(&legacy, width, 18);
            assert!(text.contains(mode), "view={view:?} width={width}");
            assert!(
                !text.contains(" Navigation "),
                "view={view:?} width={width}"
            );
        }
    }
}

#[test]
fn narrow_header_rows_are_complete_at_sixty_and_seventy_columns() {
    let model = model(true, AgentsPane::List);
    for width in [60, 70] {
        let rows = render_rows(&model, width, 18);
        assert_eq!(rows[0].trim_end(), "AI STOCK FORUM  /  Agents  /  Narrow");
        let navigation = format!("{} {}", rows[1], rows[2]);
        for label in [
            "1 Home",
            "2 Chat",
            "3 Agents",
            "4 Skills",
            "5 Connections",
            "6 Activity",
            "7 Setup",
            "8 Audit",
            "9 Help",
        ] {
            assert!(navigation.contains(label), "width={width} label={label}");
        }
    }
}

#[test]
fn view_geometry_accounts_for_the_agents_header_height() {
    for width in [80, 120] {
        let area = Rect::new(0, 0, width, 18);
        let legacy = view_geometry(area, View::Overview, false);
        let agents = view_geometry(area, View::Agents, false);

        assert_eq!(legacy.cockpit.header.height, 3);
        assert_eq!(agents.cockpit.header.height, 3);
        assert_eq!(legacy.workspace_body_height, 10);
        assert_eq!(agents.workspace_body_height, 10);
    }
}

#[test]
fn agents_empty_and_populated_states_render_readable_identity_and_readiness() {
    let empty = render_text(&model(false, AgentsPane::List), 100, 30);
    assert!(empty.contains("No agent profiles yet"));
    assert!(empty.contains("Press N"));
    let populated = render_text(&model(true, AgentsPane::Detail), 160, 44);
    for expected in [
        "Long Horizon Analyst",
        "bull",
        "fundamental compounders",
        "long-duration, quality",
        "Version 1",
        "Current",
        "Bindings",
        "Needs connection",
        "Profile",
        "Memory",
        "Skills",
        "History",
    ] {
        assert!(populated.contains(expected), "missing {expected}");
    }
    for hidden in [
        "IMMUTABLE METADATA",
        "Created ms",
        "Profile ID",
        "Digest",
        "builtin.bull",
    ] {
        assert!(!populated.contains(hidden));
    }
}

#[test]
fn list_scroll_is_an_item_offset_and_keeps_the_last_multiline_row_visible() {
    let mut model = model(true, AgentsPane::List);
    let base = model.agents.profiles.profiles[0].clone();
    model.agents.profiles.profiles = (0..12)
        .map(|index| {
            let mut profile = base.clone();
            profile.display_name = format!("Profile {index:02}");
            profile
        })
        .collect();
    model.agents.selected_profile = 11;
    model.agents.list_scroll = 11;

    let text = render_text(&model, 60, 18);
    assert!(text.contains("Profile 11"));
    assert!(!text.contains("Profile 00"));
}

#[test]
fn moving_selection_keeps_all_fitting_agent_cards_stationary() {
    let mut model = model(true, AgentsPane::List);
    let mut bear = model.agents.profiles.profiles[0].clone();
    bear.display_name = "Bear Researcher".to_owned();
    let mut lnext = bear.clone();
    lnext.display_name = "Lnext".to_owned();
    model.agents.profiles = AgentProfilesView {
        profiles: vec![bear, lnext],
        total_count: 2,
        returned_count: 2,
        truncated: false,
    };
    model.set_terminal_size(120, 30);

    assert_eq!(
        handle_event(
            &mut model,
            TuiEvent::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
        ),
        ControllerEffect::LoadSelectedAgentProfile {
            target: model.agents.profile_target().unwrap(),
            read: ai_stock_forum::ui::tui::model::AgentProfileRead::Detail
        }
    );

    let text = render_text(&model, 120, 30);
    assert!(text.contains("  Bear Researcher"));
    assert!(text.contains("Lnext"));
}

#[test]
fn editor_renders_progress_guidance_ordered_review_diffs_and_explicit_confirmation() {
    let profile = profile();
    let draft = AgentProfileDraft::new(
        "Revised Horizon Analyst".to_owned(),
        profile.description().to_owned(),
        AgentRole::Chief,
        profile.primary_specialty().to_owned(),
        profile.specialty_tags().to_vec(),
        profile.personality().to_owned(),
        profile.instructions().to_owned(),
        AgentBindings::default(),
        Vec::new(),
        Vec::new(),
    )
    .expect("valid edit draft");
    let mut editor = ProfileEditor::for_edit(
        profile.profile_id(),
        profile.profile_version_id(),
        draft.clone(),
    );
    for _ in 0..7 {
        assert_eq!(editor.submit_line(":next"), ProfileEditorEffect::None);
    }
    let ProfileEditorEffect::PreviewEdit(request) = editor.submit_line(":review") else {
        panic!("review request")
    };
    editor.apply_preview(
        request.generation,
        ProfileEditPreview {
            profile_id: request.profile_id,
            expected_active_version_id: request.expected_active_version_id,
            diffs: vec![
                ProfileFieldDiff {
                    field: ProfileDiffField::DisplayName,
                    before: ProfileFieldValue::Text("Long Horizon Analyst".to_owned()),
                    after: ProfileFieldValue::Text("Revised Horizon Analyst".to_owned()),
                },
                ProfileFieldDiff {
                    field: ProfileDiffField::Role,
                    before: ProfileFieldValue::Role(AgentRole::Bull),
                    after: ProfileFieldValue::Role(AgentRole::Chief),
                },
            ],
            review_token: ProfileReviewToken::from_uuid(Uuid::from_u128(20)),
            review_digest: sha256(b"review"),
        },
    );

    while editor.tui_field() != ai_stock_forum::ui::profile_editor::ProfileTuiField::Review {
        editor.move_tui_field(true);
    }
    let mut editor_model = model(true, AgentsPane::Editor);
    editor_model.agents.editor = Some(editor.clone());
    let review = render_text(&editor_model, 100, 30);
    assert!(!review.contains("Step 7 of 7"));
    assert!(review.contains("Review"));
    assert!(review.contains("Display name"));
    assert!(review.contains("Before"));
    assert!(review.contains("After"));
    assert!(review.contains("separate confirmation"));
    assert!(
        review.find("Display name").expect("display diff")
            < review.find("Role").expect("role diff")
    );

    let ProfileEditorEffect::Execute(command) = editor.submit_line(":activate") else {
        panic!("activation command")
    };
    let mut confirmation = model(true, AgentsPane::Confirmation);
    confirmation.agents.editor = Some(editor);
    confirmation.agents.pending_confirmation = Some(ProfileConfirmation { command });
    let confirmation = render_text(&confirmation, 100, 30);
    assert!(confirmation.contains("Confirm Activate"));
    assert!(confirmation.contains("Enter: activate"));
    assert!(!confirmation.contains("Reviewed base"));
    assert!(!confirmation.contains("Review digest"));
    assert!(confirmation.contains("Esc"));

    let mut create_editor =
        ProfileEditor::for_create(&builtin_profile_templates()[0]).expect("valid create editor");
    let mut create = model(false, AgentsPane::Editor);
    create.agents.editor = Some(create_editor.clone());
    let create_text = render_text(&create, 79, 24);
    assert!(create_text.contains("New agent"));
    assert!(create_text.contains("WASD choose"));
    assert!(create_text.contains("Enter edit/select"));
    assert!(create_text.contains("Selected field"));
    assert!(create_text.contains("Template"));

    let mut edit_template = model(true, AgentsPane::Editor);
    edit_template.agents.editor = Some(ProfileEditor::for_edit(
        profile.profile_id(),
        profile.profile_version_id(),
        draft,
    ));
    let edit_text = render_text(&edit_template, 100, 30);
    assert!(!edit_text.contains("Up/Down: choose template"));
    assert!(!edit_text.contains("Choose the complete starting profile"));
    assert!(edit_text.contains("Enter edit/select"));
    assert!(!edit_text.contains("Advanced: :role <role>"));

    for _ in 0..7 {
        assert_eq!(
            create_editor.submit_line(":next"),
            ProfileEditorEffect::None
        );
    }
    while create_editor.tui_field() != ai_stock_forum::ui::profile_editor::ProfileTuiField::Review {
        create_editor.move_tui_field(true);
    }
    create.agents.editor = Some(create_editor);
    let create_review = render_text(&create, 100, 40);
    assert!(create_review.contains("separate confirmation"));
}

#[test]
fn guided_editor_renders_domain_limits_and_readonly_bindings() {
    use ai_stock_forum::ui::profile_editor::ProfileTuiField;
    let cases = [
        (ProfileTuiField::DisplayName, DISPLAY_NAME_MAX_BYTES),
        (ProfileTuiField::Description, DESCRIPTION_MAX_BYTES),
        (
            ProfileTuiField::PrimarySpecialty,
            PRIMARY_SPECIALTY_MAX_BYTES,
        ),
        (ProfileTuiField::Personality, PERSONALITY_MAX_BYTES),
        (ProfileTuiField::Instructions, INSTRUCTIONS_MAX_BYTES),
    ];
    for (field, limit) in cases {
        let mut editor = ProfileEditor::for_create(&builtin_profile_templates()[0]).unwrap();
        while editor.tui_field() != field {
            editor.move_tui_field(true);
        }
        for (width, height) in [(70, 24), (100, 30), (140, 40)] {
            let mut model = model(false, AgentsPane::Editor);
            model.agents.editor = Some(editor.clone());
            let text = render_text(&model, width, height);
            assert!(
                text.contains(&format!("{limit} UTF-8 bytes")),
                "{field:?} at {width}"
            );
        }
    }
    let mut editor = ProfileEditor::for_create(&builtin_profile_templates()[0]).unwrap();
    while editor.tui_field() != ProfileTuiField::Bindings {
        editor.move_tui_field(true);
    }
    let mut model = model(false, AgentsPane::Editor);
    model.agents.editor = Some(editor);
    let text = render_text(&model, 100, 30);
    assert!(text.contains("Read-only"));
    assert!(!text.contains("binding-reference IDs"));
}

#[test]
fn long_safe_content_wraps_and_scrolls_while_escape_controls_never_reach_the_buffer() {
    let mut model = model(true, AgentsPane::Detail);
    model.agents.profiles.profiles[0].display_name =
        format!("\u{1b}[31m{}", "safe-long-name ".repeat(20));
    model.agents.detail_scroll = usize::MAX;

    for (width, height) in [(60, 18), (79, 24), (80, 24), (120, 30), (160, 44)] {
        let terminal = rendered(&model, width, height);
        assert!(
            terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .all(|cell| !cell.symbol().contains('\u{1b}'))
        );
    }

    let tiny = render_text(&model, 59, 18);
    assert!(tiny.contains("Terminal too small"));
    assert!(tiny.contains("Minimum: 60 x 18"));
}

#[test]
fn confirmation_distinguishes_create_from_activate() {
    let mut create = model(false, AgentsPane::Confirmation);
    create.agents.pending_confirmation = Some(ProfileConfirmation {
        command: ApplicationCommand::CreateAgentProfile {
            draft: builtin_profile_templates()[0]
                .copy_to_draft()
                .expect("valid create draft"),
            template_provenance: Some(builtin_profile_templates()[0].provenance()),
        },
    });
    let text = render_text(&create, 100, 30);
    assert!(text.contains("Confirm Create"));
    assert!(text.contains("Enter: create"));
    assert!(text.contains("Esc: return to review"));
    assert!(text.contains("Bull Researcher"));
    for chunk in builtin_profile_templates()[0]
        .digest
        .as_str()
        .as_bytes()
        .chunks(16)
    {
        let chunk = std::str::from_utf8(chunk).expect("digest chunks are UTF-8");
        assert!(!text.contains(chunk), "internal digest leaked {chunk}");
    }
}

#[test]
fn selected_historical_version_renders_full_content_metadata_and_predecessor_diff() {
    let first = profile();
    let mut candidate = first.to_draft();
    candidate.description = "Second historical prose.".to_owned();
    let second = AgentProfileVersion::next_version(
        &first,
        AgentProfileVersionId::from_uuid(Uuid::from_u128(13)),
        1_800_000_000_001,
        candidate,
    )
    .unwrap();
    let mut model = model(true, AgentsPane::History);
    model.agents.history = Some(AgentProfileHistoryView {
        profile_id: first.profile_id(),
        active_version_id: second.profile_version_id(),
        versions: vec![
            AgentProfileHistoryEntry {
                profile_version_id: second.profile_version_id(),
                version: second.version(),
                supersedes: second.supersedes(),
                created_at_ms: second.created_at_ms(),
                readiness: AgentReadiness::Unbound,
                content_digest: second.content_digest().clone(),
            },
            AgentProfileHistoryEntry {
                profile_version_id: first.profile_version_id(),
                version: first.version(),
                supersedes: first.supersedes(),
                created_at_ms: first.created_at_ms(),
                readiness: AgentReadiness::Unbound,
                content_digest: first.content_digest().clone(),
            },
        ],
        total_count: 2,
        returned_count: 2,
        truncated: false,
    });
    model.agents.selected_history_version = 0;
    model.agents.version_detail = Some(AgentProfileVersionView {
        profile: second.clone(),
        readiness: AgentReadiness::Unbound,
        predecessor_diff: vec![ProfileFieldDiff {
            field: ProfileDiffField::Description,
            before: ProfileFieldValue::Text(first.description().to_owned()),
            after: ProfileFieldValue::Text(second.description().to_owned()),
        }],
    });

    for width in [79, 120] {
        let rendered = render_text(&model, width, 70);
        for expected in ["Historical", "Read-only", "Second historical prose."] {
            assert!(
                rendered.contains(expected),
                "missing {expected:?} at width {width}"
            );
        }
        if width == 79 {
            for expected in [
                "Changes from previous version",
                "Description",
                "Before",
                "After",
            ] {
                assert!(rendered.contains(expected), "missing {expected:?}");
            }
            for chunk in second.content_digest().as_str().as_bytes().chunks(8) {
                let chunk = std::str::from_utf8(chunk).expect("digest chunks are UTF-8");
                assert!(
                    !rendered.contains(chunk),
                    "internal digest leaked {chunk:?}"
                );
            }
        }
    }
}
