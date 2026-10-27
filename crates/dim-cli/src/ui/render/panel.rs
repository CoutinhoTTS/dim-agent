//! Bordered panels shared by the palette, the queue strip and the prompts
//! (ports of `renderBorderedBlock`, `panelChoices` and `ConfirmPanel`).

use crate::ui::text::{display_width, truncate_cells};
use crate::ui::theme::Palette;
use crate::ui::theme::ansi::{Paint, fg, paint};

pub const SELECTED_MARKER: &str = "› ";
pub const IDLE_MARKER: &str = "  ";

/// The first row of a list window of `page` rows over `len` rows that
/// shows `selected`: the window stays where it was (`start`) and moves only
/// as far as the selection needs, so moving up inside it moves the
/// highlight rather than the list.
pub fn follow_window(start: usize, selected: usize, page: usize, len: usize) -> usize {
    let page = page.max(1);
    let start = start.min(len.saturating_sub(page));
    if selected < start {
        selected
    } else if selected >= start + page {
        selected + 1 - page
    } else {
        start
    }
}

/// Display width ignoring SGR/OSC sequences.
pub fn visible_width(text: &str) -> usize {
    display_width(&strip_escapes(text))
}

/// The text of a row without its SGR/OSC sequences.
pub fn strip_escapes(text: &str) -> String {
    let mut plain = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            match chars.peek() {
                Some('[') => {
                    chars.next();
                    for n in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&n) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    while let Some(n) = chars.next() {
                        if n == '\x07' {
                            break;
                        }
                        if n == '\x1b' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        plain.push(c);
    }
    plain
}

/// Greedy word wrap (port of `wrapWordsCells`): words longer than the row are
/// cut by display width.
pub fn wrap_words(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    let mut result = Vec::new();
    let mut current = String::new();
    for word in text.split(' ') {
        let candidate = if current.is_empty() {
            word.to_owned()
        } else {
            format!("{current} {word}")
        };
        if clip(&candidate, usize::MAX).1 <= width {
            current = candidate;
            continue;
        }
        if !current.is_empty() {
            result.push(std::mem::take(&mut current));
        }
        let mut chunks = wrap(word, width);
        current = chunks.pop().unwrap_or_default();
        result.extend(chunks);
    }
    result.push(current);
    result
}

/// `text` broken into rows of at most `width` columns, by grapheme.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    use unicode_segmentation::UnicodeSegmentation;
    let mut rows = vec![String::new()];
    let mut used = 0;
    for cluster in text.graphemes(true) {
        let w = crate::ui::text::display_width(cluster);
        if used + w > width && used > 0 {
            rows.push(String::new());
            used = 0;
        }
        if let Some(row) = rows.last_mut() {
            row.push_str(cluster);
        }
        used += w;
    }
    rows
}

/// The longest prefix of `text` that fits `max` columns (escapes kept and
/// free), and the width it takes.
pub fn clip(text: &str, max: usize) -> (String, usize) {
    use unicode_segmentation::UnicodeSegmentation;
    let mut out = String::new();
    let mut width = 0;
    let mut rest = text;
    'outer: while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('\x1b') {
            // Escape sequences are free: copy CSI and OSC whole.
            let len = if let Some(body) = after.strip_prefix('[') {
                let end = body
                    .char_indices()
                    .find(|(_, n)| ('\u{40}'..='\u{7e}').contains(n))
                    .map_or(body.len(), |(i, n)| i + n.len_utf8());
                2 + end
            } else if let Some(body) = after.strip_prefix(']') {
                let mut end = body.len();
                let mut indices = body.char_indices().peekable();
                while let Some((i, n)) = indices.next() {
                    if n == '\x07' {
                        end = i + 1;
                        break;
                    }
                    if n == '\x1b' && matches!(indices.peek(), Some((_, '\\'))) {
                        end = i + 2;
                        break;
                    }
                }
                2 + end
            } else {
                1
            };
            let len = len.min(rest.len());
            out.push_str(&rest[..len]);
            rest = &rest[len..];
            continue;
        }
        let run_end = rest.find('\x1b').unwrap_or(rest.len());
        for cluster in rest[..run_end].graphemes(true) {
            let w = crate::ui::text::display_width(cluster);
            if width + w > max {
                break 'outer;
            }
            width += w;
            out.push_str(cluster);
        }
        rest = &rest[run_end..];
    }
    (out, width)
}

/// Borders in `color`, one column of padding, rows padded with blanks to
/// `height` when given. `title` sits in the top border (`╭─ Queue (1) ──╮`)
/// and `hint` at the right of the bottom one (`╰── Esc close ─╯`).
pub fn bordered(
    content: &[String],
    width: usize,
    height: Option<usize>,
    color: &str,
    title: Option<&str>,
    hint: Option<&str>,
) -> Vec<String> {
    let inner = width.saturating_sub(2);
    let content_width = inner.saturating_sub(2);
    let edge = |s: &str| fg(s, color);
    // The title keeps its own styling (accent and bold).
    let top = match title {
        Some(title) if inner > visible_width(title) + 3 => {
            let fill = inner - visible_width(title) - 3;
            format!(
                "{}{title}{}",
                edge("╭─ "),
                edge(&format!(" {}╮", "─".repeat(fill)))
            )
        }
        _ => edge(&format!("╭{}╮", "─".repeat(inner))),
    };
    let mut out = vec![top];
    let body_rows = height.map(|h| h.saturating_sub(2));
    for line in content.iter().take(body_rows.unwrap_or(usize::MAX)) {
        let used = visible_width(line);
        out.push(format!(
            "{} {line}{} {}",
            edge("│"),
            " ".repeat(content_width.saturating_sub(used)),
            edge("│")
        ));
    }
    if let Some(rows) = body_rows {
        while out.len() < rows + 1 {
            out.push(format!("{}{}{}", edge("│"), " ".repeat(inner), edge("│")));
        }
    }
    out.push(match hint {
        Some(hint) if inner > visible_width(hint) + 3 => {
            let fill = inner - visible_width(hint) - 3;
            format!(
                "{}{hint}{}",
                edge(&format!("╰{} ", "─".repeat(fill))),
                edge(" ─╯")
            )
        }
        _ => edge(&format!("╰{}╯", "─".repeat(inner))),
    });
    out
}

/// A `key action · key action` hint as painted in a bottom border:
/// each key (the first word of a part) in the keybinding colour, the rest
/// muted.
pub fn key_hints(hint: &str, p: &Palette) -> String {
    hint.split(" · ")
        .map(|part| match part.split_once(' ') {
            Some((key, action)) => format!(
                "{}{}",
                fg(key, &p.keybind),
                fg(&format!(" {action}"), &p.muted)
            ),
            None => fg(part, &p.keybind),
        })
        .collect::<Vec<_>>()
        .join(&fg(" · ", &p.muted))
}

/// Horizontal choice row: the focused option carries the accent marker.
pub fn choices(labels: &[String], focused: usize, p: &Palette) -> String {
    labels
        .iter()
        .enumerate()
        .map(|(index, label)| {
            if index == focused {
                paint(
                    &format!("{SELECTED_MARKER}{label}"),
                    Paint {
                        fg: Some(&p.accent),
                        bold: true,
                        ..Paint::default()
                    },
                )
            } else {
                fg(&format!("{IDLE_MARKER}{label}"), &p.text_soft)
            }
        })
        .collect::<Vec<_>>()
        .join("   ")
}

/// A warning-titled confirmation: title, optional body, choices, hint.
pub fn confirm_panel(
    title: &str,
    body: &[String],
    options: &[String],
    focused: usize,
    hint: &str,
    width: usize,
    p: &Palette,
) -> Vec<String> {
    let inner = width.saturating_sub(4).max(1);
    let mut content = vec![paint(
        &truncate_cells(title, inner),
        Paint {
            fg: Some(&p.warning),
            bold: true,
            ..Paint::default()
        },
    )];
    content.push(String::new());
    for line in body {
        content.push(fg(&truncate_cells(line, inner), &p.text));
    }
    if !body.is_empty() {
        content.push(String::new());
    }
    content.push(choices(options, focused, p));
    content.push(String::new());
    content.push(fg(&truncate_cells(hint, inner), &p.muted));
    bordered(&content, width, None, &p.border, None, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::{Appearance, ColorMode, Theme};

    fn plain(s: &str) -> String {
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
    }

    #[test]
    fn titled_border_and_fixed_height() {
        let rows = bordered(
            &["a".into()],
            30,
            Some(5),
            "#555555",
            Some("Queue (1)"),
            None,
        );
        let text: Vec<String> = rows.iter().map(|r| plain(r)).collect();
        assert_eq!(text.len(), 5);
        assert!(text[0].starts_with("╭─ Queue (1) ─"));
        assert_eq!(display_width(&text[0]), 30);
        assert!(text.iter().all(|r| display_width(r) == 30));
    }

    #[test]
    fn confirm_panel_layout() {
        let theme = Theme::system(ColorMode::TrueColor, Appearance::Dark, None);
        let panel = confirm_panel(
            "Exit dimcode?",
            &["The agent is still running.".into()],
            &["Exit".into(), "Stay".into()],
            1,
            "y exit · n stay",
            60,
            &theme.palette,
        );
        let text: Vec<String> = panel.iter().map(|r| plain(r)).collect();
        assert!(text[1].contains("Exit dimcode?"));
        assert!(
            text.iter()
                .any(|r| r.contains("  Exit") && r.contains("› Stay"))
        );
    }
}
