//! Small cell-grid illustrations shared by the everyday workspaces.
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Color, Style},
    text::Line,
    widgets::{Block, BorderType, Borders, Paragraph},
};

use crate::ui::tui::theme::Theme;

#[derive(Clone, Copy)]
pub(super) enum Icon {
    Agents,
    Profile,
    Memory,
    Skills,
    History,
    Setup,
}

impl Icon {
    fn lines(self) -> &'static [&'static str] {
        match self {
            Self::Agents => &[" o o ", "/| |\\", " / \\ "],
            Self::Profile => &[" (o) ", " /|\\ ", " / \\ "],
            Self::Memory => &["┌───┐", "│≡  │", "└───┘"],
            Self::Skills => &["  │  ", "│ │ ┃", "┃ ┃ ┃"],
            Self::History => &["╭───╮", "│ └ │", "╰───╯"],
            Self::Setup => &[" \\|/ ", "─(o)─", " /|\\ "],
        }
    }
}

pub(super) fn render_icon(frame: &mut Frame<'_>, area: Rect, icon: Icon, style: Style) {
    frame.render_widget(
        Paragraph::new(
            icon.lines()
                .iter()
                .map(|line| Line::styled(*line, style))
                .collect::<Vec<_>>(),
        ),
        area,
    );
}

pub(super) struct ActionCard<'a> {
    pub icon: Icon,
    pub title: &'a str,
    pub description: &'a str,
    pub selected: bool,
    pub focused: bool,
    pub accent: Style,
}

pub(super) fn render_action_card(
    frame: &mut Frame<'_>,
    area: Rect,
    card: ActionCard<'_>,
    theme: &Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let surface = if card.selected {
        theme.selected_surface()
    } else {
        theme.surface()
    };
    let border = if card.selected && card.focused {
        theme.accent
    } else {
        theme.border()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border)
        .style(surface);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let inset = u16::from(inner.width >= 12);
    let text = Rect {
        x: inner.x + inset,
        width: inner.width.saturating_sub(inset * 2),
        ..inner
    };
    let icon_height = if inner.height >= 5 {
        3
    } else if inner.height >= 3 {
        1
    } else {
        0
    };
    if icon_height > 0 {
        render_icon(
            frame,
            Rect {
                height: icon_height,
                ..text
            },
            card.icon,
            if card.selected {
                theme.accent
            } else {
                card.accent
            },
        );
    }
    let title = format!("{}{}", if card.selected { "> " } else { "  " }, card.title);
    frame.render_widget(
        Paragraph::new(Line::styled(
            title,
            if card.selected && card.focused {
                theme.focus
            } else {
                Style::default().fg(theme.base.fg.unwrap_or(Color::Reset))
            },
        )),
        Rect {
            y: text.y + icon_height,
            height: 1,
            ..text
        },
    );
    if inner.height > icon_height + 1 {
        frame.render_widget(
            Paragraph::new(Line::styled(card.description, theme.muted)),
            Rect {
                y: text.y + icon_height + 1,
                height: 1,
                ..text
            },
        );
    }
}

#[derive(Clone, Copy)]
pub(super) enum Artwork {
    Bear,
    Bull,
    Bot,
}

pub(super) fn render_artwork(frame: &mut Frame<'_>, area: Rect, artwork: Artwork, style: Style) {
    let lines: &[&str] = match artwork {
        Artwork::Bear => &[
            " /\\_____/\\ ",
            "(  o   o  )",
            " \\   Y   / ",
            " /  ___  \\ ",
            "(_/     \\_)",
        ],
        Artwork::Bull => &[
            "\\\\       //",
            " \\\\_____// ",
            " ( o   o ) ",
            " /   V   \\ ",
            " \\  ===  / ",
        ],
        Artwork::Bot => &[
            "    .-.    ",
            " .--' '--. ",
            " | o   o | ",
            " |  ---  | ",
            " '-------' ",
        ],
    };
    frame.render_widget(
        Paragraph::new(
            lines
                .iter()
                .map(|line| Line::styled(*line, style))
                .collect::<Vec<_>>(),
        )
        .alignment(Alignment::Center),
        area,
    );
}
