//! The write path of the scrollback interface (design §6.2, §6.3).
//!
//! History is committed once into the terminal's native scrollback; only the
//! live region at the bottom of the screen is ever repainted. `Screen` turns a
//! frame description into one byte string: commits through the vendored
//! `emit_to_scrollback` (wrap-aware, soft/hard breaks), a row diff of the live
//! region, and cursor movement/visibility only when they changed. A frame with
//! nothing new produces no bytes at all, which keeps the hardware cursor
//! blinking.

use std::io::{self, Write};

use ratatui::layout::{Rect, Size};
use xai_ratatui_inline::{TerminalLike, emit_to_scrollback};

/// History rows kept visible when the live region grows (design §6.1).
pub const MIN_HISTORY_ROWS: u16 = 4;

const BEGIN_SYNC: &[u8] = b"\x1b[?2026h";
const END_SYNC: &[u8] = b"\x1b[?2026l";
const CURSOR_SHOW: &[u8] = b"\x1b[?25h";
const CURSOR_HIDE: &[u8] = b"\x1b[?25l";
const ERASE_LINE: &[u8] = b"\x1b[2K";
/// Erases from the cursor to the end of the row: written after a row's text,
/// so the row is never blank between two frames.
const ERASE_LINE_END: &[u8] = b"\x1b[K";
const ERASE_DOWN: &[u8] = b"\x1b[J";

/// What one frame wants on screen.
#[derive(Debug, Default, Clone)]
pub struct Frame {
    /// Text to commit into scrollback before painting. Each entry is a whole
    /// block; `\n` separates logical lines.
    pub commits: Vec<String>,
    /// The live region, one string per row, already wrapped to the width.
    /// Styling escapes are allowed; control characters are not.
    pub live: Vec<String>,
    /// Cursor position inside the live region (row, column), `None` hides it.
    pub cursor: Option<(u16, u16)>,
    /// Index in `live` where the dock (queue, composer, panels, status)
    /// starts; the rows before it are the transcript's live tail. The
    /// fullscreen interface scrolls the tail with the transcript and pins
    /// the dock to the bottom.
    pub dock: usize,
    /// A box the fullscreen interface centres over the transcript (the
    /// command palette, pickers); the scrollback interface never gets one.
    pub overlay: Option<Overlay>,
}

/// Rows drawn centred over the fullscreen transcript.
#[derive(Debug, Default, Clone)]
pub struct Overlay {
    /// Rows of equal display width.
    pub rows: Vec<String>,
    /// Cursor inside the box (row, column); `None` keeps the composer's.
    pub cursor: Option<(u16, u16)>,
}

pub struct Screen {
    size: (u16, u16),
    viewport_height: u16,
    /// First screen row of the live region. It follows the end of the
    /// committed content down the screen and is pinned to the bottom once
    /// the content reaches it; shrinking the region moves it down, so the
    /// region's bottom row (the composer's) never moves up.
    top: u16,
    /// Cursor row of the shell when the interface started; `None` when the
    /// terminal did not answer, which pins the region to the bottom.
    origin: Option<u16>,
    /// First screen row holding rows this interface wrote (0 once they have
    /// reached the top).
    content_start: u16,
    /// Some of those rows already left through the top of the screen.
    scrolled: bool,
    /// The screen row the cursor was parked on after the last frame; a
    /// resize report is compared against it to find where the region went.
    anchor: u16,
    /// Rows as last painted; `None` entries are unknown (repaint).
    painted: Vec<Option<String>>,
    cursor_visible: bool,
    /// Where the terminal's cursor is after our last write, when known.
    cursor_at: Option<(u16, u16)>,
    synchronized: bool,
}

impl Screen {
    /// `synchronized`: emit DEC 2026 around frames (false inside tmux).
    pub fn new(size: (u16, u16), synchronized: bool) -> Self {
        Self {
            size,
            viewport_height: 0,
            top: size.1,
            origin: None,
            content_start: 0,
            scrolled: false,
            anchor: 0,
            painted: Vec::new(),
            cursor_visible: true,
            cursor_at: None,
            synchronized,
        }
    }

    pub fn size(&self) -> (u16, u16) {
        self.size
    }

    /// Where the shell left the cursor: the region starts there instead of
    /// at the bottom of the screen.
    pub fn set_origin(&mut self, row: u16) {
        self.origin = Some(row);
    }

    #[cfg(test)]
    pub fn viewport_height(&self) -> u16 {
        self.viewport_height
    }

    /// How many rows the region takes for `live` rows at `size`.
    pub fn wanted_for(&self, size: (u16, u16), live: usize) -> u16 {
        let cap = size.1.saturating_sub(MIN_HISTORY_ROWS).max(1);
        live.clamp(1, cap as usize) as u16
    }

    fn cap(&self) -> u16 {
        self.size.1.saturating_sub(MIN_HISTORY_ROWS).max(1)
    }

    fn viewport_top(&self) -> u16 {
        self.top
    }

    /// Where the region starts once it is `wanted` rows tall.
    fn top_for(&self, wanted: u16) -> u16 {
        self.top.min(self.size.1.saturating_sub(wanted))
    }

    /// Renders `frame` for a terminal of `size`; returns the bytes to write
    /// (empty when nothing changed).
    pub fn render(&mut self, frame: &Frame, size: (u16, u16)) -> Vec<u8> {
        let mut out = Vec::new();
        let resized = size != self.size;
        let cap = if resized {
            size.1.saturating_sub(MIN_HISTORY_ROWS).max(1)
        } else {
            self.cap()
        };
        let wanted = frame.live.len().clamp(1, cap as usize) as u16;
        let rows_changed = self.painted_differs(frame, wanted);
        let nothing = frame.commits.is_empty()
            && !resized
            && !rows_changed
            && wanted == self.viewport_height
            && !self.cursor_needs_move(frame, wanted)
            && self.cursor_visible == frame.cursor.is_some();
        if nothing {
            return out;
        }

        // The first frame lays the viewport out; commits need it to exist.
        if self.viewport_height == 0 {
            self.size = size;
            self.viewport_height = wanted;
            self.painted = vec![None; wanted as usize];
            self.cursor_at = None;
            self.begin(&mut out);
            self.hide_for_paint(&mut out);
            let bottom = size.1.saturating_sub(1);
            match self.origin {
                Some(origin) => {
                    // Start where the shell stopped; scroll only as much as
                    // the region needs below that row.
                    let origin = origin.min(bottom);
                    let top = origin.min(size.1.saturating_sub(wanted));
                    let overflow = (origin + wanted).saturating_sub(size.1);
                    if overflow > 0 {
                        push_move(&mut out, 0, bottom);
                        out.extend(std::iter::repeat_n(b"\n", overflow as usize).flatten());
                    }
                    self.top = top;
                }
                None => {
                    // Make room at the bottom: the live region owns these rows.
                    self.top = size.1.saturating_sub(wanted);
                    push_move(&mut out, 0, bottom);
                    out.extend(
                        std::iter::repeat_n(b"\n", wanted.saturating_sub(1) as usize).flatten(),
                    );
                }
            }
            self.content_start = self.top;
            push_move(&mut out, 0, self.top);
            out.extend_from_slice(ERASE_DOWN);
            self.cursor_at = Some((0, self.top));
        } else {
            self.begin(&mut out);
            if resized {
                self.resize_to(size, wanted, &mut out);
            }
            self.hide_for_paint(&mut out);
        }

        for commit in &frame.commits {
            self.commit(commit, &mut out);
        }
        if wanted != self.viewport_height {
            self.set_height(wanted, &mut out);
        }
        self.paint(frame, &mut out);
        self.place_cursor(frame, &mut out);
        self.end(&mut out);
        out
    }

    fn begin(&self, out: &mut Vec<u8>) {
        if self.synchronized {
            out.extend_from_slice(BEGIN_SYNC);
        }
    }

    fn end(&self, out: &mut Vec<u8>) {
        if self.synchronized {
            out.extend_from_slice(END_SYNC);
        }
    }

    /// Without synchronized output, hide the cursor while rows are rewritten.
    fn hide_for_paint(&mut self, out: &mut Vec<u8>) {
        if !self.synchronized && self.cursor_visible {
            out.extend_from_slice(CURSOR_HIDE);
            self.cursor_visible = false;
        }
    }

    fn painted_differs(&self, frame: &Frame, wanted: u16) -> bool {
        let rows = visible_rows(&frame.live, wanted);
        rows.iter()
            .enumerate()
            .any(|(i, row)| self.painted.get(i).and_then(|p| p.as_deref()) != Some(row.as_str()))
    }

    fn cursor_needs_move(&self, frame: &Frame, wanted: u16) -> bool {
        match (frame.cursor, self.cursor_at) {
            (None, _) => false,
            (Some((row, col)), at) => at != Some((col, self.top_for(wanted) + row)),
        }
    }

    fn commit(&mut self, text: &str, out: &mut Vec<u8>) {
        if self.commit_in_place(text, out) {
            return;
        }
        let mut term = OutTerm {
            out,
            size: self.size,
            viewport: Rect::new(0, self.viewport_top(), self.size.0, self.viewport_height),
            reset: false,
        };
        // A commit's closing newline terminates its last row; the emitter
        // would draw it as one more, blank, row.
        let text = text.strip_suffix('\n').unwrap_or(text);
        let text = &scrub_keeping_newlines(text);
        // The writer is a Vec, so writes cannot fail.
        let written = text.split('\n').count() as u16;
        let before = self.top;
        let _ = emit_to_scrollback(&mut term, text);
        // The emitter moved the region below what it wrote.
        let moved = term.viewport.y;
        self.top = moved;
        self.note_scroll((before + written).saturating_sub(moved));
        if term.reset {
            self.painted.fill(None);
        }
        // The vendored routine leaves the cursor at the new viewport top.
        self.cursor_at = Some((0, self.top));
    }

    /// Commits rows by turning the live region's top rows into history:
    /// they are written over the rows painted there (a row that already
    /// reads the same is left alone) and the region now starts below them.
    /// Rows beyond the region are written on below it, scrolling the screen a
    /// row at a time once they reach the bottom. A streamed answer commits
    /// the rows its live tail showed, so mostly nothing is written at all;
    /// nothing is ever erased ahead of being drawn, and the next paint fills
    /// the region from its new top. Rows wider than the screen (the terminal
    /// would wrap them) go the erasing way.
    fn commit_in_place(&mut self, text: &str, out: &mut Vec<u8>) -> bool {
        let text = text.strip_suffix('\n').unwrap_or(text);
        let text = scrub_keeping_newlines(text);
        let rows: Vec<&str> = text.split('\n').collect();
        let width = self.size.0 as usize;
        if rows
            .iter()
            .any(|row| crate::ui::render::panel::visible_width(row) > width)
        {
            return false;
        }
        let bottom = i64::from(self.size.1.saturating_sub(1));
        // Where the region's first row is now. A commit taller than the
        // screen scrolls its own first rows off the top, so this goes below
        // zero while it is written.
        let mut top = i64::from(self.top);
        for (i, row) in rows.iter().enumerate() {
            let painted = self.painted.get(i).and_then(|p| p.as_deref()).map(scrub);
            if painted.as_deref() == Some(*row) {
                continue;
            }
            let mut at = top + i as i64;
            if at > bottom {
                // Past the bottom: scroll one row, the written rows move up.
                push_move(out, 0, bottom as u16);
                out.push(b'\n');
                top -= 1;
                self.note_scroll(1);
                at = bottom;
            }
            push_move(out, 0, at as u16);
            out.extend_from_slice(row.as_bytes());
            out.extend_from_slice(b"\x1b[0m");
            out.extend_from_slice(ERASE_LINE_END);
            self.cursor_at = None;
        }
        let kept = (self.viewport_height as usize).saturating_sub(rows.len());
        // The region now starts below the last row written.
        self.top = (top + rows.len() as i64).clamp(0, i64::from(self.size.1)) as u16;
        self.viewport_height = kept as u16;
        self.painted.drain(..self.painted.len().min(rows.len()));
        true
    }

    /// `rows` rows of the screen left through the top.
    fn note_scroll(&mut self, rows: u16) {
        if rows == 0 {
            return;
        }
        if rows > self.content_start {
            self.scrolled = true;
        }
        self.content_start = self.content_start.saturating_sub(rows);
    }

    fn set_height(&mut self, wanted: u16, out: &mut Vec<u8>) {
        let (_, rows) = self.size;
        let old = self.viewport_height;
        if wanted > old {
            // Scroll the screen up when the region would run past the bottom;
            // the old live rows move up with it and are repainted.
            let overflow = (self.top + wanted).saturating_sub(rows);
            if overflow > 0 {
                push_move(out, 0, rows.saturating_sub(1));
                out.extend(std::iter::repeat_n(b"\n", overflow as usize).flatten());
                self.top -= overflow;
                self.note_scroll(overflow);
            }
            // The painted rows scrolled with the screen and still read the
            // same; only the rows the region gained are unknown.
            self.painted.resize(wanted as usize, None);
            self.viewport_height = wanted;
            self.cursor_at = None;
            return;
        }
        // The region keeps its bottom row, so the composer never moves up:
        // the rows it gives back are its top ones, left blank above it.
        let gone = old - wanted;
        for i in 0..gone {
            push_move(out, 0, self.top + i);
            out.extend_from_slice(ERASE_LINE);
        }
        self.top += gone;
        self.viewport_height = wanted;
        // The rows kept scrolled nowhere; they still read the same.
        self.painted.drain(..self.painted.len().min(gone as usize));
        self.painted.resize(wanted as usize, None);
        self.cursor_at = None;
    }

    fn resize_to(&mut self, size: (u16, u16), wanted: u16, out: &mut Vec<u8>) {
        // Stage 2.8 replaces this with the reflow contract; until then keep
        // the live region valid at the new geometry.
        self.size = size;
        self.viewport_height = wanted;
        self.top = size.1.saturating_sub(wanted);
        self.painted = vec![None; wanted as usize];
        let top = self.viewport_top();
        push_move(out, 0, top);
        out.extend_from_slice(ERASE_DOWN);
        self.cursor_at = Some((0, top));
    }

    /// Repaints after a resize by rewriting the terminal's whole history.
    ///
    /// Erasing cannot survive a resize: reflowing terminals (iTerm2, Ghostty,
    /// Apple Terminal) split every row wider than the new width before the
    /// app hears about it, pushing stale pieces of the old frame into native
    /// scrollback where no escape sequence reaches them. So the screen and the
    /// scrollback are cleared (`2J`, then `3J`; `2J` first so terminals that
    /// archive the screen on it archive into what `3J` clears) and `tail`,
    /// laid out at the new width, is written from the top of a clean screen.
    /// Output from before this interface started is lost.
    pub fn rewrite(&mut self, size: (u16, u16), tail: &[String], wanted: u16) -> Vec<u8> {
        let mut out = Vec::new();
        let rows = size.1.max(1);
        let wanted = wanted.clamp(1, rows);
        self.size = size;
        self.begin(&mut out);
        if self.cursor_visible {
            out.extend_from_slice(CURSOR_HIDE);
            self.cursor_visible = false;
        }
        out.extend_from_slice(b"\x1b[2J\x1b[H\x1b[3J");
        for row in tail {
            out.extend_from_slice(scrub(row).as_bytes());
            out.extend_from_slice(b"\x1b[0m\r\n");
        }
        // The cursor is on the row after the tail; the region starts there,
        // scrolling the screen up for whatever does not fit below it.
        let after = tail.len().min((rows - 1) as usize) as u16;
        let overflow = (after + wanted).saturating_sub(rows);
        if overflow > 0 {
            push_move(&mut out, 0, rows - 1);
            out.extend(std::iter::repeat_n(b"\n", overflow as usize).flatten());
        }
        let top = after - overflow;
        self.scrolled = tail.len() > top as usize;
        self.content_start = 0;
        self.top = top;
        self.viewport_height = wanted;
        self.painted = vec![None; wanted as usize];
        self.cursor_at = None;
        self.anchor = top;
        push_move(&mut out, 0, top);
        self.end(&mut out);
        out
    }

    /// Hands the screen back: the cursor goes to the start of the row below
    /// the live region (scrolling when the region ends on the last row) and
    /// shows, so whatever is written next lands under the composer.
    pub fn leave(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        let last = (self.top + self.viewport_height.max(1) - 1).min(self.size.1.saturating_sub(1));
        push_move(&mut out, 0, last);
        out.extend_from_slice(b"\r\n");
        out.extend_from_slice(CURSOR_SHOW);
        self.cursor_visible = true;
        self.cursor_at = None;
        out
    }

    /// Hands the screen over to the fullscreen interface: the live region
    /// is erased, so what stays behind is the committed history, and the
    /// cursor waits where the region started.
    pub fn erase_live(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        push_move(&mut out, 0, self.viewport_top());
        out.extend_from_slice(ERASE_DOWN);
        out.extend_from_slice(CURSOR_SHOW);
        self.cursor_visible = true;
        self.cursor_at = None;
        out
    }

    fn paint(&mut self, frame: &Frame, out: &mut Vec<u8>) {
        let top = self.viewport_top();
        let rows = visible_rows(&frame.live, self.viewport_height);
        for (i, row) in rows.iter().enumerate() {
            if self.painted.get(i).and_then(|p| p.as_deref()) == Some(row.as_str()) {
                continue;
            }
            push_move(out, 0, top + i as u16);
            out.extend_from_slice(scrub(row).as_bytes());
            // Styling must not leak into the next row; the old text past the
            // new is erased after the row is written, never before.
            out.extend_from_slice(b"\x1b[0m");
            out.extend_from_slice(ERASE_LINE_END);
            if let Some(slot) = self.painted.get_mut(i) {
                *slot = Some(row.clone());
            }
            self.cursor_at = None;
        }
    }

    fn place_cursor(&mut self, frame: &Frame, out: &mut Vec<u8>) {
        match frame.cursor {
            Some((row, col)) => {
                let target = (col, self.viewport_top() + row);
                self.anchor = target.1;
                if self.cursor_at != Some(target) {
                    push_move(out, target.0, target.1);
                    self.cursor_at = Some(target);
                }
                if !self.cursor_visible {
                    out.extend_from_slice(CURSOR_SHOW);
                    self.cursor_visible = true;
                }
            }
            None => {
                // Parked on the region's first row, so the resize report has
                // a row to be compared against.
                let target = (0, self.viewport_top());
                self.anchor = target.1;
                if self.cursor_at != Some(target) {
                    push_move(out, target.0, target.1);
                    self.cursor_at = Some(target);
                }
                if self.cursor_visible {
                    out.extend_from_slice(CURSOR_HIDE);
                    self.cursor_visible = false;
                }
            }
        }
    }
}

/// The rows that fit: when the live region is taller than the viewport the
/// bottom rows win (the composer lives there).
fn visible_rows(live: &[String], height: u16) -> Vec<String> {
    let height = height.max(1) as usize;
    let skip = live.len().saturating_sub(height);
    let mut rows: Vec<String> = live[skip..].to_vec();
    while rows.len() < height {
        rows.push(String::new());
    }
    rows
}

/// A character that may need scrubbing: a C0 or C1 control (ESC among
/// them) or DEL. Model and tool text reaches rows verbatim: a lone `\r`
/// sends the rest of a row back to column 0 over its own border, and an
/// escape sequence can retitle the window, clear the screen, write the
/// clipboard or plant a link.
fn is_control(c: char) -> bool {
    matches!(c, '\x00'..='\x1f' | '\x7f' | '\u{80}'..='\u{9f}')
}

/// One row as it may reach the terminal: only the escapes the interface
/// itself writes survive, SGR (`ESC [ … m`) and an OSC 8 link to an
/// `http(s)` or `mailto` target (or the link's end); every other control
/// and sequence is removed.
pub(crate) fn scrub(row: &str) -> std::borrow::Cow<'_, str> {
    if row.contains(is_control) {
        scrub_slow(row, false).into()
    } else {
        row.into()
    }
}

/// A commit: rows separated by `\n`, each scrubbed.
fn scrub_keeping_newlines(text: &str) -> std::borrow::Cow<'_, str> {
    if text.contains(|c| c != '\n' && is_control(c)) {
        scrub_slow(text, true).into()
    } else {
        text.into()
    }
}

fn scrub_slow(text: &str, keep_newlines: bool) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(is_control) {
        out.push_str(&rest[..at]);
        let control = rest[at..].chars().next().unwrap_or('\x1b');
        rest = &rest[at + control.len_utf8()..];
        match control {
            '\n' if keep_newlines => out.push('\n'),
            '\x1b' => {
                let (kept, after) = escape_sequence(rest);
                if !kept.is_empty() {
                    out.push('\x1b');
                    out.push_str(kept);
                }
                rest = after;
            }
            // C1 CSI and OSC introducers: the same sequences as after ESC,
            // never ones the interface writes.
            '\u{9b}' => rest = csi(rest).1,
            '\u{9d}' => rest = osc(rest).1,
            _ => {}
        }
    }
    out.push_str(rest);
    out
}

/// The sequence after an ESC at the start of `rest`: the text to keep (the
/// whole sequence with its ESC, or nothing) and what follows it.
fn escape_sequence(rest: &str) -> (&str, &str) {
    let Some(kind) = rest.chars().next() else {
        return ("", rest);
    };
    match kind {
        '[' => {
            let (sequence, after) = csi(&rest[1..]);
            let kept = if sequence.ends_with('m') {
                &rest[..1 + sequence.len()]
            } else {
                ""
            };
            // The kept slice must carry its ESC: it is rebuilt by the caller.
            (kept, after)
        }
        ']' => {
            let (body, after) = osc(&rest[1..]);
            let link = body
                .strip_prefix("8;")
                .and_then(|params| params.split_once(';'))
                .map(|(_, target)| target);
            let kept = match link {
                Some(target)
                    if target.is_empty() || crate::ui::render::markdown::linkable(target) =>
                {
                    &rest[..1 + body.len() + (rest.len() - 1 - body.len() - after.len())]
                }
                _ => "",
            };
            (kept, after)
        }
        // DCS, APC, PM: ended by ST like an OSC; two-character escapes: just
        // the one character.
        'P' | '_' | '^' => ("", osc(&rest[1..]).1),
        _ => ("", &rest[kind.len_utf8()..]),
    }
}

/// A CSI body up to and including its final byte (`@`..`~`).
fn csi(rest: &str) -> (&str, &str) {
    let end = rest
        .char_indices()
        .find(|(_, c)| ('@'..='~').contains(c))
        .map_or(rest.len(), |(i, c)| i + c.len_utf8());
    (&rest[..end], &rest[end..])
}

/// An OSC body (without its terminator) and what follows the BEL or ST
/// that ends it.
fn osc(rest: &str) -> (&str, &str) {
    let mut end = rest.len();
    let mut skip = 0;
    if let Some(bel) = rest.find('\x07') {
        end = bel;
        skip = 1;
    }
    if let Some(st) = rest.find("\x1b\\").filter(|st| *st < end) {
        end = st;
        skip = 2;
    }
    if let Some(st) = rest.find('\u{9c}').filter(|st| *st < end) {
        end = st;
        skip = '\u{9c}'.len_utf8();
    }
    (&rest[..end], &rest[end + skip..])
}

/// CUP with 0-based (column, row).
fn push_move(out: &mut Vec<u8>, column: u16, row: u16) {
    let _ = write!(out, "\x1b[{};{}H", row + 1, column + 1);
}

/// The vendored scrollback emitter writes through this view of the screen.
struct OutTerm<'a> {
    out: &'a mut Vec<u8>,
    size: (u16, u16),
    viewport: Rect,
    reset: bool,
}

impl TerminalLike for OutTerm<'_> {
    type Writer = Vec<u8>;

    fn size(&self) -> io::Result<Size> {
        Ok(Size::new(self.size.0, self.size.1))
    }

    fn viewport_area(&self) -> Rect {
        self.viewport
    }

    fn clear(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn reset_back_buffer(&mut self) {
        self.reset = true;
    }

    fn set_viewport_area(&mut self, area: Rect) {
        self.viewport = area;
    }

    fn writer_mut(&mut self) -> &mut Vec<u8> {
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::render::testterm::Emu;

    const SIZE: (u16, u16) = (40, 12);

    fn frame(commits: &[&str], live: &[&str], cursor: Option<(u16, u16)>) -> Frame {
        Frame {
            commits: commits.iter().map(|s| s.to_string()).collect(),
            live: live.iter().map(|s| s.to_string()).collect(),
            cursor,
            dock: 0,
            overlay: None,
        }
    }

    fn drive(screen: &mut Screen, emu: &mut Emu, f: &Frame) -> Vec<u8> {
        let bytes = screen.render(f, SIZE);
        emu.feed(&bytes);
        bytes
    }

    fn setup(sync: bool) -> (Screen, Emu) {
        (
            Screen::new(SIZE, sync),
            Emu::new(SIZE.0 as usize, SIZE.1 as usize),
        )
    }

    #[test]
    fn live_region_is_pinned_to_the_bottom_rows() {
        let (mut screen, mut emu) = setup(true);
        drive(
            &mut screen,
            &mut emu,
            &frame(&[], &["box top", "> hi", "status"], Some((1, 4))),
        );
        let rows = emu.rows_text();
        assert_eq!(&rows[9..], ["box top", "> hi", "status"]);
        assert!(rows[..9].iter().all(|r| r.is_empty()));
        assert_eq!((emu.cx, emu.cy), (4, 10));
    }

    #[test]
    fn commits_land_once_in_order_above_the_live_region() {
        let (mut screen, mut emu) = setup(true);
        let live = ["composer", "status"];
        drive(&mut screen, &mut emu, &frame(&[], &live, None));
        for n in 1..=15 {
            let line = format!("message {n}");
            drive(&mut screen, &mut emu, &frame(&[&line], &live, None));
        }
        let transcript = emu.transcript();
        let messages: Vec<&String> = transcript
            .iter()
            .filter(|l| l.starts_with("message"))
            .collect();
        assert_eq!(
            messages.len(),
            15,
            "each commit appears exactly once: {transcript:?}"
        );
        let expected: Vec<String> = (1..=15).map(|n| format!("message {n}")).collect();
        assert_eq!(messages.into_iter().cloned().collect::<Vec<_>>(), expected);
        let rows = emu.rows_text();
        assert_eq!(&rows[10..], ["composer", "status"], "live region intact");
    }

    #[test]
    fn a_commit_taller_than_the_screen_scrolls_through_without_overflow() {
        // A long thought committed at once from a region at the top of the
        // screen: its first rows scroll off before the last are written.
        let (mut screen, mut emu) = setup(true);
        screen.set_origin(0);
        let live = ["composer", "status"];
        drive(&mut screen, &mut emu, &frame(&[], &live, None));
        let tall: Vec<String> = (1..=40).map(|n| format!("thought {n}")).collect();
        let commit = tall.join("\n");
        drive(&mut screen, &mut emu, &frame(&[&commit], &live, None));
        let transcript = emu.transcript();
        let written: Vec<&String> = transcript
            .iter()
            .filter(|l| l.starts_with("thought"))
            .collect();
        assert_eq!(written.len(), 40, "{transcript:?}");
        assert_eq!(written.last().map(|s| s.as_str()), Some("thought 40"));
        let rows = emu.rows_text();
        assert_eq!(&rows[10..], ["composer", "status"], "live region intact");
    }

    #[test]
    fn newline_terminated_commits_do_not_double_the_gap() {
        let (mut screen, mut emu) = setup(true);
        let live = ["composer", "status"];
        drive(&mut screen, &mut emu, &frame(&[], &live, None));
        drive(&mut screen, &mut emu, &frame(&["\none\n"], &live, None));
        drive(&mut screen, &mut emu, &frame(&["\ntwo\n"], &live, None));
        let rows = emu.rows_text();
        assert_eq!(
            rows[6..],
            ["", "one", "", "two", "composer", "status"],
            "{rows:?}"
        );
    }

    #[test]
    fn region_starts_at_the_shell_cursor_row() {
        let (mut screen, mut emu) = setup(true);
        emu.feed(b"$ dim\r\n");
        screen.set_origin(1);
        drive(
            &mut screen,
            &mut emu,
            &frame(&["\nbanner\n"], &["composer", "status"], None),
        );
        let rows = emu.rows_text();
        assert_eq!(rows[0], "$ dim");
        assert_eq!(rows[1..5], ["", "banner", "composer", "status"], "{rows:?}");
    }

    #[test]
    fn rewrite_clears_scrollback_and_writes_the_history_once() {
        let (mut screen, mut emu) = setup(true);
        emu.feed(b"before-start\r\n");
        screen.set_origin(1);
        let live = ["composer", "status"];
        drive(
            &mut screen,
            &mut emu,
            &frame(&["\nfirst\n", "\nsecond\n"], &live, Some((0, 0))),
        );
        let tail: Vec<String> = ["", "first", "", "second"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let wanted = screen.wanted_for(SIZE, live.len());
        let bytes = screen.rewrite(SIZE, &tail, wanted);
        emu.feed(&bytes);
        drive(&mut screen, &mut emu, &frame(&[], &live, Some((0, 0))));
        let rows = emu.rows_text();
        assert_eq!(
            rows[..6],
            ["", "first", "", "second", "composer", "status"],
            "{rows:?}"
        );
        let transcript = emu.transcript();
        assert!(
            !transcript.iter().any(|l| l == "before-start"),
            "{transcript:?}"
        );
        assert_eq!(transcript.iter().filter(|l| *l == "first").count(), 1);
    }

    /// Tool and shell output reaches rows verbatim: only the escapes the
    /// interface writes itself (SGR, an http(s)/mailto OSC 8 link) survive.
    #[test]
    fn only_the_interfaces_own_escapes_reach_the_terminal() {
        let row = concat!(
            "\x1b[31mred\x1b[0m ",
            "\x1b]0;title\x07",
            "\x1b[2J\x1b[H",
            "\x1b]52;c;aGk=\x1b\\",
            "\x1b]8;;file:///etc/passwd\x07bad\x1b]8;;\x07 ",
            "\x1b]8;;https://a.example/\x07ok\x1b]8;;\x07",
            "\u{9b}2K\u{9d}0;t\u{9c}\x1bc end"
        );
        assert_eq!(
            scrub(row),
            "\x1b[31mred\x1b[0m bad\x1b]8;;\x07 \x1b]8;;https://a.example/\x07ok\x1b]8;;\x07 end"
        );
        assert_eq!(scrub_keeping_newlines("a\x1b[2Jb\nc\r"), "ab\nc");
        assert!(matches!(scrub("plain text"), std::borrow::Cow::Borrowed(_)));
    }

    #[test]
    fn row_text_never_moves_the_cursor() {
        let (mut screen, mut emu) = setup(true);
        drive(
            &mut screen,
            &mut emu,
            &frame(
                &["分位数（推荐\r）\n"],
                &["│ 1 ( ) 分位数（推荐\r） │"],
                None,
            ),
        );
        // The emulator keeps a `\0` spacer after each wide character.
        let transcript: Vec<String> = emu
            .transcript()
            .iter()
            .map(|l| l.replace('\0', ""))
            .collect();
        assert!(
            transcript.iter().any(|l| l == "分位数（推荐）"),
            "{transcript:?}"
        );
        assert!(
            transcript.iter().any(|l| l == "│ 1 ( ) 分位数（推荐） │"),
            "{transcript:?}"
        );
    }

    #[test]
    fn leaving_puts_the_cursor_under_the_live_region() {
        let (mut screen, mut emu) = setup(true);
        drive(
            &mut screen,
            &mut emu,
            &frame(&["\nfirst\n"], &["composer", "status"], Some((0, 0))),
        );
        emu.feed(&screen.leave());
        emu.feed(b"$ ");
        let rows = emu.rows_text();
        let status = rows.iter().position(|r| r == "status").unwrap();
        assert_eq!(rows[status + 1], "$");
    }

    #[test]
    fn unchanged_frame_writes_nothing_and_keeps_the_cursor_still() {
        let (mut screen, mut emu) = setup(true);
        let f = frame(&[], &["a", "b"], Some((0, 1)));
        drive(&mut screen, &mut emu, &f);
        let again = drive(&mut screen, &mut emu, &f);
        assert!(
            again.is_empty(),
            "no bytes for an identical frame: {again:?}"
        );
    }

    #[test]
    fn only_changed_rows_are_rewritten() {
        let (mut screen, mut emu) = setup(true);
        drive(
            &mut screen,
            &mut emu,
            &frame(&[], &["top", "middle", "bottom"], Some((1, 0))),
        );
        let bytes = drive(
            &mut screen,
            &mut emu,
            &frame(&[], &["top", "MIDDLE", "bottom"], Some((1, 0))),
        );
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("MIDDLE"));
        assert!(
            !text.contains("top") && !text.contains("bottom"),
            "{text:?}"
        );
        assert_eq!(emu.rows_text()[10], "MIDDLE");
    }

    #[test]
    fn cursor_moves_without_repainting_rows() {
        let (mut screen, mut emu) = setup(true);
        drive(
            &mut screen,
            &mut emu,
            &frame(&[], &["abcdef"], Some((0, 1))),
        );
        let bytes = drive(
            &mut screen,
            &mut emu,
            &frame(&[], &["abcdef"], Some((0, 4))),
        );
        assert_eq!((emu.cx, emu.cy), (4, 11));
        assert!(!String::from_utf8_lossy(&bytes).contains("abcdef"));
    }

    #[test]
    fn cursor_visibility_toggles_only_when_it_changes() {
        let (mut screen, mut emu) = setup(true);
        drive(&mut screen, &mut emu, &frame(&[], &["x"], None));
        assert!(!emu.cursor_visible);
        let before = emu.visibility_changes;
        drive(&mut screen, &mut emu, &frame(&[], &["xy"], None));
        assert_eq!(emu.visibility_changes, before, "stayed hidden, no toggles");
        drive(&mut screen, &mut emu, &frame(&[], &["xy"], Some((0, 2))));
        assert!(emu.cursor_visible);
    }

    #[test]
    fn growing_and_shrinking_the_live_region_keeps_history_intact() {
        let (mut screen, mut emu) = setup(true);
        drive(
            &mut screen,
            &mut emu,
            &frame(&["history line"], &["c", "s"], None),
        );
        drive(
            &mut screen,
            &mut emu,
            &frame(&[], &["stream 1", "stream 2", "c", "s"], None),
        );
        let rows = emu.rows_text();
        assert_eq!(&rows[8..], ["stream 1", "stream 2", "c", "s"]);
        assert!(emu.transcript().contains(&"history line".to_owned()));
        drive(&mut screen, &mut emu, &frame(&[], &["c", "s"], None));
        let rows = emu.rows_text();
        // The region keeps its bottom row and gives its top rows back,
        // blank, above it: the composer never moves up.
        assert_eq!(&rows[10..], ["c", "s"]);
        assert!(rows[8..10].iter().all(String::is_empty), "{rows:?}");
        assert!(
            rows.iter().all(|r| !r.contains("stream")),
            "stale rows cleared: {rows:?}"
        );
        assert_eq!(
            emu.transcript()
                .iter()
                .filter(|l| *l == "history line")
                .count(),
            1
        );
    }

    #[test]
    fn long_commit_wraps_softly_and_appears_once() {
        let (mut screen, mut emu) = setup(true);
        let live = ["c", "s"];
        drive(&mut screen, &mut emu, &frame(&[], &live, None));
        let long = "x".repeat(95);
        drive(&mut screen, &mut emu, &frame(&[&long], &live, None));
        let joined: String = emu
            .transcript()
            .iter()
            .filter(|l| l.starts_with('x'))
            .cloned()
            .collect();
        assert_eq!(joined.len(), 95);
    }

    #[test]
    fn synchronized_blocks_balance_and_are_skipped_when_disabled() {
        let (mut screen, mut emu) = setup(true);
        drive(&mut screen, &mut emu, &frame(&["a"], &["c"], Some((0, 0))));
        assert_eq!(emu.sync_depth, 0);
        assert!(emu.sync_blocks >= 1);

        let (mut screen, mut emu) = setup(false);
        drive(&mut screen, &mut emu, &frame(&["a"], &["c"], Some((0, 0))));
        assert_eq!(emu.sync_blocks, 0);
        assert!(emu.cursor_visible, "cursor shown again after the paint");
    }

    #[test]
    fn oversized_live_region_keeps_the_bottom_rows_and_some_history() {
        let (mut screen, mut emu) = setup(true);
        let live: Vec<String> = (0..20).map(|n| format!("row {n}")).collect();
        let refs: Vec<&str> = live.iter().map(String::as_str).collect();
        drive(&mut screen, &mut emu, &frame(&[], &refs, None));
        assert_eq!(screen.viewport_height(), SIZE.1 - MIN_HISTORY_ROWS);
        assert_eq!(emu.row(11), "row 19");
    }

    #[test]
    fn hyperlinks_survive_wrapping_and_commit_once() {
        let (mut screen, mut emu) = setup(true);
        let live = ["c", "s"];
        drive(&mut screen, &mut emu, &frame(&[], &live, None));
        let link = crate::ui::render::osc8::hyperlink("https://example.com/a", "docs link");
        let text = format!("see {link} and then {}", "word ".repeat(12));
        let bytes = drive(&mut screen, &mut emu, &frame(&[&text], &live, None));
        let sent = String::from_utf8_lossy(&bytes).into_owned();
        assert!(sent.contains("\x1b]8;;https://example.com/a\x1b\\docs link\x1b]8;;\x1b\\"));
        let joined = emu.transcript().join("\n");
        assert_eq!(joined.matches("docs link").count(), 1);
    }

    /// A small deterministic generator, so a failing case can be replayed
    /// from its seed.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            if n == 0 { 0 } else { self.next() % n }
        }
        fn rows(&mut self, most: u64, width: u16) -> Vec<String> {
            (0..self.below(most + 1))
                .map(|n| {
                    // Some rows are wider than the screen, some blank.
                    let len = self.below(u64::from(width) * 2 + 2) as usize;
                    format!("{n}{}", "x".repeat(len))
                })
                .collect()
        }
    }

    #[test]
    fn random_frames_at_any_size_never_panic() {
        // DIM_STRESS_SEEDS widens the sweep for a deeper local run.
        let seeds: u64 = std::env::var("DIM_STRESS_SEEDS")
            .ok()
            .and_then(|n| n.parse().ok())
            .unwrap_or(300);
        for seed in 1..=seeds {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let mut size = (1 + rng.below(90) as u16, 1 + rng.below(40) as u16);
            let mut screen = Screen::new(size, rng.below(2) == 0);
            if rng.below(2) == 0 {
                screen.set_origin(rng.below(u64::from(size.1) + 3) as u16);
            }
            for _ in 0..40 {
                if rng.below(8) == 0 {
                    size = (1 + rng.below(90) as u16, 1 + rng.below(40) as u16);
                }
                match rng.below(10) {
                    0 => {
                        let tail = rng.rows(60, size.0);
                        let wanted = rng.below(u64::from(size.1) + 4) as u16;
                        screen.rewrite(size, &tail, wanted);
                    }
                    1 => {
                        screen.erase_live();
                    }
                    _ => {
                        let commits = (0..rng.below(3))
                            .map(|_| rng.rows(3 * u64::from(size.1) + 5, size.0).join("\n"))
                            .collect();
                        let live = rng.rows(u64::from(size.1) + 5, size.0);
                        let cursor = (rng.below(3) > 0).then(|| {
                            (
                                rng.below(live.len() as u64 + 2) as u16,
                                rng.below(u64::from(size.0) + 2) as u16,
                            )
                        });
                        let frame = Frame {
                            commits,
                            live,
                            cursor,
                            ..Frame::default()
                        };
                        screen.render(&frame, size);
                    }
                }
            }
            screen.leave();
        }
    }
}
