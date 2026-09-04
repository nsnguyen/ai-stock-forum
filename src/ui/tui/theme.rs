use ratatui::style::{Color, Modifier, Style};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub accent: Style,
    pub focus: Style,
    pub muted: Style,
    pub success: Style,
    pub warning: Style,
    pub error: Style,
}

impl Theme {
    pub fn from_no_color(no_color: bool) -> Self {
        if no_color {
            return Self {
                accent: Style::default().add_modifier(Modifier::BOLD),
                focus: Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED),
                muted: Style::default().add_modifier(Modifier::DIM),
                success: Style::default().add_modifier(Modifier::BOLD),
                warning: Style::default().add_modifier(Modifier::BOLD | Modifier::ITALIC),
                error: Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            };
        }

        Self {
            accent: Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            focus: Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD | Modifier::REVERSED),
            muted: Style::default().fg(Color::DarkGray),
            success: Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            warning: Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            error: Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        }
    }

    pub fn styles(self) -> [Style; 6] {
        [
            self.accent,
            self.focus,
            self.muted,
            self.success,
            self.warning,
            self.error,
        ]
    }
}
