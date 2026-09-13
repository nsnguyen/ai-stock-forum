use ratatui::style::{Color, Modifier, Style};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub base: Style,
    pub accent: Style,
    pub focus: Style,
    pub muted: Style,
    pub success: Style,
    pub warning: Style,
    pub error: Style,
}

impl Theme {
    pub fn agent_monogram(self, id: crate::domain::AgentProfileId) -> Style {
        if self.base.fg.is_none() {
            return self.accent;
        }
        let colors = [
            Color::Rgb(111, 202, 196),
            Color::Rgb(234, 176, 112),
            Color::Rgb(173, 155, 227),
            Color::Rgb(128, 183, 231),
            Color::Rgb(207, 160, 180),
        ];
        let bucket = id.as_uuid().as_bytes().iter().fold(0usize, |value, byte| {
            value.wrapping_mul(31).wrapping_add(usize::from(*byte))
        });
        Style::default()
            .fg(colors[bucket % colors.len()])
            .add_modifier(Modifier::BOLD)
    }

    pub fn from_no_color(no_color: bool) -> Self {
        if no_color {
            return Self {
                base: Style::default(),
                accent: Style::default().add_modifier(Modifier::BOLD),
                focus: Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED),
                muted: Style::default().add_modifier(Modifier::DIM),
                success: Style::default().add_modifier(Modifier::BOLD),
                warning: Style::default().add_modifier(Modifier::BOLD | Modifier::ITALIC),
                error: Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            };
        }

        Self {
            base: Style::default().fg(Color::Gray).bg(Color::Rgb(24, 24, 27)),
            accent: Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            focus: Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD | Modifier::REVERSED),
            muted: Style::default().fg(Color::Rgb(155, 160, 171)),
            success: Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
            warning: Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
            error: Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        }
    }

    pub fn styles(self) -> [Style; 7] {
        [
            self.base,
            self.accent,
            self.focus,
            self.muted,
            self.success,
            self.warning,
            self.error,
        ]
    }
}
