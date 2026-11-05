//! The `/resume` picker: a filterable list of stored sessions (port of
//! `sessionsPanelItems` / `SessionsPanel`).

use crate::model::session::{Command, SessionItem, SessionsScope, stored_title};
use crate::ui::i18n::{t, tf};
use crate::ui::list_panel::{Event, Item, ListPanel};
use crate::ui::prompts::Effect;
use crate::ui::terminal::{KeyCode, KeyEvent, Modifiers};
use crate::ui::theme::Palette;
use crate::ui::tilde;
use std::time::{Duration, Instant};

const HINT: &str = "Enter open · Ctrl+S scope · Ctrl+R refresh · Esc close";
const TITLE_WORKSPACE: &str = "Sessions · This workspace";
const TITLE_REPOSITORY: &str = "Sessions · This repository";
const TITLE_ALL: &str = "Sessions · All workspaces";
/// Shown by the empty list of the exact launch workspace and of a one-entry
/// git set, which lists the same.
const EMPTY_WORKSPACE: &str = "No saved sessions in this workspace. Ctrl+S shows more.";
const EMPTY_OTHER: &str = "No saved sessions yet.";

pub struct SessionsPanel {
    list: ListPanel,
    request_id: Option<String>,
    paged: Option<bool>,
    due: Option<Instant>,
    append: bool,
    next_cursor: Option<String>,
    show_acp: bool,
    scope: SessionsScope,
    /// The launch workspace's git set, when git answered: the local scope
    /// adopts it, and the cycle returns to it from `All`. A failed git read
    /// leaves it `None`, and the local scope stays the exact launch
    /// workspace.
    repository: Option<Vec<String>>,
}

impl SessionsPanel {
    pub fn new(show_acp: bool) -> Self {
        Self {
            request_id: None,
            paged: None,
            due: Some(Instant::now()),
            append: false,
            next_cursor: None,
            show_acp,
            scope: SessionsScope::Local(None),
            repository: None,
            list: ListPanel::new(
                TITLE_WORKSPACE,
                HINT,
                "Loading sessions...",
                EMPTY_WORKSPACE,
                "No matching sessions.",
                true,
            ),
        }
    }

    pub fn set_items(&mut self, items: Result<Vec<SessionItem>, String>) {
        if self.paged.is_none() {
            self.paged = Some(false);
        }
        let workspaces = self.names_workspaces();
        self.list
            .set_items(items.map(|items| session_rows(items, workspaces)), None);
    }

    pub fn page(
        &mut self,
        id: &str,
        paged: bool,
        result: Result<(Vec<SessionItem>, Option<String>), String>,
    ) {
        if self.request_id.as_deref() != Some(id) {
            return;
        }
        self.request_id = None;
        self.paged = Some(paged);
        self.list.remote_filter(paged);
        match result {
            Ok((items, next_cursor)) => {
                self.next_cursor = next_cursor;
                if self.append {
                    self.list
                        .append_items(session_rows(items, self.names_workspaces()));
                } else {
                    self.set_items(Ok(items));
                }
            }
            Err(error) => self.set_items(Err(error)),
        }
    }

    pub fn request(&mut self) -> Option<Command> {
        let reset = self.due.is_some_and(|due| Instant::now() >= due);
        if self.due.is_some() && !reset {
            return None;
        }
        if !reset
            && (self.request_id.is_some() || !self.list.near_end() || self.next_cursor.is_none())
        {
            return None;
        }
        self.due = None;
        self.append = !reset;
        let id = uuid::Uuid::new_v4().to_string();
        self.request_id = Some(id.clone());
        Some(Command::ListSessions {
            id,
            query: self.list.query().trim().to_owned(),
            cursor: if reset { None } else { self.next_cursor.take() },
            show_acp: self.show_acp,
            scope: self.scope.clone(),
        })
    }

    /// The launch workspace's git set arrived: the local scope adopts it. A
    /// set of more than one changes what it lists, so the cursor is dropped
    /// and the first page is read again; a one-entry set lists exactly the
    /// launch workspace — canonical `cwd` treats it as the same filter — so
    /// only the title changes, and a list already read stays.
    pub fn repository(&mut self, workspaces: Vec<String>) {
        if workspaces.is_empty() {
            // A failed git read sends nothing; keep the exact launch
            // workspace.
            return;
        }
        self.repository = Some(workspaces.clone());
        if let SessionsScope::Local(_) = &self.scope {
            self.scope = SessionsScope::Local(Some(workspaces));
            if self.names_workspaces() {
                // A set of more than one changes what the local scope lists:
                // drop the cursor and read the first page again.
                self.list.set_title(t(TITLE_REPOSITORY));
                self.list.set_empty_text(EMPTY_OTHER);
                self.refresh();
            } else {
                // Only the title changes: a one-entry set lists exactly the
                // launch workspace, so `cwd` is the filter already under way.
                self.list.set_title(t(TITLE_REPOSITORY));
            }
        }
    }

    pub fn refresh(&mut self) {
        self.request_id = None;
        self.next_cursor = None;
        self.due = Some(Instant::now());
        self.list.loading();
    }

    fn filter_changed(&mut self) {
        if self.paged != Some(false) {
            self.refresh();
            self.due = Some(Instant::now() + Duration::from_millis(150));
        }
    }

    /// A paste goes into the filter.
    pub fn paste(&mut self, text: &str) {
        self.list.paste(text);
        self.filter_changed();
    }

    fn names_workspaces(&self) -> bool {
        match &self.scope {
            SessionsScope::All => true,
            SessionsScope::Local(Some(workspaces)) => workspaces.len() > 1,
            SessionsScope::Local(None) => false,
        }
    }

    fn next_scope(&self) -> SessionsScope {
        match &self.scope {
            SessionsScope::Local(_) => SessionsScope::All,
            // The local scope returns as it now is: the git set when git
            // answered, the exact launch workspace otherwise.
            SessionsScope::All => SessionsScope::Local(self.repository.clone()),
        }
    }

    /// Ctrl+S widens and narrows the list like a new query: the scope
    /// changes and the first page is read again.
    fn cycle_scope(&mut self) {
        self.scope = self.next_scope();
        let (title, empty) = match &self.scope {
            SessionsScope::Local(None) => (TITLE_WORKSPACE, EMPTY_WORKSPACE),
            SessionsScope::Local(Some(workspaces)) if workspaces.len() > 1 => {
                (TITLE_REPOSITORY, EMPTY_OTHER)
            }
            // A one-entry local set lists exactly the launch workspace.
            SessionsScope::Local(Some(_)) => (TITLE_REPOSITORY, EMPTY_WORKSPACE),
            SessionsScope::All => (TITLE_ALL, EMPTY_OTHER),
        };
        self.list.set_title(t(title));
        self.list.set_empty_text(empty);
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Effect {
        if key.code == KeyCode::Char('s') && key.mods.contains(Modifiers::CTRL) {
            self.cycle_scope();
            return Effect::Refresh;
        }
        let before = self.list.query().to_owned();
        let event = self.list.handle_key(key);
        if before != self.list.query() {
            self.filter_changed();
        }
        match event {
            Event::None => Effect::None,
            Event::Close => Effect::Close,
            Event::Pick(id) => Effect::Resume(id),
            Event::Refresh => Effect::Refresh,
        }
    }

    pub fn cursor(&self, width: usize) -> Option<(usize, usize)> {
        self.list.cursor(width)
    }

    pub fn render(&self, width: usize, rows: usize, p: &Palette) -> Vec<String> {
        self.list.render(width, rows, p)
    }
}

fn session_rows(items: Vec<SessionItem>, workspaces: bool) -> Vec<Item> {
    let now = now_ms();
    items
        .iter()
        .map(|item| {
            let detail = if workspaces {
                format!(
                    "{} · {}",
                    relative_time(item.updated_at, now),
                    tilde(&item.cwd)
                )
            } else {
                relative_time(item.updated_at, now)
            };
            let mut row = Item::new(
                item.id.clone(),
                match tag(item.origin.as_deref()) {
                    Some(tag) => format!("{tag} {}", label(item)),
                    None => label(item),
                },
                detail,
            );
            row.search = search_text(item);
            row
        })
        .collect()
}

fn search_text(item: &SessionItem) -> String {
    [
        item.title.as_deref(),
        Some(item.id.as_str()),
        Some(item.cwd.as_str()),
        Some(item.model.as_str()),
        item.preview.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ")
}

/// The row's class, by the origin the daemon reports. A fixed token, not
/// prose: the three labels stay one column wide (#557).
fn tag(origin: Option<&str>) -> Option<&'static str> {
    match origin {
        Some("acp") => Some("[ACP ]"),
        Some("internal") => Some("[Agent]"),
        Some("client") | Some("unknown") => Some("[Chat]"),
        _ => None,
    }
}

/// The stored title unless it is the placeholder, else the first words, else
/// the id.
pub fn label(item: &SessionItem) -> String {
    if let Some(title) = stored_title(item.title.as_deref()) {
        return title.to_owned();
    }
    let preview = item.preview.as_deref().map(str::trim).unwrap_or("");
    if !preview.is_empty() {
        return preview.replace('\n', " ");
    }
    item.title
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map_or_else(|| item.id.clone(), str::to_owned)
}

pub fn relative_time(updated_at: u64, now: u64) -> String {
    let elapsed = now.saturating_sub(updated_at);
    const MINUTE: u64 = 60_000;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    if elapsed < MINUTE {
        t("now").to_owned()
    } else if elapsed < HOUR {
        tf(
            "{count}m ago",
            &[("count", &(elapsed / MINUTE).to_string())],
        )
    } else if elapsed < DAY {
        tf("{count}h ago", &[("count", &(elapsed / HOUR).to_string())])
    } else {
        tf("{count}d ago", &[("count", &(elapsed / DAY).to_string())])
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::terminal::{KeyCode, Modifiers};
    use crate::ui::theme::{Appearance, ColorMode, palette::terminal_palette};

    fn item(id: &str, title: Option<&str>, preview: Option<&str>) -> SessionItem {
        SessionItem {
            id: id.into(),
            title: title.map(str::to_owned),
            preview: preview.map(str::to_owned),
            cwd: "/work/alpha".into(),
            updated_at: 0,
            model: "p/m".into(),
            origin: Some("client".into()),
        }
    }

    /// The panel's drawn text, title and empty line included.
    fn shown(panel: &SessionsPanel) -> String {
        let palette = terminal_palette(ColorMode::TrueColor, Appearance::Dark);
        panel
            .render(72, 12, &palette)
            .iter()
            .map(|row| crate::ui::render::panel::strip_escapes(row))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Ctrl+S, followed by what the TUI does for `Effect::Refresh`: refresh
    /// the panel and ask it for the next page.
    fn ctrl_s(panel: &mut SessionsPanel) -> SessionsScope {
        let effect = panel.handle_key(KeyEvent::new(KeyCode::Char('s'), Modifiers::CTRL));
        assert!(matches!(effect, Effect::Refresh));
        panel.refresh();
        match panel.request() {
            Some(Command::ListSessions { scope, cursor, .. }) => {
                assert!(cursor.is_none(), "a scope change reads the first page");
                scope
            }
            _ => panic!("a scope change asks for the first page"),
        }
    }

    #[test]
    fn label_prefers_a_real_name_then_the_first_message() {
        assert_eq!(label(&item("a", Some("Fix bug"), Some("hi"))), "Fix bug");
        assert_eq!(
            label(&item("a", Some("New session"), Some("hello there"))),
            "hello there"
        );
        assert_eq!(label(&item("abc", None, None)), "abc");
    }

    #[test]
    fn moving_up_inside_the_window_moves_the_highlight_not_the_list() {
        use crate::ui::theme::{Appearance, ColorMode, palette::terminal_palette};
        let p = terminal_palette(ColorMode::TrueColor, Appearance::Dark);
        let mut panel = SessionsPanel::new(false);
        let items = (0..30)
            .map(|i| item(&format!("s{i}"), Some(&format!("session {i:02}")), None))
            .collect();
        panel.set_items(Ok(items));
        let key = |code| KeyEvent::new(code, Modifiers::NONE);
        let shown = |panel: &SessionsPanel| -> Vec<String> {
            panel
                .render(60, 10, &p)
                .iter()
                .map(|r| {
                    let plain = crate::ui::render::panel::strip_escapes(r);
                    if r.contains("\x1b[30;43m") {
                        plain.replacen("  [Chat] session", "› [Chat] session", 1)
                    } else {
                        plain
                    }
                })
                .filter(|r| r.contains("session "))
                .collect()
        };
        shown(&panel);
        for _ in 0..7 {
            panel.handle_key(key(KeyCode::Down));
        }
        let rows = shown(&panel);
        assert!(
            rows[0].contains("session 03") && rows[4].contains("› [Chat] session 07"),
            "{rows:?}"
        );
        panel.handle_key(key(KeyCode::Up));
        panel.handle_key(key(KeyCode::Up));
        let rows = shown(&panel);
        assert!(
            rows[0].contains("session 03") && rows[2].contains("› [Chat] session 05"),
            "{rows:?}"
        );
        for _ in 0..3 {
            panel.handle_key(key(KeyCode::Up));
        }
        let rows = shown(&panel);
        assert!(rows[0].contains("› [Chat] session 02"), "{rows:?}");
    }

    #[test]
    fn filter_narrows_and_enter_resumes_the_selected_row() {
        let mut panel = SessionsPanel::new(false);
        panel.set_items(Ok(vec![
            item("one", None, Some("alpha topic")),
            item("two", None, Some("beta topic")),
        ]));
        for c in "beta".chars() {
            panel.handle_key(KeyEvent::new(KeyCode::Char(c), Modifiers::NONE));
        }
        match panel.handle_key(KeyEvent::new(KeyCode::Enter, Modifiers::NONE)) {
            Effect::Resume(id) => assert_eq!(id, "two"),
            _ => panic!("expected resume"),
        }
    }

    /// Two stops whatever git answered: the exact launch workspace, a
    /// one-worktree repository, sibling worktrees, or nothing at all.
    #[test]
    fn the_cycle_has_two_states_after_every_kind_of_git_answer() {
        let mut panel = SessionsPanel::new(false);
        assert_eq!(ctrl_s(&mut panel), SessionsScope::All);
        assert_eq!(ctrl_s(&mut panel), SessionsScope::Local(None));

        let mut panel = SessionsPanel::new(false);
        panel.repository(vec!["/repo".to_owned()]);
        assert_eq!(ctrl_s(&mut panel), SessionsScope::All);
        assert_eq!(
            ctrl_s(&mut panel),
            SessionsScope::Local(Some(vec!["/repo".to_owned()]))
        );

        let mut panel = SessionsPanel::new(false);
        let set = vec!["/repo".to_owned(), "/repo-fix".to_owned()];
        panel.repository(set.clone());
        assert_eq!(ctrl_s(&mut panel), SessionsScope::All);
        assert_eq!(ctrl_s(&mut panel), SessionsScope::Local(Some(set)));

        // git answers while every workspace is shown: the local stop
        // returns to the repository, without disturbing the list shown.
        let mut panel = SessionsPanel::new(false);
        ctrl_s(&mut panel);
        let set = vec!["/repo".to_owned(), "/repo-fix".to_owned()];
        panel.repository(set.clone());
        assert_eq!(ctrl_s(&mut panel), SessionsScope::Local(Some(set)));
    }

    /// The local title comes from git detection: any set, one entry
    /// included, is the repository.
    #[test]
    fn the_title_names_the_repository_however_many_worktrees_git_found() {
        let mut panel = SessionsPanel::new(false);
        assert!(shown(&panel).contains(TITLE_WORKSPACE));
        ctrl_s(&mut panel);
        assert!(shown(&panel).contains(TITLE_ALL));
        ctrl_s(&mut panel);
        assert!(shown(&panel).contains(TITLE_WORKSPACE));

        let mut panel = SessionsPanel::new(false);
        panel.repository(vec!["/repo".to_owned()]);
        assert!(shown(&panel).contains(TITLE_REPOSITORY));
        ctrl_s(&mut panel);
        assert!(shown(&panel).contains(TITLE_ALL));
        ctrl_s(&mut panel);
        assert!(shown(&panel).contains(TITLE_REPOSITORY));
    }

    /// `/resume` opens before git replies: the answer changes the title,
    /// and only a set of more than one reads the list again.
    #[test]
    fn a_one_entry_set_changes_only_the_title_and_a_multi_entry_set_reads_again() {
        for (set, reread) in [
            (vec!["/repo".to_owned()], false),
            (vec!["/repo".to_owned(), "/repo-fix".to_owned()], true),
        ] {
            let mut panel = SessionsPanel::new(false);
            let Some(Command::ListSessions {
                id,
                scope: SessionsScope::Local(None),
                cursor: None,
                ..
            }) = panel.request()
            else {
                panic!("the opening read lists the exact launch workspace");
            };
            panel.page(
                &id,
                true,
                Ok((vec![item("a", Some("Fix bug"), None)], None)),
            );
            panel.repository(set.clone());
            assert!(shown(&panel).contains(TITLE_REPOSITORY));
            let next = panel.request();
            if reread {
                match next {
                    Some(Command::ListSessions {
                        scope: SessionsScope::Local(Some(asked)),
                        cursor: None,
                        ..
                    }) => assert_eq!(asked, set),
                    _ => panic!("a multi-entry set reads the first page again"),
                }
            } else {
                assert!(
                    next.is_none(),
                    "a one-entry set lists the filter already read"
                );
            }
        }
    }

    /// The workspace line names the launch workspace only while the local
    /// scope lists exactly it; a one-entry set lists the same, so the line
    /// stays.
    #[test]
    fn the_empty_text_keeps_the_workspace_line_while_the_local_scope_lists_it() {
        let mut panel = SessionsPanel::new(false);
        panel.set_items(Ok(Vec::new()));
        assert!(shown(&panel).contains(EMPTY_WORKSPACE));
        panel.repository(vec!["/repo".to_owned()]);
        assert!(shown(&panel).contains(EMPTY_WORKSPACE));

        let mut all = SessionsPanel::new(false);
        ctrl_s(&mut all);
        all.set_items(Ok(Vec::new()));
        assert!(shown(&all).contains(EMPTY_OTHER));
        assert!(!shown(&all).contains(EMPTY_WORKSPACE));

        let mut several = SessionsPanel::new(false);
        several.repository(vec!["/repo".to_owned(), "/repo-fix".to_owned()]);
        several.set_items(Ok(Vec::new()));
        assert!(shown(&several).contains(EMPTY_OTHER));
        assert!(!shown(&several).contains(EMPTY_WORKSPACE));
    }

    /// A row names its workspace inside the local scope only when the scope
    /// holds more than one, so a one-worktree repository adds no path.
    #[test]
    fn rows_name_their_workspace_only_when_the_local_scope_holds_more_than_one() {
        let rows = || vec![item("a", Some("Fix bug"), None)];
        let workspace = "/work/alpha";

        let mut panel = SessionsPanel::new(false);
        panel.set_items(Ok(rows()));
        assert!(!shown(&panel).contains(workspace));
        panel.repository(vec![workspace.to_owned()]);
        assert!(!shown(&panel).contains(workspace));

        panel.repository(vec![workspace.to_owned(), "/work/beta".to_owned()]);
        panel.set_items(Ok(rows()));
        assert!(shown(&panel).contains(workspace));

        let mut all = SessionsPanel::new(false);
        ctrl_s(&mut all);
        all.set_items(Ok(rows()));
        assert!(shown(&all).contains(workspace));
    }
}
