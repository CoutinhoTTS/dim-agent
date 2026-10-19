//! Block-level layout shared by the commit path and the live region: the
//! gutter, cell-accurate wrapping and the row forms of user and assistant
//! text. Markdown styling (2.5) plugs in through [`render_streamed`]'s
//! renderer argument; until then answers lay out as wrapped plain text.

use crate::ui::text::{display_width, wrap_cells};

pub const ASSISTANT_MARK: &str = "\u{25c6}"; // ◆
/// The bar in front of every row of a user message, as Amp draws it.
pub const USER_MARK: &str = "\u{2503}"; // ┃

/// Gutter columns for a terminal of `width` (narrow terminals get one).
pub fn gutter_width(width: usize) -> usize {
    if width < 40 { 1 } else { 2 }
}

/// The mark cell(s) of a gutter.
pub fn gutter_mark(mark: &str, gutter: usize) -> String {
    let first: String = mark.chars().take(1).collect();
    if gutter <= 1 {
        first
    } else {
        format!("{first} ")
    }
}

/// Plain stand-in for the Markdown renderer.
#[cfg(test)]
pub fn render_plain(source: &str, width: usize) -> Vec<String> {
    wrap_cells(source.trim_end_matches('\n'), width)
}

/// How a user message is painted: its colour (bar and text) and the
/// surface tint behind both (none in Amp's structure).
pub struct UserStyle<'a> {
    pub color: &'a str,
    pub surface: Option<&'a str>,
}

/// A user message: the bar on every row, the text in its colour. With a surface
/// tint every row is padded to the content width so it reads as a band.
pub fn render_user(text: &str, width: usize, style: &UserStyle<'_>) -> Vec<String> {
    use crate::ui::theme::ansi::{Paint, paint};
    let gutter = gutter_width(width);
    let content = width.saturating_sub(gutter).max(1);
    wrap_cells(text, content)
        .into_iter()
        .map(|row| {
            let lead = gutter_mark(USER_MARK, gutter);
            let body = match style.surface {
                Some(_) => {
                    let used = display_width(&row);
                    format!("{row}{}", " ".repeat(content.saturating_sub(used)))
                }
                None if row.is_empty() => " ".to_owned(),
                None => row,
            };
            format!(
                "{}{}",
                paint(
                    &lead,
                    Paint {
                        fg: Some(style.color),
                        bg: style.surface,
                        ..Paint::default()
                    }
                ),
                paint(
                    &body,
                    Paint {
                        fg: Some(style.color),
                        bg: style.surface,
                        ..Paint::default()
                    }
                ),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gutter_narrows_below_forty_columns() {
        assert_eq!(gutter_width(39), 1);
        assert_eq!(gutter_width(40), 2);
        assert_eq!(gutter_mark("◆", 1), "◆");
        assert_eq!(gutter_mark("◆", 2), "◆ ");
    }

    #[test]
    fn user_rows_carry_the_bar_on_every_row() {
        let style = UserStyle {
            color: "ansi:2",
            surface: None,
        };
        let strip = |s: &str| {
            let mut out = String::new();
            let mut skip = false;
            for c in s.chars() {
                match (skip, c) {
                    (false, '\x1b') => skip = true,
                    (true, 'm') => skip = false,
                    (false, c) => out.push(c),
                    _ => {}
                }
            }
            out
        };
        // Wide terminal: two-column gutter.
        let rows: Vec<String> = render_user(&"word ".repeat(10), 40, &style)
            .iter()
            .map(|r| strip(r))
            .collect();
        assert!(rows[0].starts_with("┃ word"));
        assert!(rows[1].starts_with("┃ "));
        // Narrow terminal: one-column gutter.
        let narrow: Vec<String> = render_user("one two three", 12, &style)
            .iter()
            .map(|r| strip(r))
            .collect();
        assert_eq!(narrow[0], "┃one two thr");
        assert_eq!(narrow[1], "┃ee");
        // A surface tint pads every row to the content width.
        let banded = UserStyle {
            surface: Some("#2a2a31"),
            ..style
        };
        let rows = render_user("hi", 40, &banded);
        assert_eq!(display_width(&strip(&rows[0])), 40);
    }
}
