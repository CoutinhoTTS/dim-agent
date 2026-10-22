//! A shell-mode command (`!` / `!!`) while the App Server runs it: the
//! output its progress has shown, terminal escapes removed, only the newest
//! kept, for the live view under the transcript.

use std::collections::VecDeque;
use std::time::Instant;

use crate::model::shell::{Outcome, ShellResult, ShellRun};

/// Output kept for the live view: the newest lines within both bounds (the
/// `exec` tool's preview bounds).
const KEEP_LINES: usize = 2000;
const KEEP_BYTES: usize = 50 * 1024;
/// A line longer than this is split.
const MAX_LINE_CHARS: usize = 8 * 1024;

pub struct Running {
    pub run_id: String,
    pub command: String,
    pub private: bool,
    pub started: Instant,
    /// A stop was asked for; the run ends when the App Server says so.
    pub stopping: bool,
    capture: Capture,
}

impl Running {
    pub fn new(command: String, private: bool) -> Self {
        Self {
            run_id: uuid::Uuid::new_v4().to_string(),
            command,
            private,
            started: Instant::now(),
            stopping: false,
            capture: Capture::default(),
        }
    }

    /// Output the run produced since the last; `skipped` bytes were left
    /// out before it.
    pub fn progress(&mut self, text: &str, skipped: u64) {
        if skipped > 0 {
            self.capture.flush();
            self.capture.push(&format!("… {skipped} bytes skipped"));
        }
        self.capture.pending.push_str(text);
        while let Some(at) = self.capture.pending.find('\n') {
            let line: String = self.capture.pending.drain(..=at).collect();
            self.capture.push(&line[..at]);
        }
        if self.capture.pending.len() > MAX_LINE_CHARS {
            self.capture.flush();
        }
    }

    /// The newest `count` lines, the unfinished one included.
    pub fn tail(&self, count: usize) -> (Vec<String>, usize) {
        let capture = &self.capture;
        let mut lines: Vec<String> = capture.lines.iter().cloned().collect();
        let partial = clean(&capture.pending);
        if !partial.is_empty() {
            lines.push(partial);
        }
        let total = capture.total + usize::from(lines.len() > capture.lines.len());
        let skip = lines.len().saturating_sub(count);
        (lines.split_off(skip), total)
    }

    /// The finished run as its block: the reply's output, or what the
    /// progress showed when the reply carries no outcome.
    pub fn finish(mut self, result: &Result<ShellResult, String>) -> ShellRun {
        self.capture.flush();
        match result {
            Ok(result) => result.run(self.command, self.private, self.capture.total),
            Err(reason) => ShellRun {
                command: self.command,
                outcome: Outcome::Failed(reason.clone()),
                output: self.capture.lines.into_iter().collect(),
                total_lines: self.capture.total,
                private: self.private,
            },
        }
    }
}

#[derive(Default)]
struct Capture {
    lines: VecDeque<String>,
    bytes: usize,
    total: usize,
    /// Blank lines held back until something follows them: trailing blanks
    /// are dropped.
    blanks: usize,
    /// The unfinished line, for the live view.
    pending: String,
}

impl Capture {
    fn push(&mut self, raw: &str) {
        let line = clean(raw);
        if line.is_empty() {
            self.blanks += 1;
            return;
        }
        for _ in 0..std::mem::take(&mut self.blanks) {
            self.keep(String::new());
        }
        self.keep(line);
    }

    fn keep(&mut self, line: String) {
        self.total += 1;
        self.bytes += line.len() + 1;
        self.lines.push_back(line);
        while self.lines.len() > KEEP_LINES || self.bytes > KEEP_BYTES {
            let Some(dropped) = self.lines.pop_front() else {
                break;
            };
            self.bytes -= dropped.len() + 1;
        }
    }

    /// Ends the unfinished line.
    fn flush(&mut self) {
        let line = std::mem::take(&mut self.pending);
        if !line.is_empty() {
            self.push(&line);
        }
    }
}

/// A line as a terminal would leave it: escapes removed, only what follows
/// the last carriage return, tabs expanded, other controls dropped, no
/// trailing blanks.
fn clean(raw: &str) -> String {
    let stripped = strip_escapes(raw);
    let visible = stripped
        .split('\r')
        .rfind(|segment| !segment.is_empty())
        .unwrap_or("");
    let mut out = String::new();
    let mut column = 0;
    for c in visible.chars() {
        if c == '\t' {
            let spaces = 8 - column % 8;
            out.push_str(&" ".repeat(spaces));
            column += spaces;
        } else if !c.is_control() {
            out.push(c);
            column += 1;
        }
    }
    out.truncate(out.trim_end().len());
    out
}

/// Removes CSI, OSC and two-byte escape sequences.
fn strip_escapes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for c in chars.by_ref() {
                    if ('\x40'..='\x7e').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\x07' {
                        break;
                    }
                    if c == '\x1b' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_keeps_clean_lines_and_the_unfinished_one() {
        let mut running = Running::new("make".into(), false);
        running.progress("\x1b[31mred\x1b[0m\tx\nhal", 0);
        running.progress("f\r\nb\n\n\n", 0);
        assert_eq!(
            running.tail(10),
            (vec!["red     x".into(), "half".into(), "b".into()], 3)
        );
        running.progress("partial", 0);
        assert_eq!(running.tail(2), (vec!["b".into(), "partial".into()], 4));
        running.progress("", 9);
        assert_eq!(running.tail(1).0, ["… 9 bytes skipped"]);
    }

    #[test]
    fn only_the_newest_output_is_kept() {
        let mut running = Running::new("seq 1 5000".into(), true);
        for line in 1..=5000 {
            running.progress(&format!("{line}\n"), 0);
        }
        let (lines, total) = running.tail(usize::MAX);
        assert_eq!((lines.len(), total), (KEEP_LINES, 5000));
        assert_eq!(lines.last().map(String::as_str), Some("5000"));
    }

    #[test]
    fn a_run_without_a_reply_keeps_what_it_showed() {
        let mut running = Running::new("sleep 9".into(), false);
        running.progress("one\ntwo", 0);
        let run = running.finish(&Err("closed".into()));
        assert_eq!(run.outcome, Outcome::Failed("closed".into()));
        assert_eq!((run.output.len(), run.total_lines), (2, 2));
    }
}
