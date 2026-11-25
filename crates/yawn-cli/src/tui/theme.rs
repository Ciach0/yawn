use crate::event::Level;
use ratatui::style::{Color, Modifier, Style};

#[derive(Copy, Clone, Debug)]
pub struct Theme {
    pub color: bool,
}

impl Theme {
    fn fg(&self, c: Color) -> Style {
        if self.color { Style::new().fg(c) } else { Style::new() }
    }

    pub fn base(&self) -> Style {
        Style::new()
    }

    pub fn dim(&self) -> Style {
        Style::new().add_modifier(Modifier::DIM)
    }

    pub fn strong(&self) -> Style {
        Style::new().add_modifier(Modifier::BOLD)
    }

    pub fn accent(&self) -> Style {
        self.fg(Color::Cyan).add_modifier(Modifier::BOLD)
    }

    pub fn ok(&self) -> Style {
        self.fg(Color::Green).add_modifier(Modifier::BOLD)
    }

    pub fn warn(&self) -> Style {
        self.fg(Color::Yellow)
    }

    pub fn error(&self) -> Style {
        self.fg(Color::Red).add_modifier(Modifier::BOLD)
    }

    pub fn level(&self, l: Level) -> Style {
        match l {
            Level::Error => self.error(),
            Level::Warn => self.warn(),
            Level::Info => self.fg(Color::Cyan),
            Level::Debug | Level::Trace => self.dim(),
        }
    }

    pub fn selected(&self) -> Style {
        Style::new().add_modifier(Modifier::REVERSED)
    }
}
