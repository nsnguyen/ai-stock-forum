use ratatui::layout::{Constraint, Layout, Rect};

use super::model::LayoutMode;

pub const MIN_WIDTH: u16 = 60;
pub const MIN_HEIGHT: u16 = 18;
pub const MEDIUM_WIDTH: u16 = 80;
pub const WIDE_WIDTH: u16 = 120;
pub const MEDIUM_HEIGHT: u16 = 24;
pub const WIDE_HEIGHT: u16 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CockpitLayout {
    pub mode: LayoutMode,
    pub viewport: Rect,
    pub header: Rect,
    pub navigation: Option<Rect>,
    pub workspace: Rect,
    pub inspector: Option<Rect>,
    pub message: Rect,
    pub command: Rect,
}

pub fn layout_mode(area: Rect) -> LayoutMode {
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        LayoutMode::TooSmall
    } else if area.width >= WIDE_WIDTH && area.height >= WIDE_HEIGHT {
        LayoutMode::Wide
    } else if area.width >= MEDIUM_WIDTH && area.height >= MEDIUM_HEIGHT {
        LayoutMode::Medium
    } else {
        LayoutMode::Narrow
    }
}

pub fn calculate(area: Rect, inspector_open: bool) -> CockpitLayout {
    let mode = layout_mode(area);
    if mode == LayoutMode::TooSmall {
        return CockpitLayout {
            mode,
            viewport: area,
            header: area,
            navigation: None,
            workspace: area,
            inspector: None,
            message: area,
            command: area,
        };
    }

    let bands = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(3),
    ])
    .split(area);
    let header = bands[0];
    let content = bands[1];
    let message = bands[2];
    let command = bands[3];

    let (navigation, workspace, inspector) = match mode {
        LayoutMode::Wide => {
            let columns = Layout::horizontal([
                Constraint::Length(22),
                Constraint::Fill(2),
                Constraint::Fill(1),
            ])
            .split(content);
            (Some(columns[0]), columns[1], Some(columns[2]))
        }
        LayoutMode::Medium => {
            let columns = Layout::horizontal([Constraint::Length(20), Constraint::Min(0)])
                .split(content);
            let workspace = columns[1];
            (
                Some(columns[0]),
                workspace,
                inspector_open.then(|| centered_overlay(workspace)),
            )
        }
        LayoutMode::Narrow => (
            None,
            content,
            inspector_open.then(|| centered_overlay(content)),
        ),
        LayoutMode::TooSmall => unreachable!("too-small mode returns before splitting"),
    };

    CockpitLayout {
        mode,
        viewport: area,
        header,
        navigation,
        workspace,
        inspector,
        message,
        command,
    }
}

fn centered_overlay(area: Rect) -> Rect {
    let overlay_width = seventy_percent(area.width);
    let overlay_height = seventy_percent(area.height);
    let horizontal = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(overlay_width),
        Constraint::Fill(1),
    ])
    .split(area);
    let vertical = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(overlay_height),
        Constraint::Fill(1),
    ])
    .split(area);

    Rect::new(horizontal[1].x, vertical[1].y, horizontal[1].width, vertical[1].height)
}

fn seventy_percent(value: u16) -> u16 {
    u16::try_from((u32::from(value) * 70) / 100).unwrap_or(value)
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;

    use super::{calculate, layout_mode};
    use crate::ui::tui::model::LayoutMode;

    #[test]
    fn exact_breakpoints_choose_the_documented_modes() {
        let cases = [
            (Rect::new(0, 0, 120, 30), LayoutMode::Wide),
            (Rect::new(0, 0, 119, 30), LayoutMode::Medium),
            (Rect::new(0, 0, 80, 24), LayoutMode::Medium),
            (Rect::new(0, 0, 79, 24), LayoutMode::Narrow),
            (Rect::new(0, 0, 60, 18), LayoutMode::Narrow),
            (Rect::new(0, 0, 59, 18), LayoutMode::TooSmall),
            (Rect::new(0, 0, 120, 17), LayoutMode::TooSmall),
        ];
        for (area, expected) in cases {
            assert_eq!(layout_mode(area), expected, "area={area:?}");
        }
    }

    #[test]
    fn wide_has_three_columns_and_medium_uses_an_overlay_inspector() {
        let wide = calculate(Rect::new(0, 0, 140, 40), true);
        assert!(wide.navigation.is_some());
        assert!(wide.inspector.is_some());
        assert!(wide.inspector.unwrap().x > wide.workspace.x);

        let medium = calculate(Rect::new(0, 0, 100, 30), true);
        assert!(medium.navigation.is_some());
        assert!(medium.inspector.is_some());
        assert!(medium.inspector.unwrap().width < medium.viewport.width);
    }

    #[test]
    fn narrow_uses_tabs_and_too_small_uses_the_whole_viewport() {
        let narrow = calculate(Rect::new(0, 0, 70, 20), true);
        assert_eq!(narrow.navigation, None);
        assert!(narrow.inspector.is_some());

        let tiny = calculate(Rect::new(4, 7, 40, 10), true);
        assert_eq!(tiny.mode, LayoutMode::TooSmall);
        assert_eq!(tiny.workspace, tiny.viewport);
        assert_eq!(tiny.navigation, None);
        assert_eq!(tiny.inspector, None);
    }

    #[test]
    fn content_bands_are_fixed_and_preserve_the_viewport_origin() {
        let layout = calculate(Rect::new(4, 7, 120, 30), false);

        assert_eq!(layout.header, Rect::new(4, 7, 120, 3));
        assert_eq!(layout.message, Rect::new(4, 33, 120, 1));
        assert_eq!(layout.command, Rect::new(4, 34, 120, 3));
        assert_eq!(layout.workspace.y, 10);
        assert_eq!(layout.workspace.height, 23);
    }

    #[test]
    fn overlays_are_centered_within_the_workspace_and_can_be_closed() {
        let medium = calculate(Rect::new(10, 20, 100, 30), true);
        let inspector = medium.inspector.expect("open inspector overlay");

        assert_eq!(inspector.width, 56);
        assert_eq!(inspector.height, 16);
        assert!(inspector.x >= medium.workspace.x);
        assert!(inspector.y >= medium.workspace.y);
        assert!(inspector.right() <= medium.workspace.right());
        assert!(inspector.bottom() <= medium.workspace.bottom());
        assert_eq!(calculate(Rect::new(0, 0, 100, 30), false).inspector, None);
        assert_eq!(calculate(Rect::new(0, 0, 70, 20), false).inspector, None);
    }

    #[test]
    fn extreme_dimensions_are_total_and_keep_every_rectangle_contained() {
        for area in [
            Rect::new(u16::MAX, u16::MAX, 0, 0),
            Rect::new(u16::MAX, u16::MAX, 1, 1),
            Rect::new(1, 1, u16::MAX, u16::MAX),
        ] {
            let layout = calculate(area, true);
            assert_eq!(layout.viewport, area);
            assert!(layout.header.x >= area.x && layout.header.y >= area.y);
            assert!(layout.header.right() <= area.right());
            assert!(layout.header.bottom() <= area.bottom());
            assert!(layout.workspace.x >= area.x && layout.workspace.y >= area.y);
            assert!(layout.workspace.right() <= area.right());
            assert!(layout.workspace.bottom() <= area.bottom());
            for rectangle in [layout.navigation, layout.inspector]
                .into_iter()
                .flatten()
            {
                assert!(rectangle.x >= area.x && rectangle.y >= area.y);
                assert!(rectangle.right() <= area.right());
                assert!(rectangle.bottom() <= area.bottom());
            }
        }
    }
}
