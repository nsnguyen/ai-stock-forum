use super::*;

pub(super) fn retain_field(model: &mut TuiModel) {
    if let Some(editor) = model.skills.editor.as_mut() {
        editor.set_tui_field(editor.field(), model.skills.field_input.text());
    }
}

fn select_field(model: &mut TuiModel, field: SkillEditorField) {
    if let Some(editor) = model.skills.editor.as_mut() {
        editor.select_tui_field(field);
    }
    model.skills.content_scroll = 0;
    model.skills.synchronize_field_input();
}

fn move_grid(current: usize, code: KeyCode, columns: usize, last: usize) -> usize {
    match code {
        KeyCode::Up if current >= columns => current - columns,
        KeyCode::Down if current + columns <= last => current + columns,
        KeyCode::Left if columns == 1 || !current.is_multiple_of(columns) => {
            current.saturating_sub(1)
        }
        KeyCode::Right if columns == 1 || current % columns + 1 < columns => {
            (current + 1).min(last)
        }
        _ => current,
    }
}

pub(super) fn scroll(model: &mut TuiModel, code: KeyCode) -> ControllerEffect {
    let limit = views::skill_content_scroll_limit(model);
    let value = model.skills.content_scroll.min(limit);
    let page = model.workspace_body_height.saturating_sub(6).max(1);
    model.skills.content_scroll = match code {
        KeyCode::Up => value.saturating_sub(1),
        KeyCode::Down => value.saturating_add(1).min(limit),
        KeyCode::PageUp => value.saturating_sub(page),
        KeyCode::PageDown => value.saturating_add(page).min(limit),
        KeyCode::Home => 0,
        KeyCode::End => limit,
        _ => value,
    };
    ControllerEffect::Redraw
}

pub(super) fn handle_editor_key(model: &mut TuiModel, key: KeyEvent) -> ControllerEffect {
    if model.input_mode == InputMode::Type {
        match key.code {
            KeyCode::Esc | KeyCode::Enter if no_modifiers(key.modifiers) => {
                retain_field(model);
                model.set_input_mode(InputMode::Nav);
            }
            KeyCode::Tab if no_modifiers(key.modifiers) => {
                retain_field(model);
                return cycle_focus(model, true);
            }
            KeyCode::BackTab if backtab_modifiers(key.modifiers) => {
                retain_field(model);
                return cycle_focus(model, false);
            }
            KeyCode::Char(character) if text_modifiers(key.modifiers) => {
                model.skills.field_input.insert(character);
                retain_field(model);
            }
            KeyCode::Backspace if no_modifiers(key.modifiers) => {
                model.skills.field_input.backspace();
                retain_field(model);
            }
            KeyCode::Delete if no_modifiers(key.modifiers) => {
                model.skills.field_input.delete();
                retain_field(model);
            }
            KeyCode::Left if no_modifiers(key.modifiers) => model.skills.field_input.move_left(),
            KeyCode::Right if no_modifiers(key.modifiers) => model.skills.field_input.move_right(),
            KeyCode::Home if no_modifiers(key.modifiers) => model.skills.field_input.move_home(),
            KeyCode::End if no_modifiers(key.modifiers) => model.skills.field_input.move_end(),
            _ => return ControllerEffect::None,
        }
        return ControllerEffect::Redraw;
    }
    let key = normalize_nav_direction(key);
    if key.code == KeyCode::BackTab && backtab_modifiers(key.modifiers) {
        return cycle_focus(model, false);
    }
    if !no_modifiers(key.modifiers) {
        return ControllerEffect::None;
    }
    if key.code == KeyCode::Tab {
        return cycle_focus(model, true);
    }
    let page = model.skills.editor_page;
    if page == SkillEditorPage::Review && key.code == KeyCode::Char('i') {
        model.skills.technical_details = !model.skills.technical_details;
        model.skills.content_scroll = 0;
        return ControllerEffect::Redraw;
    }
    if key.code == KeyCode::Esc {
        model.skills.content_scroll = 0;
        match page {
            SkillEditorPage::Home => model.skills.pane = SkillsPane::Detail,
            SkillEditorPage::ReferenceEdit | SkillEditorPage::ReferenceRemove => {
                model.skills.editor_page = SkillEditorPage::Section(SkillSection::References);
            }
            SkillEditorPage::Review => {
                model.skills.editor_page = SkillEditorPage::Home;
                model.skills.editor.as_mut().unwrap().clear_review();
                return ControllerEffect::CancelSkillReview;
            }
            _ => model.skills.editor_page = SkillEditorPage::Home,
        }
        return ControllerEffect::Redraw;
    }
    if matches!(
        key.code,
        KeyCode::PageUp | KeyCode::PageDown | KeyCode::Home | KeyCode::End
    ) || (page == SkillEditorPage::Review && matches!(key.code, KeyCode::Up | KeyCode::Down))
        || (page == SkillEditorPage::Section(SkillSection::Instructions)
            && matches!(key.code, KeyCode::Up | KeyCode::Down))
    {
        return scroll(model, key.code);
    }
    if matches!(
        key.code,
        KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right
    ) {
        let forward = matches!(key.code, KeyCode::Down | KeyCode::Right);
        match page {
            SkillEditorPage::Home => {
                model.skills.editor_home_selection = move_grid(
                    model.skills.editor_home_selection.min(5),
                    key.code,
                    views::skill_editor_home_columns(model),
                    5,
                );
            }
            SkillEditorPage::Section(SkillSection::References) => {
                let count = model.skills.editor.as_ref().unwrap().references().len();
                model.skills.reference_selection = if forward {
                    (model.skills.reference_selection + 1).min(count)
                } else {
                    model.skills.reference_selection.saturating_sub(1)
                };
            }
            SkillEditorPage::Section(section) => {
                let fields = section.fields();
                let selected = fields
                    .iter()
                    .position(|f| *f == model.skills.editor.as_ref().unwrap().field())
                    .unwrap_or(0);
                let next = if forward {
                    (selected + 1).min(fields.len() - 1)
                } else {
                    selected.saturating_sub(1)
                };
                select_field(model, fields[next]);
            }
            SkillEditorPage::ReferenceEdit => {
                model.skills.reference_action = if forward {
                    (model.skills.reference_action + 1).min(2)
                } else {
                    model.skills.reference_action.saturating_sub(1)
                };
                if model.skills.reference_action < 2 {
                    select_field(
                        model,
                        [
                            SkillEditorField::ReferenceName,
                            SkillEditorField::ReferenceBody,
                        ][model.skills.reference_action],
                    );
                }
            }
            _ => {}
        }
        return ControllerEffect::Redraw;
    }
    if page == SkillEditorPage::Section(SkillSection::References) {
        match key.code {
            KeyCode::Char('n') => {
                model.skills.reference_selection =
                    model.skills.editor.as_ref().unwrap().references().len();
                return open_reference(model);
            }
            KeyCode::Char('x') => {
                let editor = model.skills.editor.as_mut().unwrap();
                if editor.has_pending_reference() {
                    model.set_message(
                        Severity::Warning,
                        "Finish the unfinished note before removing another.",
                    );
                } else if model.skills.reference_selection < editor.references().len() {
                    model.skills.editor_page = SkillEditorPage::ReferenceRemove;
                }
                return ControllerEffect::Redraw;
            }
            _ => {}
        }
    }
    if key.code != KeyCode::Enter {
        return ControllerEffect::None;
    }
    match page {
        SkillEditorPage::Home => {
            let selected = model.skills.editor_home_selection.min(5);
            if let Some(section) = SkillSection::ALL.get(selected).copied() {
                model.skills.editor_page = SkillEditorPage::Section(section);
                select_field(model, section.fields()[0]);
            } else if selected == 5 {
                model.skills.editor_page = SkillEditorPage::Discard;
            } else {
                return request_review(model);
            }
        }
        SkillEditorPage::Section(SkillSection::References) => return open_reference(model),
        SkillEditorPage::Section(_) => {
            model.skills.synchronize_field_input();
            model.set_input_mode(InputMode::Type);
        }
        SkillEditorPage::ReferenceEdit => {
            if model.skills.reference_action == 2 {
                if model.skills.editor.as_mut().unwrap().commit_tui_reference() {
                    model.skills.editor_page = SkillEditorPage::Section(SkillSection::References);
                }
            } else {
                model.skills.synchronize_field_input();
                model.set_input_mode(InputMode::Type);
            }
        }
        SkillEditorPage::ReferenceRemove => {
            select_reference(model);
            model
                .skills
                .editor
                .as_mut()
                .unwrap()
                .remove_selected_reference();
            model.skills.editor_page = SkillEditorPage::Section(SkillSection::References);
        }
        SkillEditorPage::Review => return request_review(model),
        SkillEditorPage::Discard => {
            model.skills.editor = None;
            model.skills.pending_confirmation = None;
            model.skills.field_input.clear();
            model.skills.pane = SkillsPane::Detail;
            return ControllerEffect::CancelSkillReview;
        }
    }
    ControllerEffect::Redraw
}

fn request_review(model: &mut TuiModel) -> ControllerEffect {
    let editor = model.skills.editor.as_mut().unwrap();
    model.skills.editor_page = SkillEditorPage::Review;
    model.skills.content_scroll = 0;
    if editor.go_to_review().is_err() {
        model.set_message(
            Severity::Warning,
            "Finish or correct the draft fields before reviewing.",
        );
        return ControllerEffect::Redraw;
    }
    let effect = editor.submit_keyboard_line("");
    apply_skill_editor_effect(model, effect)
}

fn select_reference(model: &mut TuiModel) {
    let editor = model.skills.editor.as_mut().unwrap();
    editor.clear_reference_selection();
    for _ in 0..=model.skills.reference_selection {
        editor.select_reference(true);
    }
}

fn open_reference(model: &mut TuiModel) -> ControllerEffect {
    if !model
        .skills
        .editor
        .as_ref()
        .unwrap()
        .has_pending_reference()
    {
        let count = model.skills.editor.as_ref().unwrap().references().len();
        if model.skills.reference_selection < count {
            select_reference(model);
            model
                .skills
                .editor
                .as_mut()
                .unwrap()
                .begin_edit_selected_reference();
        } else {
            model.skills.editor.as_mut().unwrap().begin_add_reference();
        }
    }
    model.skills.reference_action = 0;
    model.skills.editor_page = SkillEditorPage::ReferenceEdit;
    select_field(model, SkillEditorField::ReferenceName);
    ControllerEffect::Redraw
}

fn preview(model: &mut TuiModel, starter: bool) -> ControllerEffect {
    if starter {
        model.skills.create_source_detail = None;
        if model.skills.selected_create_source == 0 {
            return ControllerEffect::Redraw;
        }
    } else {
        model.skills.clear_skill_context();
    }
    let selected_skill = if starter {
        model.skills.selected_create_source - 1
    } else {
        model.skills.selected_skill
    };
    if selected_skill >= model.skills.library.skills.len() {
        return ControllerEffect::Redraw;
    }
    ControllerEffect::LoadSkillPreview {
        selected_skill,
        starter,
    }
}

pub(super) fn handle_workspace_key(
    model: &mut TuiModel,
    key: KeyEvent,
) -> Option<ControllerEffect> {
    if !text_modifiers(key.modifiers) {
        return None;
    }
    let code = match key.code {
        KeyCode::Char(c) => KeyCode::Char(c.to_ascii_lowercase()),
        code => code,
    };
    let pane = if model.focus == Focus::List && model.skills.pane != SkillsPane::CreateSource {
        SkillsPane::List
    } else {
        model.skills.pane
    };
    if matches!(code, KeyCode::Char('n' | 'c'))
        && matches!(pane, SkillsPane::List | SkillsPane::Detail)
    {
        if model.skills.editor.is_some() {
            model.skills.pane = SkillsPane::Editor;
            model.skills.editor_page = SkillEditorPage::Home;
            model.set_focus(Focus::Workspace);
            model.set_message(
                Severity::Info,
                "Resumed your draft. Review or discard it before starting another.",
            );
        } else {
            model.skills.pane = SkillsPane::CreateSource;
            model.skills.selected_create_source = 0;
            model.skills.create_source_detail = None;
            model.set_focus(Focus::List);
        }
        return Some(ControllerEffect::Redraw);
    }
    if code == KeyCode::Char('e') && matches!(pane, SkillsPane::Detail | SkillsPane::List) {
        if model.skills.editor.is_some() || model.skills.start_version() {
            model.skills.pane = SkillsPane::Editor;
            model.skills.editor_page = SkillEditorPage::Home;
            model.set_focus(Focus::Workspace);
        }
        return Some(ControllerEffect::Redraw);
    }
    if pane == SkillsPane::List && matches!(code, KeyCode::Up | KeyCode::Down) {
        let selected = model.skills.selected_skill;
        model.skills.selected_skill = if code == KeyCode::Down {
            (selected + 1).min(model.skills.library.skills.len().saturating_sub(1))
        } else {
            selected.saturating_sub(1)
        };
        return Some(if selected != model.skills.selected_skill {
            preview(model, false)
        } else {
            ControllerEffect::Redraw
        });
    }
    if pane == SkillsPane::CreateSource {
        match code {
            KeyCode::Up | KeyCode::Down => {
                let current = model.skills.selected_create_source;
                model.skills.selected_create_source = if code == KeyCode::Down {
                    (current + 1).min(model.skills.library.skills.len())
                } else {
                    current.saturating_sub(1)
                };
                return Some(if current != model.skills.selected_create_source {
                    preview(model, true)
                } else {
                    ControllerEffect::Redraw
                });
            }
            KeyCode::Enter => {
                model.set_focus(Focus::Workspace);
                if model.skills.selected_create_source == 0 {
                    model.skills.start_create(None);
                    return Some(ControllerEffect::Redraw);
                }
                return Some(ControllerEffect::LoadSkillStarter {
                    selected_skill: model.skills.selected_create_source - 1,
                });
            }
            KeyCode::Esc => {
                model.skills.pane = SkillsPane::List;
                model.set_focus(Focus::List);
                return Some(ControllerEffect::Redraw);
            }
            _ => {}
        }
    }
    if pane == SkillsPane::Detail {
        if code == KeyCode::Char('i') {
            model.skills.technical_details = !model.skills.technical_details;
            model.skills.content_scroll = 0;
            return Some(ControllerEffect::Redraw);
        }
        if matches!(
            code,
            KeyCode::PageUp
                | KeyCode::PageDown
                | KeyCode::Home
                | KeyCode::End
                | KeyCode::Up
                | KeyCode::Down
        ) {
            return Some(scroll(model, code));
        }
        if code == KeyCode::Enter && model.skills.editor.is_some() {
            if model.skills.selected_action() == SkillDetailAction::CreateVersion {
                model.skills.pane = SkillsPane::Editor;
                model.skills.editor_page = SkillEditorPage::Home;
            } else {
                model.set_message(
                    Severity::Warning,
                    "Resume your draft with E, then review or discard it first.",
                );
            }
            return Some(ControllerEffect::Redraw);
        }
        if code == KeyCode::Enter
            && model.skills.detail.as_ref().is_none_or(|detail| {
                model.skills.selected_summary().is_none_or(|summary| {
                    summary.skill_ref.skill_id() != detail.skill_ref.skill_id()
                })
            })
        {
            model.set_message(Severity::Info, "Open a skill from the library first.");
            return Some(ControllerEffect::Redraw);
        }
    }
    if pane == SkillsPane::AssignmentReview
        && matches!(
            code,
            KeyCode::PageUp
                | KeyCode::PageDown
                | KeyCode::Home
                | KeyCode::End
                | KeyCode::Up
                | KeyCode::Down
        )
    {
        return Some(scroll(model, code));
    }
    if code == KeyCode::Char('i')
        && matches!(
            pane,
            SkillsPane::History | SkillsPane::AssignmentReview | SkillsPane::Result
        )
    {
        model.skills.technical_details = !model.skills.technical_details;
        model.skills.content_scroll = 0;
        return Some(ControllerEffect::Redraw);
    }
    None
}
