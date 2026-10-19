//! Cell-accurate text measurement shared by every drawing layer: display
//! width, truncation, tab expansion and wrapping by display columns. It
//! depends on nothing else in the interface.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

pub fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

/// Cuts `text` to `max_width` cells with a trailing ellipsis when it does
/// not fit (grapheme-aware, the budget includes the ellipsis).
pub fn truncate_cells(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if display_width(text) <= max_width {
        return text.to_owned();
    }
    if max_width == 1 {
        return "…".to_owned();
    }
    let budget = max_width - 1;
    let mut out = String::new();
    let mut used = 0;
    for cluster in text.graphemes(true) {
        let cell = display_width(cluster);
        if used + cell > budget {
            break;
        }
        out.push_str(cluster);
        used += cell;
    }
    out.push('…');
    out
}

/// Columns between tab stops, CommonMark's.
const TAB_STOP: usize = 4;

/// `text` with each tab turned into the spaces up to the next tab stop of
/// its line. A tab never reaches the terminal (it would move the cursor
/// over cells the row does not paint, and rows are scrubbed of it), so
/// indentation made of tabs (Go, Makefiles) is kept this way.
pub fn expand_tabs(text: &str) -> std::borrow::Cow<'_, str> {
    if !text.contains('\t') {
        return text.into();
    }
    let mut out = String::with_capacity(text.len() + 8);
    let mut column = 0;
    for cluster in text.graphemes(true) {
        match cluster {
            "\t" => {
                let spaces = TAB_STOP - column % TAB_STOP;
                out.extend(std::iter::repeat_n(' ', spaces));
                column += spaces;
            }
            "\n" | "\r\n" | "\r" => {
                out.push_str(cluster);
                column = 0;
            }
            _ => {
                out.push_str(cluster);
                column += display_width(cluster);
            }
        }
    }
    out.into()
}

/// Wraps `text` into rows of at most `width` cells, breaking only between
/// grapheme clusters. A cluster wider than the row stays alone on its row.
pub fn wrap_cells(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    let normalized = expand_tabs(text).replace("\r\n", "\n").replace('\r', "\n");
    let mut rows = Vec::new();
    for line in normalized.split('\n') {
        if line.is_empty() {
            rows.push(String::new());
            continue;
        }
        let mut row = String::new();
        let mut row_width = 0;
        for cluster in line.graphemes(true) {
            let cell = display_width(cluster);
            if !row.is_empty() && row_width + cell > width {
                rows.push(std::mem::take(&mut row));
                row_width = 0;
            }
            row.push_str(cluster);
            row_width += cell;
        }
        rows.push(row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_expand_to_the_next_stop_of_their_line() {
        assert_eq!(expand_tabs("\tx"), "    x");
        assert_eq!(expand_tabs("ab\tc\n\td"), "ab  c\n    d");
        // Wide characters count their cells.
        assert_eq!(expand_tabs("中\tx"), "中  x");
        assert!(matches!(
            expand_tabs("plain"),
            std::borrow::Cow::Borrowed(_)
        ));
    }

    #[test]
    fn wrap_breaks_on_cells_not_chars() {
        assert_eq!(wrap_cells("中文中文", 4), vec!["中文", "中文"]);
        assert_eq!(wrap_cells("ab\n\ncd", 5), vec!["ab", "", "cd"]);
        // A wide cluster larger than the row stays whole.
        assert_eq!(wrap_cells("中", 1), vec!["中"]);
        // ZWJ emoji stays one cluster.
        let family = "👨\u{200d}👩\u{200d}👧";
        assert_eq!(wrap_cells(&format!("a{family}"), 2), vec!["a", family]);
    }

    #[test]
    fn measuring_at_any_width_never_panics() {
        let samples = ["", "a", "中文中文", "👨\u{200d}👩\u{200d}👧x", "\ttab\tbed", "e\u{301}", "a\nb\n\n", "\u{200b}"];
        for text in samples {
            for width in 0..6 {
                let _ = wrap_cells(text, width);
                let _ = truncate_cells(text, width);
            }
            let _ = expand_tabs(text);
            let _ = display_width(text);
        }
    }
}
