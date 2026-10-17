use crate::{
    Result,
    client::{Client, RequestTooLarge, RpcError},
    display,
    input::Input,
};
use dim_agent_types::{ContentBlock, UserInput};
use serde::Deserialize;
use serde_json::json;
use std::io::{self, Write};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

const INLINE_PASTE: usize = 16 * 1024;
const MAX_PASTE: usize = 10 * 1024 * 1024;
const PASTE_START: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";

enum Part {
    Text(String),
    Paste {
        text: String,
        lines: usize,
        path: Option<String>,
    },
}
impl Part {
    fn len(&self) -> usize {
        match self {
            Self::Text(text) => text.len(),
            Self::Paste { .. } => 1,
        }
    }
}

#[derive(Default)]
pub struct Draft {
    parts: Vec<Part>,
    cursor: usize,
}

impl Draft {
    pub fn clear(&mut self) {
        self.parts.clear();
        self.cursor = 0;
    }

    pub fn plain(&self) -> Option<&str> {
        match self.parts.as_slice() {
            [Part::Text(text)] if !text.contains(['\r', '\n']) => Some(text),
            _ => None,
        }
    }

    pub fn command(&self) -> Option<&str> {
        let Part::Text(text) = self.parts.first()? else {
            return None;
        };
        match text.split_whitespace().next()? {
            command @ ("/exit" | "/export" | "/mcp" | "/effort" | "/reasoning" | "/fast"
            | "/agent") => Some(command),
            _ if text.starts_with('!') => Some("!"),
            _ => None,
        }
    }

    fn empty(&self) -> bool {
        self.parts.iter().all(|part| match part {
            Part::Text(text) => text.trim().is_empty(),
            Part::Paste { .. } => false,
        })
    }

    fn locate(&self, position: usize) -> (usize, usize) {
        let mut start = 0;
        for (index, part) in self.parts.iter().enumerate() {
            if position <= start + part.len() {
                return (index, position - start);
            }
            start += part.len();
        }
        (self.parts.len(), 0)
    }

    fn insert(&mut self, part: Part) {
        let inserted = part.len();
        let (index, offset) = self.locate(self.cursor);
        match self.parts.get_mut(index) {
            Some(Part::Text(text)) => {
                let tail = text.split_off(offset);
                self.parts
                    .splice(index + 1..index + 1, [part, Part::Text(tail)]);
            }
            Some(Part::Paste { .. }) => self.parts.insert(index + offset, part),
            None => self.parts.push(part),
        }
        self.cursor += inserted;
        self.join_text();
        // Inserting a combining character can join both adjacent graphemes.
        self.snap_cursor();
    }

    fn snap_cursor(&mut self) {
        let (index, offset) = self.locate(self.cursor);
        if let Some(Part::Text(text)) = self.parts.get(index)
            && offset > 0
            && offset < text.len()
        {
            let next = text
                .grapheme_indices(true)
                .map(|(pos, _)| pos)
                .find(|&pos| pos >= offset)
                .unwrap_or(text.len());
            self.cursor += next - offset;
        }
    }

    fn join_text(&mut self) {
        self.parts
            .retain(|part| !matches!(part, Part::Text(text) if text.is_empty()));
        let mut index = 1;
        while index < self.parts.len() {
            if matches!(
                (&self.parts[index - 1], &self.parts[index]),
                (Part::Text(_), Part::Text(_))
            ) {
                let Part::Text(tail) = self.parts.remove(index) else {
                    unreachable!()
                };
                let Part::Text(text) = &mut self.parts[index - 1] else {
                    unreachable!()
                };
                text.push_str(&tail);
            } else {
                index += 1;
            }
        }
    }

    fn boundaries(&self) -> Vec<usize> {
        let mut positions = vec![0];
        let mut start = 0;
        for part in &self.parts {
            match part {
                Part::Text(text) => positions.extend(
                    text.grapheme_indices(true)
                        .map(|(offset, grapheme)| start + offset + grapheme.len()),
                ),
                Part::Paste { .. } => positions.push(start + 1),
            }
            start += part.len();
        }
        positions
    }

    fn erase(&mut self, start: usize, end: usize) {
        let mut base = 0;
        for part in &mut self.parts {
            let length = part.len();
            match part {
                Part::Text(text) => {
                    let from = start.saturating_sub(base).min(length);
                    let to = end.saturating_sub(base).min(length);
                    text.replace_range(from..to, "");
                }
                Part::Paste { .. } if start <= base && end > base => {
                    *part = Part::Text(String::new());
                }
                Part::Paste { .. } => {}
            }
            base += length;
        }
        self.cursor = start;
        self.join_text();
        self.snap_cursor();
    }

    fn horizontal(&mut self, forward: bool, erase: bool) {
        let positions = self.boundaries();
        let next = if forward {
            positions.into_iter().find(|&pos| pos > self.cursor)
        } else {
            positions.into_iter().rfind(|&pos| pos < self.cursor)
        };
        if let Some(next) = next {
            if erase {
                self.erase(self.cursor.min(next), self.cursor.max(next));
            } else {
                self.cursor = next;
            }
        }
    }

    fn layout(&self, width: usize) -> (Vec<String>, Vec<(usize, usize, usize)>) {
        let columns = width.saturating_sub(1).max(3);
        let mut lines = vec!["> ".to_owned()];
        let mut positions = vec![(0, 0, 2)];
        let (mut offset, mut column, mut pastes) = (0, 2, 0);
        for part in &self.parts {
            let pieces: Vec<(String, usize)> = match part {
                Part::Text(text) => text
                    .graphemes(true)
                    .map(|g| (g.to_owned(), g.len()))
                    .collect(),
                Part::Paste { lines, .. } => {
                    pastes += 1;
                    vec![(format!("[Paste {pastes}: {lines} lines]"), 1)]
                }
            };
            for (piece, length) in pieces {
                if matches!(piece.as_str(), "\n" | "\r\n") {
                    lines.push("  ".into());
                    column = 2;
                } else {
                    let mut shown = String::new();
                    for ch in piece.chars() {
                        if ch.is_control()
                            || matches!(ch, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
                        {
                            shown.extend(ch.escape_debug());
                        } else {
                            shown.push(ch);
                        }
                    }
                    if matches!(part, Part::Paste { .. }) {
                        shown = truncate(&shown, columns.saturating_sub(2));
                    }
                    let cells = shown.width();
                    if column + cells > columns && column > 2 {
                        lines.push("  ".into());
                        column = 2;
                        *positions.last_mut().expect("cursor boundary") =
                            (offset, lines.len() - 1, column);
                    }
                    lines.last_mut().expect("input row").push_str(&shown);
                    column += cells;
                }
                offset += length;
                positions.push((offset, lines.len() - 1, column));
            }
        }
        (lines, positions)
    }

    fn vertical(&mut self, width: usize, direction: i32) {
        let (_, positions) = self.layout(width);
        let Some(&(_, row, column)) = positions.iter().rfind(|&&(pos, _, _)| pos == self.cursor)
        else {
            return;
        };
        let target = row as i32 + direction;
        if target >= 0
            && let Some(&(position, _, _)) = positions
                .iter()
                .filter(|&&(_, row, _)| row as i32 == target)
                .min_by_key(|&&(_, _, col)| col.abs_diff(column))
        {
            self.cursor = position;
        }
    }

    fn line_edge(&mut self, end: bool) {
        let (mut start, mut base) = (0, 0);
        for part in &self.parts {
            if let Part::Text(text) = part {
                for (offset, grapheme) in text.grapheme_indices(true) {
                    if matches!(grapheme, "\n" | "\r\n") {
                        if base + offset >= self.cursor {
                            self.cursor = if end { base + offset } else { start };
                            return;
                        }
                        start = base + offset + grapheme.len();
                    }
                }
            }
            base += part.len();
        }
        self.cursor = if end { base } else { start };
    }

    pub async fn prepare(
        &mut self,
        client: &mut Client,
        session: &str,
        #[cfg(unix)] interrupts: &mut tokio::signal::unix::Signal,
        #[cfg(windows)] interrupts: &mut tokio::signal::windows::CtrlC,
        notice: &mut Option<String>,
    ) -> Result<Option<UserInput>> {
        let mut pastes = 0;
        for part in &mut self.parts {
            let Part::Paste { text, path, .. } = part else {
                continue;
            };
            pastes += 1;
            if text.len() <= INLINE_PASTE || path.is_some() {
                continue;
            }
            tokio::select! {
                biased;
                _ = interrupts.recv() => {
                    *notice = Some("Preparation cancelled; draft retained".into());
                    return Ok(None);
                },
                _ = std::future::ready(()) => {}
            }
            println!("Saving text attachment...");
            let mut cancelled = false;
            let stored: Result<String> = async {
                let id = client
                    .send("attachment.put", json!({"sessionId":session,"text":text}))
                    .await?;
                let response = client.response(id);
                tokio::pin!(response);
                let reply = loop {
                    tokio::select! {
                        biased;
                        _ = interrupts.recv() => {
                            if cancelled {
                                println!("Still waiting for the text attachment to settle...");
                            } else {
                                cancelled = true;
                                println!("Waiting for the text attachment to settle...");
                            }
                        }
                        result = &mut response => break result?,
                    }
                };
                #[derive(Deserialize)]
                struct Stored {
                    path: String,
                }
                Ok(serde_json::from_value::<Stored>(reply)
                    .map_err(|_| "Attachment outcome could not be confirmed; resume the Session instead of retrying automatically")?.path)
            }.await;
            match stored {
                Ok(stored) => *path = Some(stored),
                Err(error) if error.is::<RpcError>() || error.is::<RequestTooLarge>() => {
                    *notice = Some(format!("Paste {pastes}: {error}; draft retained"));
                    return Ok(None);
                }
                Err(error) => return Err(error),
            };
            if cancelled {
                *notice = Some("Preparation cancelled; draft retained".into());
                return Ok(None);
            }
        }
        let text = self
            .parts
            .iter()
            .map(|part| match part {
                Part::Text(text) => text.clone(),
                Part::Paste { text, lines, path } => match path {
                    Some(path) => format!("[Pasted text: {path} ({lines} lines)]"),
                    None => text.clone(),
                },
            })
            .collect::<String>();
        tokio::select! {
            biased;
            _ = interrupts.recv() => {
                *notice = Some("Preparation cancelled; draft retained".into());
                return Ok(None);
            },
            _ = std::future::ready(()) => {}
        }
        Ok(Some(UserInput {
            content: vec![ContentBlock::Text { text }],
        }))
    }
}

enum Key {
    Text(String),
    Paste(String),
    Rejected(&'static str),
    Enter,
    Newline,
    Interrupt,
    Eof,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Backspace,
    Delete,
}

#[derive(Default)]
struct Paste {
    bytes: Vec<u8>,
    rejected: Option<&'static str>,
}
impl Paste {
    fn append(&mut self, bytes: &[u8]) {
        if self.bytes.len() + bytes.len() > MAX_PASTE {
            self.rejected = Some("Paste exceeds 10 MiB; draft retained");
        } else if self.rejected.is_none() {
            self.bytes.extend_from_slice(bytes);
        }
    }
}

#[derive(Default)]
struct Decoder {
    bytes: Vec<u8>,
    paste: Option<Paste>,
}
impl Decoder {
    fn next(&mut self) -> Option<Key> {
        loop {
            if let Some(paste) = &mut self.paste {
                let end = self
                    .bytes
                    .windows(PASTE_END.len())
                    .position(|s| s == PASTE_END);
                if let Some(cancel) = self.bytes.windows(2).position(|s| s == b"\x03\x03")
                    && end.is_none_or(|end| cancel < end)
                {
                    self.bytes.clear();
                    self.paste = None;
                    return Some(Key::Rejected("Paste cancelled; draft retained"));
                }
                if let Some(end) = end {
                    paste.append(&self.bytes[..end]);
                    self.bytes.drain(..end + PASTE_END.len());
                    let paste = self.paste.take().expect("paste end");
                    return Some(if let Some(reason) = paste.rejected {
                        Key::Rejected(reason)
                    } else {
                        match String::from_utf8(paste.bytes) {
                            Ok(text) => Key::Paste(text),
                            Err(_) => Key::Rejected("Paste must be UTF-8; draft retained"),
                        }
                    });
                }
                let safe = self.bytes.len().saturating_sub(PASTE_END.len() - 1);
                paste.append(&self.bytes[..safe]);
                self.bytes.drain(..safe);
                return None;
            }
            let &first = self.bytes.first()?;
            let key = match first {
                3 => Key::Interrupt,
                4 => Key::Eof,
                #[cfg(windows)]
                26 => Key::Eof,
                8 | 127 => Key::Backspace,
                10 => Key::Newline,
                13 => Key::Enter,
                27 => {
                    if self.bytes.starts_with(PASTE_START) {
                        self.bytes.drain(..PASTE_START.len());
                        self.paste = Some(Paste::default());
                        continue;
                    }
                    if PASTE_START.starts_with(&self.bytes) || self.bytes.len() == 1 {
                        return None;
                    }
                    if !matches!(self.bytes[1], b'[' | b'O') {
                        self.bytes.remove(0);
                        continue;
                    }
                    let end = self.bytes[2..]
                        .iter()
                        .position(|b| (0x40..=0x7e).contains(b))?
                        + 2;
                    let key = match &self.bytes[..=end] {
                        b"\x1b[A" | b"\x1bOA" => Some(Key::Up),
                        b"\x1b[B" | b"\x1bOB" => Some(Key::Down),
                        b"\x1b[C" | b"\x1bOC" => Some(Key::Right),
                        b"\x1b[D" | b"\x1bOD" => Some(Key::Left),
                        b"\x1b[H" | b"\x1bOH" | b"\x1b[1~" | b"\x1b[7~" => Some(Key::Home),
                        b"\x1b[F" | b"\x1bOF" | b"\x1b[4~" | b"\x1b[8~" => Some(Key::End),
                        b"\x1b[3~" => Some(Key::Delete),
                        b"\x1b[13;2u" | b"\x1b[27;2;13~" => Some(Key::Newline),
                        _ => None,
                    };
                    self.bytes.drain(..=end);
                    if let Some(key) = key {
                        return Some(key);
                    }
                    continue;
                }
                0..=31 => {
                    self.bytes.remove(0);
                    continue;
                }
                _ => {
                    let length = match first {
                        0..=0x7f => 1,
                        0xc2..=0xdf => 2,
                        0xe0..=0xef => 3,
                        0xf0..=0xf4 => 4,
                        _ => 1,
                    };
                    let length = length.min(self.bytes.len());
                    let value = std::str::from_utf8(&self.bytes[..length]);
                    let key = match value {
                        Ok(text) => Key::Text(text.into()),
                        Err(error) => {
                            let length = error.error_len()?;
                            self.bytes.drain(..length);
                            return Some(Key::Rejected(
                                "Terminal input must be UTF-8; draft retained",
                            ));
                        }
                    };
                    self.bytes.drain(..length);
                    return Some(key);
                }
            };
            self.bytes.remove(0);
            return Some(key);
        }
    }
}

fn truncate(text: &str, columns: usize) -> String {
    if text.width() <= columns {
        return text.into();
    }
    let mut shown = String::new();
    for grapheme in text.graphemes(true) {
        if shown.width() + grapheme.width() + 1 > columns {
            break;
        }
        shown.push_str(grapheme);
    }
    if columns > 0 {
        shown.push('…');
    }
    shown
}

#[derive(Default)]
struct Screen {
    widths: Vec<usize>,
    cursor: (usize, usize),
    columns: usize,
}
impl Screen {
    fn physical(&self, columns: usize) -> (usize, usize) {
        let columns = columns.max(1);
        let mut rows = 0;
        let mut cursor = 0;
        for (index, &width) in self.widths.iter().enumerate() {
            let cells = if index == self.cursor.0 {
                cursor = rows + self.cursor.1 / columns;
                width.max(self.cursor.1 + 1)
            } else {
                width
            };
            rows += cells.div_ceil(columns).max(1);
        }
        (rows, cursor)
    }

    fn draw(&mut self, draft: &Draft, size: (usize, usize), notice: Option<&str>) -> Result<()> {
        let (mut lines, positions) = draft.layout(size.0);
        let &(_, row, column) = positions
            .iter()
            .rfind(|&&(pos, _, _)| pos == draft.cursor)
            .expect("draft cursor");
        let top = row.saturating_sub(size.1.saturating_sub(2));
        let end = (top + size.1.saturating_sub(1).max(1)).min(lines.len());
        lines = lines[top..end].to_vec();
        lines.push(truncate(
            &display::safe(notice.unwrap_or(
                "Enter: send   Ctrl+J: newline   Ctrl+C: clear   Ctrl+C twice: cancel paste",
            )),
            size.0.saturating_sub(1),
        ));
        let mut output = io::stdout().lock();
        let (previous_rows, previous_cursor) = self.physical(size.0);
        if previous_rows == 0 {
            writeln!(output)?;
        } else {
            write!(output, "\r")?;
            if previous_cursor > 0 {
                write!(output, "\x1b[{}A", previous_cursor)?;
            }
            for index in 0..previous_rows {
                write!(output, "\x1b[2K")?;
                if index + 1 < previous_rows {
                    write!(output, "\x1b[1B\r")?;
                }
            }
            if previous_rows > 1 {
                write!(output, "\x1b[{}A", previous_rows - 1)?;
            }
            write!(output, "\r")?;
        }
        for (index, text) in lines.iter().enumerate() {
            if index > 0 {
                write!(output, "\r\n")?;
            }
            write!(output, "{text}")?;
        }
        self.widths = lines.iter().map(|line| line.width()).collect();
        self.cursor = (row - top, column);
        self.columns = size.0;
        write!(output, "\r\x1b[{}A", lines.len() - self.cursor.0 - 1)?;
        if column > 0 {
            write!(output, "\x1b[{column}C")?;
        }
        output.flush()?;
        Ok(())
    }
}
impl Drop for Screen {
    fn drop(&mut self) {
        let (rows, cursor) = self.physical(self.columns);
        if rows > 0 {
            let mut output = io::stdout().lock();
            let _ = write!(output, "\r\x1b[{}B\x1b[2K\n", rows - cursor - 1);
            let _ = output.flush();
        }
    }
}

pub async fn edit(
    input: &mut Input,
    draft: &mut Draft,
    mut notice: Option<String>,
    #[cfg(unix)] interrupts: &mut tokio::signal::unix::Signal,
    #[cfg(windows)] interrupts: &mut tokio::signal::windows::CtrlC,
) -> Result<bool> {
    let _mode = input.editor()?;
    let mut screen = Screen::default();
    let mut decoder = Decoder::default();
    #[cfg(unix)]
    let mut resize = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::window_change())?;
    let mut size = input.size()?;
    screen.draw(draft, size, notice.as_deref())?;
    loop {
        let bytes = tokio::select! {
            _ = interrupts.recv() => {
                if draft.parts.is_empty() && decoder.paste.is_none() {
                    return Ok(false);
                }
                draft.clear();
                decoder = Decoder::default();
                notice = None;
                screen.draw(draft, size, None)?;
                continue;
            },
            _ = async {
                #[cfg(unix)]
                { resize.recv().await; }
                #[cfg(windows)]
                { std::future::pending::<()>().await; }
            } => {
                size = input.size()?;
                screen.draw(draft, size, notice.as_deref())?;
                continue;
            },
            bytes = input.read_bytes() => bytes?,
        };
        let Some(bytes) = bytes else { return Ok(false) };
        let mut changed = bytes.is_empty();
        decoder.bytes.extend(bytes);
        while let Some(key) = decoder.next() {
            changed = true;
            notice = None;
            match key {
                Key::Enter if !draft.empty() => {
                    screen.draw(draft, size, None)?;
                    return Ok(true);
                }
                Key::Enter => {}
                Key::Eof if draft.parts.is_empty() => return Ok(false),
                Key::Eof => notice = Some("Draft retained; clear it before exiting".into()),
                Key::Interrupt if draft.parts.is_empty() => return Ok(false),
                Key::Interrupt => {
                    draft.clear();
                }
                Key::Text(text) => draft.insert(Part::Text(text)),
                Key::Newline => draft.insert(Part::Text("\n".into())),
                Key::Paste(text) if text.is_empty() => {}
                Key::Paste(text) if text.len() > INLINE_PASTE || text.contains(['\r', '\n']) => {
                    let lines = text.lines().count();
                    draft.insert(Part::Paste {
                        text,
                        lines,
                        path: None,
                    });
                }
                Key::Paste(text) => draft.insert(Part::Text(text)),
                Key::Rejected(reason) => notice = Some(reason.into()),
                Key::Left => draft.horizontal(false, false),
                Key::Right => draft.horizontal(true, false),
                Key::Backspace => draft.horizontal(false, true),
                Key::Delete => draft.horizontal(true, true),
                Key::Up => draft.vertical(size.0, -1),
                Key::Down => draft.vertical(size.0, 1),
                Key::Home => draft.line_edge(false),
                Key::End => draft.line_edge(true),
            }
        }
        if changed {
            size = input.size()?;
            screen.draw(draft, size, notice.as_deref())?;
        }
    }
}
