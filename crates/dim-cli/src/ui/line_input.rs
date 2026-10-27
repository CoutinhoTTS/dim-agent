//! A one-line text field on the composer's editor: the same keys (cursor
//! movement, word moves, Home/End, Delete, Ctrl shortcuts), pastes, and a
//! view that scrolls sideways so the cursor stays visible. Forms use it for
//! their inputs so a field edits like the composer, not like a buffer that
//! only appends and pops.

use crate::ui::terminal::{KeyCode, KeyEvent, to_crossterm};
use crate::ui::text::display_width;
use xai_ratatui_textarea::TextArea;

pub struct LineInput {
    area: TextArea,
}

impl LineInput {
    pub fn new(text: &str) -> Self {
        let mut area = TextArea::new();
        area.set_text(text);
        area.set_cursor(text.len());
        Self { area }
    }

    pub fn text(&self) -> &str {
        self.area.text()
    }

    pub fn is_empty(&self) -> bool {
        self.area.text().is_empty()
    }

    pub fn clear(&mut self) {
        self.area.set_text("");
        self.area.set_cursor(0);
    }

    /// Byte offset of the cursor in `text`.
    pub fn cursor(&self) -> usize {
        self.area.cursor().min(self.area.text().len())
    }

    /// Edits with `key`; `false` leaves the key to the owner: the keys a
    /// form navigates or submits with, which a line never takes.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        if matches!(
            key.code,
            KeyCode::Enter
                | KeyCode::Tab
                | KeyCode::BackTab
                | KeyCode::Up
                | KeyCode::Down
                | KeyCode::PageUp
                | KeyCode::PageDown
                | KeyCode::Esc
        ) {
            return false;
        }
        if let Some(event) = to_crossterm(key) {
            self.area.input(event);
        }
        true
    }

    /// Inserts pasted text at the cursor, on one line: a line break becomes
    /// a space, other controls go.
    pub fn paste(&mut self, text: &str) {
        let line: String = text
            .replace("\r\n", "\n")
            .chars()
            .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
            .filter(|c| !c.is_control())
            .collect();
        self.area.insert_str(&line);
    }

    /// `width` cells of the text with the cursor inside them, and the
    /// cursor's column within those cells. The window keeps one cell for a
    /// cursor at the end of the text.
    pub fn view(&self, width: usize) -> (String, usize) {
        window(self.text(), self.cursor(), width)
    }
}

/// `width` cells of `text` with the cursor's column inside them (one cell
/// is kept for a cursor at the end), and that column within the window.
pub(crate) fn window(text: &str, cursor: usize, width: usize) -> (String, usize) {
    let width = width.max(1);
    let column = display_width(&text[..cursor]);
    let start = column.saturating_sub(width - 1);
    let end = start + width;
    let mut shown = String::new();
    let mut at = 0;
    for ch in text.chars() {
        let w = display_width(&ch.to_string());
        if at >= start && at + w <= end {
            shown.push(ch);
        }
        at += w;
        if at >= end {
            break;
        }
    }
    (shown, column - start)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::terminal::Modifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, Modifiers::NONE)
    }

    fn typed(input: &mut LineInput, text: &str) {
        for c in text.chars() {
            assert!(input.handle_key(key(KeyCode::Char(c))));
        }
    }

    #[test]
    fn edits_at_the_cursor_like_the_composer() {
        let mut input = LineInput::new("");
        typed(&mut input, "https://api.example");
        assert!(input.handle_key(key(KeyCode::Home)));
        assert_eq!(input.cursor(), 0);
        assert!(input.handle_key(key(KeyCode::Delete)));
        assert_eq!(input.text(), "ttps://api.example");
        typed(&mut input, "h");
        assert!(input.handle_key(key(KeyCode::End)));
        assert!(input.handle_key(key(KeyCode::Left)));
        assert!(input.handle_key(key(KeyCode::Backspace)));
        assert_eq!(input.text(), "https://api.exampe");
        for code in [KeyCode::Enter, KeyCode::Tab, KeyCode::Up, KeyCode::Esc] {
            assert!(!input.handle_key(key(code)));
        }
        assert_eq!(input.text(), "https://api.exampe");
    }

    #[test]
    fn a_paste_lands_at_the_cursor_on_one_line() {
        let mut input = LineInput::new("ab");
        assert!(input.handle_key(key(KeyCode::Left)));
        input.paste("1\n2\r\n3\x1b[2J");
        assert_eq!(input.text(), "a1 2 3[2Jb");
        input.clear();
        assert!(input.is_empty());
        assert_eq!(input.cursor(), 0);
    }

    #[test]
    fn the_view_scrolls_so_the_cursor_stays_visible() {
        assert_eq!(window("abc", 3, 10), ("abc".into(), 3));
        assert_eq!(window("abcdefgh", 8, 4), ("fgh".into(), 3));
        assert_eq!(window("abcdefgh", 2, 4), ("abcd".into(), 2));
        assert_eq!(window("abcdefgh", 5, 4), ("cdef".into(), 3));
        assert_eq!(window("中文字", 6, 3), ("文".into(), 2));
    }
}
