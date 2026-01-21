//! Stateful panel widget for the TUI dashboard.
//!
//! The panel renders titled content with scrolling and a keybinding footer that
//! dims when unfocused.
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

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
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

        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(inner);
        let content_area = sections[0];
        let keybinding_area = sections[1];

        let view_height = content_area.height as usize;
        let content_len = wrapped_content_len(&self.content, content_area.width);
        state.sync_scroll(content_len, view_height);
        let scroll_offset = state.scroll_offset.min(u16::MAX as usize) as u16;

        let paragraph = Paragraph::new(self.content)
            .scroll((scroll_offset, 0))
            .wrap(Wrap { trim: false });
        paragraph.render(content_area, buf);

        let line = keybinding_line(state, state.focused);
        let footer = Paragraph::new(line).wrap(Wrap { trim: true });
        footer.render(keybinding_area, buf);

        if content_area.height > 0 && content_area.width > 0 && content_len > view_height {
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

    /// Handles mouse scroll events without changing focus.
    pub fn handle_mouse_event(
        &mut self,
        mouse: MouseEvent,
        content_len: usize,
        view_height: usize,
    ) -> PanelEvent {
        let delta = match mouse.kind {
            MouseEventKind::ScrollUp => -1,
            MouseEventKind::ScrollDown => 1,
            _ => return PanelEvent::None,
        };

        if self.apply_scroll_delta(delta, content_len, view_height) {
            return PanelEvent::Scrolled;
        }

        PanelEvent::None
    }

    pub fn sync_scroll(&mut self, content_len: usize, view_height: usize) {
        let max_offset = max_scroll_offset(content_len, view_height);
        if self.scroll_offset > max_offset {
            self.scroll_offset = max_offset;
        }
        let scrollbar_content_len = scrollbar_content_length(content_len, view_height);
        self.scrollbar_state = ScrollbarState::new(scrollbar_content_len)
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
            let scrollbar_content_len = scrollbar_content_length(content_len, view_height);
            self.scrollbar_state = ScrollbarState::new(scrollbar_content_len)
                .position(self.scroll_offset)
                .viewport_content_length(view_height);
            return true;
        }
        false
    }
}

fn keybinding_line(state: &PanelState, focused: bool) -> Line<'static> {
    let (key_style, label_style) = if focused {
        (
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(Color::DarkGray),
        )
    } else {
        (
            Style::default().fg(Color::Gray).add_modifier(Modifier::DIM),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        )
    };
    let mut spans = Vec::new();
    let mut push_binding = |key_label: String, label: &str| {
        if !spans.is_empty() {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(key_label, key_style));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(label.to_string(), label_style));
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

fn scrollbar_content_length(content_len: usize, view_height: usize) -> usize {
    if content_len == 0 || view_height == 0 {
        return 0;
    }
    max_scroll_offset(content_len, view_height).saturating_add(1)
}

pub(crate) fn wrapped_content_len(content: &[Line], width: u16) -> usize {
    let width = width as usize;
    if width == 0 {
        return 0;
    }
    content
        .iter()
        .map(|line| wrapped_line_len(line, width))
        .sum()
}

fn wrapped_line_len(line: &Line, width: usize) -> usize {
    let line_width = line.width();
    let wrapped = line_width.saturating_add(width.saturating_sub(1)) / width;
    wrapped.max(1)
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
        KeyCode::BackTab => "Shift+Tab".to_string(),
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
    use ratatui::layout::Rect;

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
    fn vim_keys_adjust_offset_when_focused() {
        let mut state = PanelState::new();
        state.set_focused(true);

        let event =
            state.handle_key_event(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE), 8, 3);
        assert_eq!(state.scroll_offset(), 1);
        assert_eq!(event, PanelEvent::Scrolled);

        let event =
            state.handle_key_event(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE), 8, 3);
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
    fn mouse_scroll_adjusts_offset_without_focus() {
        let mut state = PanelState::new();

        let mouse = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };
        let event = state.handle_mouse_event(mouse, 10, 3);
        assert_eq!(state.scroll_offset(), 1);
        assert_eq!(event, PanelEvent::Scrolled);

        let mouse = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };
        let event = state.handle_mouse_event(mouse, 10, 3);
        assert_eq!(state.scroll_offset(), 0);
        assert_eq!(event, PanelEvent::Scrolled);
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
    fn focused_panel_renders_keybinding_footer() {
        let mut state = PanelState::new();
        state.set_focused(true);
        state.register_keybinding(KeyCode::Char('r'), KeyModifiers::NONE, "Refresh");

        let area = Rect::new(0, 0, 36, 5);
        let mut buffer = Buffer::empty(area);
        let panel = Panel::new("Title", Vec::new());
        panel.render(area, &mut buffer, &mut state);

        let footer_y = area.y + area.height - 2;
        let footer = row_text(&buffer, footer_y, area.width);
        assert!(footer.contains("j/k or Up/Down"));
        assert!(footer.contains("r Refresh"));
    }

    #[test]
    fn unfocused_panel_renders_keybinding_footer() {
        let mut state = PanelState::new();

        let area = Rect::new(0, 0, 36, 5);
        let mut buffer = Buffer::empty(area);
        let panel = Panel::new("Title", Vec::new());
        panel.render(area, &mut buffer, &mut state);

        let footer_y = area.y + area.height - 2;
        let footer = row_text(&buffer, footer_y, area.width);
        assert!(footer.contains("j/k or Up/Down"));
    }

    #[test]
    fn backtab_keybinding_displays_as_shift_tab() {
        let binding = PanelKeybinding::new(KeyCode::BackTab, KeyModifiers::NONE, "Prev panel");

        let label = format_keybinding(&binding);

        assert_eq!(label, "Shift+Tab");
    }

    #[test]
    fn sync_scroll_clamps_offset() {
        let mut state = PanelState::new();
        state.scroll_offset = 10;
        state.sync_scroll(5, 3);
        assert_eq!(state.scroll_offset, 2);
    }

    #[test]
    fn short_content_does_not_scroll() {
        let mut state = PanelState::new();
        state.set_focused(true);

        let event = state.handle_key_event(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), 1, 3);

        assert_eq!(state.scroll_offset(), 0);
        assert_eq!(event, PanelEvent::None);
    }

    #[test]
    fn scroll_reaches_bottom_and_top() {
        let mut state = PanelState::new();
        state.set_focused(true);

        let content_len = 5;
        let view_height = 2;
        for _ in 0..10 {
            state.handle_key_event(
                KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
                content_len,
                view_height,
            );
        }
        assert_eq!(state.scroll_offset(), 3);

        for _ in 0..10 {
            state.handle_key_event(
                KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
                content_len,
                view_height,
            );
        }
        assert_eq!(state.scroll_offset(), 0);
    }

    #[test]
    fn wrapped_content_len_counts_wrapped_lines() {
        let content = vec![Line::from("12345678")];

        let content_len = wrapped_content_len(&content, 4);

        assert_eq!(content_len, 2);
    }

    #[test]
    fn render_keeps_scroll_offset_for_wrapped_content() {
        let mut state = PanelState::new();
        state.set_focused(true);
        state.scroll_offset = 1;

        let area = Rect::new(0, 0, 6, 4);
        let mut buffer = Buffer::empty(area);
        let panel = Panel::new("Title", vec![Line::from("12345678")]);
        panel.render(area, &mut buffer, &mut state);

        assert_eq!(state.scroll_offset(), 1);
    }

    #[test]
    fn scrollbar_hides_for_exact_fit() {
        let area = Rect::new(0, 0, 20, 10);
        let lines = sample_lines(7);
        let (buffer, content_area) = render_panel_with_content(area, lines);

        let thumb_height = thumb_height(&buffer, content_area);
        assert_eq!(thumb_height, 0);
    }

    #[test]
    fn scrollbar_not_rendered_for_non_scrollable_content() {
        let area = Rect::new(0, 0, 20, 10);
        let lines = sample_lines(7);
        let (buffer, content_area) = render_panel_with_content(area, lines);

        assert!(!has_scrollbar(&buffer, content_area));
    }

    #[test]
    fn scrollbar_rendered_for_overflowing_content() {
        let area = Rect::new(0, 0, 20, 10);
        let lines = sample_lines(8);
        let (buffer, content_area) = render_panel_with_content(area, lines);

        assert!(has_scrollbar(&buffer, content_area));
    }

    #[test]
    fn scrollbar_thumb_shrinks_for_single_line_overflow() {
        let area = Rect::new(0, 0, 20, 10);
        let lines = sample_lines(8);
        let (buffer, content_area) = render_panel_with_content(area, lines);

        let thumb_height = thumb_height(&buffer, content_area);
        assert_eq!(thumb_height, track_length(content_area).saturating_sub(1));
    }

    #[test]
    fn scrollbar_thumb_clamps_for_large_overflow() {
        let area = Rect::new(0, 0, 20, 10);
        let lines = sample_lines(30);
        let (buffer, content_area) = render_panel_with_content(area, lines);

        let thumb_height = thumb_height(&buffer, content_area);
        assert_eq!(thumb_height, 1);
    }

    #[test]
    fn scrollbar_hides_when_no_content() {
        let area = Rect::new(0, 0, 20, 10);
        let lines = Vec::new();
        let (buffer, content_area) = render_panel_with_content(area, lines);

        let thumb_height = thumb_height(&buffer, content_area);
        assert_eq!(thumb_height, 0);
    }

    fn row_text(buffer: &Buffer, y: u16, width: u16) -> String {
        let mut row = String::new();
        for x in 0..width {
            row.push_str(buffer[(x, y)].symbol());
        }
        row.trim_end().to_string()
    }

    fn render_panel_with_content(area: Rect, lines: Vec<Line<'static>>) -> (Buffer, Rect) {
        let mut state = PanelState::new();
        state.set_focused(true);

        let mut buffer = Buffer::empty(area);
        let panel = Panel::new("Title", lines);
        panel.render(area, &mut buffer, &mut state);

        (buffer, panel_content_area(area))
    }

    fn panel_content_area(area: Rect) -> Rect {
        let inner = Block::default().borders(Borders::ALL).inner(area);
        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(inner);
        sections[0]
    }

    fn track_length(content_area: Rect) -> usize {
        content_area.height.saturating_sub(2) as usize
    }

    fn thumb_height(buffer: &Buffer, content_area: Rect) -> usize {
        let scrollbar_x = content_area
            .x
            .saturating_add(content_area.width.saturating_sub(1));
        let thumb_symbol = ratatui::symbols::scrollbar::DOUBLE_VERTICAL.thumb;
        let mut count = 0;
        for y in content_area.y..content_area.y.saturating_add(content_area.height) {
            if buffer[(scrollbar_x, y)].symbol() == thumb_symbol {
                count += 1;
            }
        }
        count
    }

    fn has_scrollbar(buffer: &Buffer, content_area: Rect) -> bool {
        let scrollbar_x = content_area
            .x
            .saturating_add(content_area.width.saturating_sub(1));
        let symbols = ratatui::symbols::scrollbar::DOUBLE_VERTICAL;
        for y in content_area.y..content_area.y.saturating_add(content_area.height) {
            let symbol = buffer[(scrollbar_x, y)].symbol();
            if symbol == symbols.thumb
                || symbol == symbols.track
                || symbol == symbols.begin
                || symbol == symbols.end
            {
                return true;
            }
        }
        false
    }

    fn sample_lines(count: usize) -> Vec<Line<'static>> {
        (0..count)
            .map(|idx| Line::from(format!("Line {idx}")))
            .collect()
    }
}
