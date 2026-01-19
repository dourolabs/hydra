//! Stateful panel widget for the TUI dashboard.
//!
//! The panel renders titled content with scrolling and an optional keybinding
//! footer when focused.
//!
//! ```no_run
//! use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
//! use metis::command::dashboard::panel::{Panel, PanelEvent, PanelState};
//! use ratatui::{layout::Rect, text::Line, Frame};
//!
//! fn render_panel(frame: &mut Frame, area: Rect, key: Option<KeyEvent>) {
//!     let mut state = PanelState::new();
//!     state.set_focused(true);
//!     state.register_keybinding(KeyCode::Char('r'), KeyModifiers::NONE, "Refresh");
//!
//!     if let Some(key) = key {
//!         let _ = state.handle_key_event(key, 12, area.height as usize);
//!     }
//!
//!     let lines = vec![Line::from("Line 1"), Line::from("Line 2")];
//!     let panel = Panel::new("Recent Activity", lines);
//!     frame.render_stateful_widget(panel, area, &mut state);
//! }
//! ```

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, StatefulWidget,
        Widget, Wrap,
    },
};

#[derive(Clone, Debug)]
pub struct Panel<'a> {
    title: Line<'a>,
    content: Vec<Line<'a>>,
}

impl<'a> Panel<'a> {
    pub fn new<T>(title: T, content: Vec<Line<'a>>) -> Self
    where
        T: Into<Line<'a>>,
    {
        Self {
            title: title.into(),
            content,
        }
    }
}

impl StatefulWidget for Panel<'_> {
    type State = PanelState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let border_style = if state.focused {
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let block = Block::default()
            .title(self.title)
            .borders(Borders::ALL)
            .border_style(border_style);
        let inner = block.inner(area);
        block.render(area, buf);

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let (content_area, keybinding_area) = if state.focused {
            let sections = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0), Constraint::Length(1)])
                .split(inner);
            (sections[0], Some(sections[1]))
        } else {
            (inner, None)
        };

        let view_height = content_area.height as usize;
        state.sync_scroll(self.content.len(), view_height);
        let scroll_offset = state.scroll_offset.min(u16::MAX as usize) as u16;

        let paragraph = Paragraph::new(self.content)
            .scroll((scroll_offset, 0))
            .wrap(Wrap { trim: false });
        paragraph.render(content_area, buf);

        if let Some(area) = keybinding_area {
            let line = keybinding_line(state);
            let footer = Paragraph::new(line).wrap(Wrap { trim: true });
            footer.render(area, buf);
        }

        if content_area.height > 0 && content_area.width > 0 {
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .thumb_style(Style::default().fg(Color::White))
                .track_style(Style::default().fg(Color::DarkGray));
            scrollbar.render(content_area, buf, &mut state.scrollbar_state);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelKeybinding {
    code: KeyCode,
    modifiers: KeyModifiers,
    label: String,
}

impl PanelKeybinding {
    fn new(code: KeyCode, modifiers: KeyModifiers, label: impl Into<String>) -> Self {
        Self {
            code,
            modifiers,
            label: label.into(),
        }
    }

    fn matches(&self, key: KeyEvent) -> bool {
        self.code == key.code && self.modifiers == key.modifiers
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PanelEvent {
    None,
    Scrolled,
    Keybinding(PanelKeybinding),
}

#[derive(Clone, Debug)]
pub struct PanelState {
    scroll_offset: usize,
    scrollbar_state: ScrollbarState,
    focused: bool,
    keybindings: Vec<PanelKeybinding>,
}

impl Default for PanelState {
    fn default() -> Self {
        Self::new()
    }
}

impl PanelState {
    pub fn new() -> Self {
        Self {
            scroll_offset: 0,
            scrollbar_state: ScrollbarState::default(),
            focused: false,
            keybindings: Vec::new(),
        }
    }

    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    pub fn focused(&self) -> bool {
        self.focused
    }

    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    pub fn register_keybinding(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        label: impl Into<String>,
    ) {
        let binding = PanelKeybinding::new(code, modifiers, label);
        if let Some(existing) = self
            .keybindings
            .iter_mut()
            .find(|entry| entry.code == code && entry.modifiers == modifiers)
        {
            *existing = binding;
        } else {
            self.keybindings.push(binding);
        }
    }

    pub fn clear_keybindings(&mut self) {
        self.keybindings.clear();
    }

    /// Handles scroll keys and registered keybindings while focused.
    pub fn handle_key_event(
        &mut self,
        key: KeyEvent,
        content_len: usize,
        view_height: usize,
    ) -> PanelEvent {
        if !self.focused {
            return PanelEvent::None;
        }

        if is_scroll_key(key) {
            if self.apply_scroll_delta(scroll_delta(key), content_len, view_height) {
                return PanelEvent::Scrolled;
            }
            return PanelEvent::None;
        }

        if let Some(binding) = self.keybindings.iter().find(|binding| binding.matches(key)) {
            return PanelEvent::Keybinding(binding.clone());
        }

        PanelEvent::None
    }

    pub fn sync_scroll(&mut self, content_len: usize, view_height: usize) {
        let max_offset = max_scroll_offset(content_len, view_height);
        if self.scroll_offset > max_offset {
            self.scroll_offset = max_offset;
        }
        self.scrollbar_state = ScrollbarState::new(content_len)
            .position(self.scroll_offset)
            .viewport_content_length(view_height);
    }

    /// Applies a scroll delta and returns true when the offset changes.
    pub fn apply_scroll_delta(
        &mut self,
        delta: i32,
        content_len: usize,
        view_height: usize,
    ) -> bool {
        let max_offset = max_scroll_offset(content_len, view_height);
        let next_offset = if delta < 0 {
            self.scroll_offset
                .saturating_sub(delta.unsigned_abs() as usize)
        } else {
            self.scroll_offset.saturating_add(delta as usize)
        };
        let clamped = next_offset.min(max_offset);
        if clamped != self.scroll_offset {
            self.scroll_offset = clamped;
            self.scrollbar_state = ScrollbarState::new(content_len)
                .position(self.scroll_offset)
                .viewport_content_length(view_height);
            return true;
        }
        false
    }
}

fn keybinding_line(state: &PanelState) -> Line<'static> {
    let mut spans = Vec::new();
    let mut push_binding = |key_label: String, label: &str| {
        if !spans.is_empty() {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(
            key_label,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            label.to_string(),
            Style::default().fg(Color::DarkGray),
        ));
    };

    push_binding("j/k or Up/Down".to_string(), "Scroll");
    for binding in &state.keybindings {
        push_binding(format_keybinding(binding), &binding.label);
    }

    Line::from(spans)
}

fn is_scroll_key(key: KeyEvent) -> bool {
    if !key.modifiers.is_empty() {
        return false;
    }

    matches!(key.code, KeyCode::Up | KeyCode::Down)
        || matches!(
            key.code,
            KeyCode::Char('j') | KeyCode::Char('k') | KeyCode::Char('J') | KeyCode::Char('K')
        )
}

fn scroll_delta(key: KeyEvent) -> i32 {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('K') => -1,
        KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('J') => 1,
        _ => 0,
    }
}

fn max_scroll_offset(content_len: usize, view_height: usize) -> usize {
    if view_height == 0 {
        return 0;
    }
    content_len.saturating_sub(view_height)
}

fn format_keybinding(binding: &PanelKeybinding) -> String {
    let mut parts = Vec::new();
    let modifiers = binding.modifiers;
    if modifiers.contains(KeyModifiers::CONTROL) {
        parts.push("Ctrl".to_string());
    }
    if modifiers.contains(KeyModifiers::ALT) {
        parts.push("Alt".to_string());
    }
    if modifiers.contains(KeyModifiers::SHIFT) {
        parts.push("Shift".to_string());
    }
    if modifiers.contains(KeyModifiers::META) {
        parts.push("Meta".to_string());
    }
    parts.push(format_key_code(binding.code));

    parts.join("+")
}

fn format_key_code(code: KeyCode) -> String {
    match code {
        KeyCode::Backspace => "Backspace".to_string(),
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Left => "Left".to_string(),
        KeyCode::Right => "Right".to_string(),
        KeyCode::Up => "Up".to_string(),
        KeyCode::Down => "Down".to_string(),
        KeyCode::Home => "Home".to_string(),
        KeyCode::End => "End".to_string(),
        KeyCode::PageUp => "PageUp".to_string(),
        KeyCode::PageDown => "PageDown".to_string(),
        KeyCode::Tab => "Tab".to_string(),
        KeyCode::BackTab => "BackTab".to_string(),
        KeyCode::Delete => "Delete".to_string(),
        KeyCode::Insert => "Insert".to_string(),
        KeyCode::F(value) => format!("F{value}"),
        KeyCode::Char(' ') => "Space".to_string(),
        KeyCode::Char(value) => value.to_string(),
        KeyCode::Null => "Null".to_string(),
        KeyCode::Esc => "Esc".to_string(),
        _ => format!("{code:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_keys_adjust_offset_when_focused() {
        let mut state = PanelState::new();
        state.set_focused(true);

        let event = state.handle_key_event(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), 10, 3);
        assert_eq!(state.scroll_offset(), 1);
        assert_eq!(event, PanelEvent::Scrolled);

        let event = state.handle_key_event(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), 10, 3);
        assert_eq!(state.scroll_offset(), 0);
        assert_eq!(event, PanelEvent::Scrolled);
    }

    #[test]
    fn scroll_keys_do_not_fire_when_unfocused() {
        let mut state = PanelState::new();

        let event = state.handle_key_event(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), 10, 3);
        assert_eq!(state.scroll_offset(), 0);
        assert_eq!(event, PanelEvent::None);
    }

    #[test]
    fn custom_keybindings_return_event() {
        let mut state = PanelState::new();
        state.set_focused(true);
        state.register_keybinding(KeyCode::Char('r'), KeyModifiers::NONE, "Refresh");

        let event =
            state.handle_key_event(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE), 0, 0);

        match event {
            PanelEvent::Keybinding(binding) => {
                assert_eq!(binding.label, "Refresh");
            }
            _ => panic!("expected keybinding event"),
        }
    }

    #[test]
    fn sync_scroll_clamps_offset() {
        let mut state = PanelState::new();
        state.scroll_offset = 10;
        state.sync_scroll(5, 3);
        assert_eq!(state.scroll_offset, 2);
    }
}
