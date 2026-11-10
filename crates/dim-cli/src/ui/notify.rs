//! Desktop notifications for a run that needs the user back (port of the TS
//! `terminal-notification.ts`): it finished, or it waits on a permission or a
//! question. The interface owns nothing outside the terminal, so it asks the
//! terminal to notify, in the one form each understands: iTerm2 `OSC 9`,
//! kitty `OSC 99`, the rxvt `OSC 777` notify of Ghostty, WezTerm and foot.
//! Anything else gets a plain `BEL`, which every terminal turns into some
//! signal.
//!
//! Inside tmux or GNU screen the chosen form is wrapped for passthrough
//! (`OSC 52` and the rest of what dim writes there): a multiplexer keeps a
//! notification it does not know to itself, and forwarding it upward is what
//! lets a finished run say what finished. A notification asserts nothing
//! after it is delivered, so the record's reason for staying inside a
//! multiplexer (`ui::status`, `ui::progress`) does not apply. herdr forwards
//! `OSC` itself and its `HERDR_ENV` is not a multiplexer for this purpose, so
//! it keeps taking the form its terminal names.
//!
//! On by default; `/settings` switches it (`tuiNotifyOnComplete`).

use crate::ui::i18n::{t, tf};

/// `config.json` key of the switch; absent means on.
pub const NOTIFY_KEY: &str = "tuiNotifyOnComplete";

/// Why the run wants the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Complete,
    /// A permission request for the named tool.
    Permission(String),
    Question,
}

/// The multiplexer a notification is written inside, if any.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Multiplexer {
    #[default]
    None,
    Tmux,
    Screen,
}

impl Multiplexer {
    pub fn from_env() -> Self {
        Self::of(|name| std::env::var(name).ok())
    }

    /// The multiplexer these variables describe: tmux by `TMUX` or a `tmux`
    /// `TERM`, GNU screen by `STY` or a `screen` `TERM`. herdr is not one: it
    /// forwards the `OSC` to the terminal it runs in, so there the terminal's
    /// own form reaches the user unchanged, and `HERDR_ENV` names no wrapping.
    fn of(var: impl Fn(&str) -> Option<String>) -> Self {
        let var = |name: &str| var(name).unwrap_or_default();
        if !var("TMUX").is_empty() {
            return Self::Tmux;
        }
        if !var("STY").is_empty() {
            return Self::Screen;
        }
        let term = var("TERM");
        if term.starts_with("tmux") {
            return Self::Tmux;
        }
        if term.starts_with("screen") {
            return Self::Screen;
        }
        Self::None
    }
}

/// The terminal as notifications see it.
#[derive(Debug, Default)]
pub struct Terminal {
    pub term_program: String,
    pub term: String,
    pub kitty: bool,
    pub multiplexer: Multiplexer,
}

impl Terminal {
    pub fn from_env() -> Self {
        let var = |name| std::env::var(name).unwrap_or_default();
        Self {
            term_program: var("TERM_PROGRAM"),
            term: var("TERM"),
            kitty: !var("KITTY_WINDOW_ID").is_empty(),
            multiplexer: Multiplexer::from_env(),
        }
    }
}

/// The bytes that make `terminal` show `event`; never empty.
pub fn sequence(terminal: &Terminal, event: &Event) -> Vec<u8> {
    let body = match event {
        Event::Complete => t("Task complete").to_owned(),
        Event::Permission(tool) => tf("Permission needed: {tool}", &[("tool", tool)]),
        Event::Question => t("Question waiting for your answer").to_owned(),
    };
    let clean = |text: &str| -> String {
        text.chars()
            .filter(|c| !c.is_control())
            .map(|c| if c == ';' { ',' } else { c })
            .collect()
    };
    let (title, body) = ("DIM", clean(&body));
    let program = terminal.term_program.as_str();
    let term = terminal.term.as_str();
    let form = if program == "iTerm.app" {
        Some(format!("\x1b]9;{title}: {body}\x07"))
    } else if term == "xterm-kitty" || terminal.kitty {
        Some(format!("\x1b]99;;{title}: {body}\x1b\\"))
    } else if matches!(program, "ghostty" | "WezTerm")
        || term == "xterm-ghostty"
        || term.starts_with("foot")
        || term.starts_with("rxvt-unicode")
    {
        Some(format!("\x1b]777;notify;{title};{body}\x07"))
    } else {
        None
    };
    match form {
        Some(text) => passthrough(terminal.multiplexer, text.into_bytes()),
        None => b"\x07".to_vec(),
    }
}

/// `sequence` wrapped for `multiplexer`: past the multiplexer's own header,
/// every `ESC` of the sequence is doubled so the multiplexer reads the inner
/// sequence as one string instead of acting on it. `BEL` needs no escaping
/// for either multiplexer, so a notification body never ends the string
/// early.
fn passthrough(multiplexer: Multiplexer, sequence: Vec<u8>) -> Vec<u8> {
    let header: &[u8] = match multiplexer {
        Multiplexer::None => return sequence,
        Multiplexer::Tmux => b"\x1bPtmux;",
        Multiplexer::Screen => b"\x1bP",
    };
    let mut escaped = Vec::with_capacity(sequence.len() * 2);
    for byte in sequence {
        if byte == 0x1b {
            escaped.push(0x1b);
        }
        escaped.push(byte);
    }
    let mut wrapped = Vec::with_capacity(header.len() + escaped.len() + 2);
    wrapped.extend_from_slice(header);
    wrapped.extend_from_slice(&escaped);
    wrapped.extend_from_slice(b"\x1b\\");
    wrapped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on(term_program: &str, term: &str) -> Terminal {
        Terminal {
            term_program: term_program.into(),
            term: term.into(),
            ..Terminal::default()
        }
    }

    fn in_mux(multiplexer: Multiplexer, term_program: &str, term: &str) -> Terminal {
        Terminal {
            multiplexer,
            ..on(term_program, term)
        }
    }

    #[test]
    fn each_terminal_gets_the_form_it_understands() {
        let done = Event::Complete;
        assert_eq!(
            sequence(&on("iTerm.app", "xterm-256color"), &done),
            b"\x1b]9;DIM: Task complete\x07"
        );
        assert_eq!(
            sequence(&on("", "xterm-kitty"), &done),
            b"\x1b]99;;DIM: Task complete\x1b\\"
        );
        assert_eq!(
            sequence(
                &on("ghostty", "xterm-ghostty"),
                &Event::Permission("bash;rm".into())
            ),
            "\x1b]777;notify;DIM;Permission needed: bash,rm\x07".as_bytes()
        );
        assert_eq!(
            sequence(
                &on("ghostty", "xterm-ghostty"),
                &Event::Permission("ba\u{7}sh\u{1b}".into())
            ),
            "\x1b]777;notify;DIM;Permission needed: bash\x07".as_bytes()
        );
        assert_eq!(sequence(&on("Apple_Terminal", "xterm"), &done), b"\x07");
    }

    #[test]
    fn a_multiplexer_forwards_the_form_the_terminal_understands() {
        let question = Event::Question;
        assert_eq!(
            sequence(
                &in_mux(Multiplexer::Tmux, "iTerm.app", "tmux-256color"),
                &question
            ),
            b"\x1bPtmux;\x1b\x1b]9;DIM: Question waiting for your answer\x07\x1b\\"
        );
        assert_eq!(
            sequence(
                &in_mux(Multiplexer::Tmux, "ghostty", "tmux-256color"),
                &question
            ),
            "\x1bPtmux;\x1b\x1b]777;notify;DIM;Question waiting for your answer\x07\x1b\\"
                .as_bytes()
        );
        assert_eq!(
            sequence(
                &in_mux(Multiplexer::Screen, "iTerm.app", "screen-256color"),
                &question
            ),
            b"\x1bP\x1b\x1b]9;DIM: Question waiting for your answer\x07\x1b\\"
        );
        assert_eq!(
            sequence(&in_mux(Multiplexer::Tmux, "", "xterm-kitty"), &question),
            "\x1bPtmux;\x1b\x1b]99;;DIM: Question waiting for your answer\x1b\x1b\\\x1b\\"
                .as_bytes()
        );
        assert_eq!(
            sequence(
                &in_mux(Multiplexer::Tmux, "Apple_Terminal", "tmux-256color"),
                &question
            ),
            b"\x07"
        );
    }

    #[test]
    fn herdr_is_not_a_multiplexer_for_notifications() {
        let done = Event::Complete;
        assert_eq!(
            sequence(&on("ghostty", "xterm-ghostty"), &done),
            "\x1b]777;notify;DIM;Task complete\x07".as_bytes()
        );
        assert_eq!(sequence(&on("", "xterm-256color"), &done), b"\x07");
    }

    #[test]
    fn the_multiplexer_is_named_by_its_own_variable() {
        let of = |pairs: &[(&str, &str)]| {
            Multiplexer::of(|name| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| (*value).to_owned())
            })
        };
        assert_eq!(
            of(&[("TMUX", "/tmp/tmux-501/default,1,0")]),
            Multiplexer::Tmux
        );
        assert_eq!(of(&[("STY", "1.pts-0.host")]), Multiplexer::Screen);
        assert_eq!(of(&[("TERM", "screen-256color")]), Multiplexer::Screen);
        assert_eq!(
            of(&[("TMUX", "x"), ("STY", "y"), ("TERM", "screen")]),
            Multiplexer::Tmux
        );
        assert_eq!(
            of(&[("HERDR_ENV", "1"), ("TERM", "xterm-256color")]),
            Multiplexer::None
        );
        assert_eq!(of(&[("TERM", "tmux-256color")]), Multiplexer::Tmux);
        assert_eq!(of(&[]), Multiplexer::None);
    }
}
