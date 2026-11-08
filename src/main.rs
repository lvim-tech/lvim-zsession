// lvim-zsession — keep zellij's own session resurrection, fix its one wart.
//
// zellij serializes sessions on its own (session_serialization true) and brings them back by
// name: the session-manager lists them, `zellij attach <name>` resurrects them. A pane that was
// running a program (nvim, claude, ...) comes back as a "command pane" — press ENTER, the program
// runs — but there is NO shell underneath it: quit the program and you are left with a dead pane
// (a blank screen after a TUI drops its alt-screen) and no prompt to start the next thing from.
//
// This plugin repairs exactly that, live, without touching serialization:
//
//   * It watches PaneUpdate (broadcast to every plugin). When a terminal pane reports
//     `exited` — a command pane whose program finished and is now held — the plugin replaces it
//     IN PLACE with a real interactive shell. The shell inherits the directory of the pane it
//     replaces (cwd:None → zellij fills it from the replaced pane: its live process, or the cwd
//     it was spawned with), so you land at a prompt in the program's project directory.
//   * The shell is spawned as the bare default shell (no args), the same way zellij spawns its
//     own panes, so the serializer records it as a plain pane and a program you start in it is
//     discovered and serialized natively — the next resurrection works like always.
//   * When one of OUR shells exits (Ctrl-D) it is simply closed — no loop into another shell.
//     Panes that are held but not yet started (resurrected, waiting for ENTER) are left alone.
//
// Why not CommandPaneExited: zellij delivers that event only to the plugin that opened the pane,
// so a resurrected pane never reports it. PaneUpdate is the only broadcast signal.
//
// Extra: pipe "save" → save_session() (serialize now, e.g. `Ctrl s` then `s` before logging out).

#![cfg_attr(test, allow(dead_code))]

use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::time::Duration;
use zellij_tile::prelude::*;

// zellij's `default_shell` is "zsh" in this setup; override with the plugin configuration key
// `shell` if it ever changes. Must match what zellij spawns for a plain pane (name, no args) so
// the serializer keeps treating our replacement shells as plain panes.
const DEFAULT_SHELL: &str = "zsh";

mod panel;
mod ui;

use panel::{Agents, Kind, Row, Saved};

#[derive(Default)]
struct State {
    /// `panel "on"`: this instance is the session panel (a floating pane opened by `Ctrl s`, `s`)
    /// and does nothing else. The background instance (load_plugins) keeps the pane repair and the
    /// `save` pipe; the two must never both replace one pane.
    panel: Option<Panel>,
    shell: String,
    ours: HashSet<u32>,    // terminal ids of the shells this plugin spawned
    handled: HashSet<u32>, // exited panes already acted on (replacement/close in flight)
}

// Host tests cannot link the plugin's exports (they import zellij's host functions), so the
// registration is left out of the test build and the panel's pure parts are tested alone.
#[cfg(not(test))]
register_plugin!(State);

impl ZellijPlugin for State {
    fn load(&mut self, configuration: BTreeMap<String, String>) {
        if configuration.get("panel").map(|v| v.trim()) == Some("on") {
            self.panel = Some(Panel::new(&configuration));
            request_permission(&[
                PermissionType::ReadApplicationState, // SessionUpdate, SaveSession
                PermissionType::ChangeApplicationState, // switch, rename, kill, delete
                PermissionType::RunCommands,          // the saved layouts
                PermissionType::MessageAndLaunchOtherPlugins, // ask lvim-zclaude, notice
                PermissionType::ReadCliPipes,         // lvim-zclaude answers with `zellij pipe`
            ]);
            subscribe(&[
                EventType::SessionUpdate,
                EventType::Key,
                EventType::RunCommandResult,
                EventType::PermissionRequestResult,
                EventType::ModeUpdate,
            ]);
            return;
        }
        self.shell = configuration
            .get("shell")
            .cloned()
            .unwrap_or_else(|| DEFAULT_SHELL.to_string());
        request_permission(&[
            PermissionType::ReadApplicationState, // PaneUpdate, SaveSession
            PermissionType::ChangeApplicationState, // close panes
            // OpenCommandPaneInPlaceOfPaneId is gated behind RunCommands (measured against
            // zellij 0.46: the denial names it; the shim then panics on the empty response).
            PermissionType::RunCommands,
            PermissionType::MessageAndLaunchOtherPlugins, // the "notice" broadcast
        ]);
        subscribe(&[EventType::PaneUpdate, EventType::CommandPaneExited]);
    }

    fn update(&mut self, event: Event) -> bool {
        if let Some(panel) = self.panel.as_mut() {
            return panel.update(event);
        }
        match event {
            Event::PaneUpdate(manifest) => {
                let mut alive: HashSet<u32> = HashSet::new();
                for pane in manifest.panes.values().flatten() {
                    if pane.is_plugin {
                        continue;
                    }
                    alive.insert(pane.id);
                    if !pane.exited || self.handled.contains(&pane.id) {
                        continue;
                    }
                    self.handled.insert(pane.id);
                    if self.ours.contains(&pane.id)
                        || self.is_shell(pane.terminal_command.as_deref())
                    {
                        // our shell (or a bare shell pane) finished: a normal close, no new shell
                        close_pane_with_id(PaneId::Terminal(pane.id));
                    } else {
                        self.replace_with_shell(pane.id);
                    }
                }
                // Forget panes that are gone. `ours` is kept in full: it is tiny and a stale
                // manifest generated before a replacement landed must not drop a fresh shell.
                self.handled.retain(|id| alive.contains(id));
            }
            // Only ever delivered for panes THIS plugin opened — i.e. one of our shells exited.
            Event::CommandPaneExited(id, _, _) if self.handled.insert(id) => {
                close_pane_with_id(PaneId::Terminal(id));
            }
            _ => {}
        }
        false
    }

    fn pipe(&mut self, message: PipeMessage) -> bool {
        if let Some(panel) = self.panel.as_mut() {
            if message.name == "lvim-zclaude-summary" {
                let payload = message.payload.as_deref().unwrap_or("");
                if !panel::has_per_session(payload) {
                    return false;
                }
                let agents = panel::parse_agents(payload);
                let changed = agents != panel.agents;
                panel.agents = agents;
                return changed;
            }
            return false;
        }
        if message.name == "save" {
            let outcome = match save_session() {
                Ok(()) => "session saved".to_string(),
                Err(e) => format!("session save FAILED: {e}"),
            };
            notice(&outcome);
        }
        if message.name == "restart" {
            restart();
        }
        false
    }

    fn render(&mut self, rows: usize, cols: usize) {
        if let Some(panel) = self.panel.as_ref() {
            panel.render(rows, cols);
        }
    }
}

#[derive(Default)]
enum Mode {
    #[default]
    List,
    /// Renaming this session; the buffer.
    Rename(String),
    /// Naming a new session; the buffer.
    New(String),
    /// Del was pressed on this name; `y` (or Del again) does it.
    Confirm(String),
}

/// Where the panel sits — lvim-zclaude's board size, so the two floating panels match. A launched
/// pane is first drawn at zellij's default size; it can move itself only once permitted.
const SIZE: [(&str, &str); 4] = [
    ("x", "10%"),
    ("y", "15%"),
    ("width", "80%"),
    ("height", "70%"),
];

#[derive(Default)]
struct Panel {
    live: Vec<SessionInfo>,
    dead: Vec<(String, Duration)>,
    saved: BTreeMap<String, Saved>,
    /// The tab names of each saved layout.
    saved_tabs: BTreeMap<String, Vec<(String, bool)>>,
    /// The tab chosen with ←/→ in the detail line — Enter lands there. Cleared when the row changes.
    tab: Option<usize>,
    agents: BTreeMap<String, Agents>,
    /// The resurrectable names the saved layouts were last read for.
    saved_for: Vec<String>,
    filter: String,
    selected: usize,
    mode: Mode,
    message: Option<String>,
    asked: bool,
    palette: Option<ui::Palette>,
    /// x, y, width, height — from the configuration, or `SIZE`.
    geometry: [String; 4],
}

impl Panel {
    fn new(configuration: &BTreeMap<String, String>) -> Panel {
        let get = |(key, fallback): (&str, &str)| {
            configuration
                .get(key)
                .filter(|v| !v.trim().is_empty())
                .cloned()
                .unwrap_or_else(|| fallback.to_string())
        };
        Panel {
            geometry: SIZE.map(get),
            ..Panel::default()
        }
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::PermissionRequestResult(PermissionStatus::Granted) => self.first_look(),
            Event::PermissionRequestResult(_) => false,
            Event::ModeUpdate(mode) => {
                self.palette = Some(ui::Palette::from_style(&mode.style.colors));
                self.first_look();
                true
            }
            Event::SessionUpdate(live, dead) => {
                self.first_look();
                self.take(live, dead);
                true
            }
            Event::RunCommandResult(_, stdout, _, ctx) => {
                if ctx.get("step").map(String::as_str) != Some("saved") {
                    return false;
                }
                let out = String::from_utf8_lossy(&stdout);
                self.saved = panel::parse_saved(&out);
                self.saved_tabs = panel::parse_saved_tabs(&out);
                true
            }
            Event::Key(key) => self.key(key),
            _ => false,
        }
    }

    /// Once, as soon as the panel may act: its size and title, the session list fresh from disk,
    /// and the agents from lvim-zclaude. `SessionUpdate` alone is not enough — a server that no
    /// session list has asked about yet knows only itself, and then sends only its own changes.
    fn first_look(&mut self) -> bool {
        if self.asked {
            return false;
        }
        self.asked = true;
        let own = get_plugin_ids().plugin_id;
        rename_plugin_pane(own, "sessions");
        let [x, y, w, h] = self.geometry.clone();
        if let Some(coordinates) =
            FloatingPaneCoordinates::new(Some(x), Some(y), Some(w), Some(h), None, None)
        {
            change_floating_panes_coordinates(vec![(PaneId::Plugin(own), coordinates)]);
        }
        pipe_message_to_plugin(MessageToPlugin::new("lvim-zclaude-summary-request"));
        match get_session_list() {
            Ok(snapshot) => {
                self.take(snapshot.live_sessions, snapshot.resurrectable_sessions);
                true
            }
            Err(e) => {
                eprintln!("lvim-zsession: session list: {e}");
                false
            }
        }
    }

    fn take(&mut self, live: Vec<SessionInfo>, dead: Vec<(String, Duration)>) {
        // An update that knows only this session must not empty a list the snapshot filled.
        if live.len() <= 1 && dead.is_empty() && self.live.len() + self.dead.len() > 1 {
            self.live.retain(|s| !s.is_current_session);
            self.live.extend(live);
            return;
        }
        let mut names: Vec<String> = dead.iter().map(|(n, _)| n.clone()).collect();
        names.sort();
        if names != self.saved_for {
            self.saved_for = names;
            run_command(
                &["sh", "-c", panel::SAVED_SCRIPT],
                BTreeMap::from([("step".to_string(), "saved".to_string())]),
            );
        }
        self.live = live;
        self.dead = dead;
    }

    fn rows(&self) -> Vec<Row> {
        let mut rows: Vec<Row> = self
            .live
            .iter()
            .map(|s| Row {
                name: s.name.clone(),
                kind: Kind::Live {
                    tabs: s.tabs.len(),
                    panes: s
                        .panes
                        .panes
                        .values()
                        .flatten()
                        .filter(|p| !p.is_plugin)
                        .count(),
                    clients: s.connected_clients,
                    current: s.is_current_session,
                },
            })
            .chain(self.dead.iter().map(|(name, age)| Row {
                name: name.clone(),
                kind: Kind::Dead {
                    age: *age,
                    saved: self.saved.get(name).copied(),
                },
            }))
            .collect();
        panel::sort(&mut rows);
        rows
    }

    /// The sessions the filter lets through, and last the row that makes a new one — named after
    /// the filter when no session has exactly that name.
    fn items(&self) -> Vec<ui::Item> {
        let rows = self.rows();
        let mut items: Vec<ui::Item> = panel::filter(&rows, &self.filter)
            .into_iter()
            .cloned()
            .map(ui::Item::Session)
            .collect();
        let typed = self.filter.trim();
        if typed.is_empty() {
            items.push(ui::Item::New(None));
        } else if !rows.iter().any(|r| r.name.eq_ignore_ascii_case(typed)) {
            items.push(ui::Item::New(Some(typed.to_string())));
        }
        items
    }

    fn current_item(&self) -> Option<ui::Item> {
        let items = self.items();
        items
            .get(self.selected.min(items.len().saturating_sub(1)))
            .cloned()
    }

    fn key(&mut self, key: KeyWithModifier) -> bool {
        let bare = |b: BareKey| key.is_key_without_modifier(b);
        match std::mem::take(&mut self.mode) {
            Mode::Rename(buf) => {
                self.mode = self.edit(&key, buf, true);
                return true;
            }
            Mode::New(buf) => {
                self.mode = self.edit(&key, buf, false);
                return true;
            }
            Mode::Confirm(name) => {
                if bare(BareKey::Delete) || bare(BareKey::Char('y')) {
                    self.remove(&name);
                } else {
                    self.message = None;
                }
                return true;
            }
            Mode::List => {}
        }
        let count = self.items().len();
        let before = (self.selected, self.filter.clone());
        let handled = self.list_key(&key, count);
        if (self.selected, self.filter.clone()) != before {
            self.tab = None;
        }
        handled
    }

    fn list_key(&mut self, key: &KeyWithModifier, count: usize) -> bool {
        let ctrl = |b: BareKey| key.is_key_with_ctrl_modifier(b);
        let bare = |b: BareKey| key.is_key_without_modifier(b);
        if bare(BareKey::Left) || bare(BareKey::Right) {
            let tabs = self.tabs_of_current();
            if !tabs.is_empty() {
                let n = tabs.len();
                let active = tabs.iter().position(|(_, active)| *active);
                if self.tab.is_none() && active.is_none() {
                    // A saved session has no active tab: the first arrow lands on an end.
                    self.tab = Some(if bare(BareKey::Right) { 0 } else { n - 1 });
                    return true;
                }
                let from = self.tab.unwrap_or_else(|| active.unwrap_or(0));
                self.tab = Some(if bare(BareKey::Right) {
                    (from + 1) % n
                } else {
                    (from + n - 1) % n
                });
            }
            return true;
        }
        if bare(BareKey::Esc) || ctrl(BareKey::Char('c')) {
            close_self();
        } else if bare(BareKey::Down) || bare(BareKey::Tab) || ctrl(BareKey::Char('j')) {
            if count > 0 {
                self.selected = (self.selected + 1) % count;
            }
        } else if bare(BareKey::Up) || ctrl(BareKey::Char('p')) || ctrl(BareKey::Char('k')) {
            if count > 0 {
                self.selected = (self.selected + count - 1) % count;
            }
        } else if bare(BareKey::Enter) {
            self.open();
        } else if ctrl(BareKey::Char('n')) {
            self.message = None;
            self.mode = Mode::New(self.filter.trim().to_string());
        } else if bare(BareKey::Delete) {
            if let Some(ui::Item::Session(row)) = self.current_item() {
                let what = if row.is_current() {
                    "kill THIS session — everything running here ends".to_string()
                } else if row.is_live() {
                    format!("kill {}", row.name)
                } else {
                    format!("delete the saved {}", row.name)
                };
                self.message = Some(format!("{what}?"));
                self.mode = Mode::Confirm(row.name);
            }
        } else if ctrl(BareKey::Char('r')) {
            let current = self
                .rows()
                .into_iter()
                .find(|r| r.is_current())
                .map(|r| r.name);
            self.message = None;
            self.mode = Mode::Rename(current.unwrap_or_default());
        } else if ctrl(BareKey::Char('a')) {
            self.message = Some(match save_session() {
                Ok(()) => "session saved".to_string(),
                Err(e) => format!("save FAILED: {e}"),
            });
            notice(self.message.as_deref().unwrap_or(""));
        } else if ctrl(BareKey::Char('x')) {
            disconnect_other_clients();
            self.message = Some("other clients disconnected".to_string());
        } else if bare(BareKey::Backspace) {
            self.filter.pop();
            self.selected = 0;
        } else if let Some(c) = typed(key) {
            self.filter.push(c);
            self.selected = 0;
        } else {
            return false;
        }
        true
    }

    /// A terminal pane of a live session's tab (by position): focusing it is how the switch lands
    /// on that tab.
    fn pane_in_tab(&self, session: &str, tab: usize) -> Option<u32> {
        let info = self.live.iter().find(|s| s.name == session)?;
        let panes = info.panes.panes.get(&tab)?;
        panes
            .iter()
            .filter(|p| !p.is_plugin && !p.is_suppressed)
            .min_by_key(|p| (!p.is_focused, p.id))
            .map(|p| p.id)
    }

    /// The selected session's tabs in order, and which one is active (a saved session: the one
    /// its layout was saved on).
    fn tabs_of_current(&self) -> Vec<(String, bool)> {
        let Some(ui::Item::Session(row)) = self.current_item() else {
            return Vec::new();
        };
        let saved = self.saved_tabs.get(&row.name);
        let mut tabs: Vec<(String, bool)> = match self.live.iter().find(|s| s.name == row.name) {
            Some(info) => {
                let mut tabs: Vec<&TabInfo> = info.tabs.iter().collect();
                tabs.sort_by_key(|t| t.position);
                let mut tabs: Vec<(String, bool)> =
                    tabs.iter().map(|t| (t.name.clone(), t.active)).collect();
                // `active` is per client: a session nobody is attached to has none. The layout
                // zellij keeps saving for it says which tab it was left on.
                if !tabs.iter().any(|(_, a)| *a) {
                    if let Some(focus) = saved.and_then(|s| s.iter().position(|(_, f)| *f)) {
                        if let Some(tab) = tabs.get_mut(focus) {
                            tab.1 = true;
                        }
                    }
                }
                tabs
            }
            None => saved.cloned().unwrap_or_default(),
        };
        // Nothing says which: the first, where zellij puts a client by default.
        if !tabs.iter().any(|(_, a)| *a) {
            if let Some(first) = tabs.first_mut() {
                first.1 = true;
            }
        }
        tabs
    }

    /// A key in the input line (rename, new session): the mode that follows.
    fn edit(&mut self, key: &KeyWithModifier, mut buf: String, rename: bool) -> Mode {
        if key.is_key_without_modifier(BareKey::Esc) {
            self.message = None;
            return Mode::List;
        }
        if key.is_key_without_modifier(BareKey::Enter) {
            let name = buf.trim().to_string();
            if rename {
                if !name.is_empty() {
                    rename_session(&name);
                    self.message = Some(format!("renamed to {name}"));
                }
            } else {
                self.create(&name);
            }
            return Mode::List;
        }
        if key.is_key_without_modifier(BareKey::Backspace) {
            buf.pop();
        } else if let Some(c) = typed(key) {
            buf.push(c);
        }
        if rename {
            Mode::Rename(buf)
        } else {
            Mode::New(buf)
        }
    }

    fn open(&mut self) {
        match self.current_item() {
            Some(ui::Item::New(name)) => self.create(name.as_deref().unwrap_or("")),
            Some(ui::Item::Session(row)) if row.is_current() => {
                close_self();
                if let Some(tab) = self.tab {
                    switch_tab_to(tab as u32 + 1);
                }
            }
            Some(ui::Item::Session(row)) => {
                // A resurrectable name is brought back by the same calls.
                match self.tab.map(|tab| (tab, self.pane_in_tab(&row.name, tab))) {
                    Some((_, Some(pane))) => {
                        switch_session_with_focus(&row.name, None, Some((pane, false)))
                    }
                    Some((tab, None)) => switch_session_with_focus(&row.name, Some(tab), None),
                    None => switch_session(Some(&row.name)),
                }
                close_self();
            }
            None => {}
        }
    }

    /// A new session — named, or with a name zellij makes up — and this client goes there.
    fn create(&mut self, name: &str) {
        let name = name.trim();
        if self.rows().iter().any(|r| r.name == name) {
            self.message = Some(format!("{name} exists — Enter on it opens it"));
            return;
        }
        let name = if name.is_empty() { None } else { Some(name) };
        switch_session_with_cwd(name, None);
        close_self();
    }

    fn remove(&mut self, name: &str) {
        let Some(row) = self.rows().into_iter().find(|r| r.name == name) else {
            return;
        };
        let result = if row.is_live() {
            kill_sessions(&[name])
        } else {
            delete_dead_session(name)
        };
        self.message = Some(match result {
            Ok(()) if row.is_live() => format!("{name} killed"),
            Ok(()) => format!("{name} deleted"),
            Err(e) => format!("{name}: {e}"),
        });
        self.selected = 0;
        if let Ok(snapshot) = get_session_list() {
            self.live = snapshot.live_sessions;
            self.dead = snapshot.resurrectable_sessions;
        }
    }

    /// What the selected row holds that the columns do not: a live session's tabs, a saved one's
    /// age and what comes back, the new-session row's name.
    /// The line under the list: the selected session and its tabs (←/→ chooses one), or what
    /// the new-session row does.
    fn detail(&self) -> ui::Detail {
        match self.current_item() {
            Some(ui::Item::New(Some(name))) => {
                ui::Detail::Text(format!("Enter starts a new session named {name}"))
            }
            Some(ui::Item::New(None)) => ui::Detail::Text(
                "Enter starts a new session (zellij names it) · Ctrl n names it first".to_string(),
            ),
            Some(ui::Item::Session(row)) => {
                // Until an arrow chooses one, the tab shown chosen is the one the session is on.
                let tabs = self.tabs_of_current();
                let active = tabs.iter().position(|(_, active)| *active);
                ui::Detail::Tabs {
                    name: row.name.clone(),
                    tabs: tabs.into_iter().map(|(n, _)| n).collect(),
                    chosen: self.tab.or(active),
                }
            }
            None => ui::Detail::Text(String::new()),
        }
    }

    fn render(&self, rows: usize, cols: usize) {
        let items = self.items();
        let cursor = self.selected.min(items.len().saturating_sub(1));
        let prompt = match &self.mode {
            Mode::Confirm(_) => self.message.as_deref(),
            _ => None,
        };
        let input = match &self.mode {
            Mode::Rename(buf) => Some(("rename this session to: ", buf.as_str())),
            Mode::New(buf) => Some(("new session name (empty = zellij picks): ", buf.as_str())),
            _ => None,
        };
        let view = ui::View {
            items: &items,
            cursor,
            filter: &self.filter,
            agents: &self.agents,
            live: self.live.len(),
            saved: self.dead.len(),
            palette: self.palette,
            detail: self.detail(),
            prompt,
            input,
            message: if prompt.is_some() {
                None
            } else {
                self.message.as_deref()
            },
        };
        ui::draw(&view, rows, cols);
    }
}

/// A character typed without Ctrl/Alt — for the filter and the input line.
fn typed(key: &KeyWithModifier) -> Option<char> {
    match key.bare_key {
        BareKey::Char(c) if key.has_no_modifiers() || key.has_modifiers(&[KeyModifier::Shift]) => {
            Some(c)
        }
        _ => None,
    }
}

impl State {
    fn replace_with_shell(&mut self, pane_id: u32) {
        let shell = CommandToRun {
            path: self.shell.clone().into(),
            args: Vec::new(),
            cwd: None, // inherit the replaced pane's directory (filled in by zellij)
        };
        match open_command_pane_in_place_of_pane_id(
            PaneId::Terminal(pane_id),
            shell,
            true,
            BTreeMap::new(),
        ) {
            Some(PaneId::Terminal(new_id)) => {
                self.ours.insert(new_id);
                eprintln!(
                    "lvim-zsession: pane {pane_id} exited, shell {new_id} opened in its place"
                );
            }
            other => eprintln!("lvim-zsession: could not replace exited pane {pane_id}: {other:?}"),
        }
    }

    // "zsh" / "/bin/zsh" with no arguments — a plain shell, whoever spawned it.
    fn is_shell(&self, terminal_command: Option<&str>) -> bool {
        let Some(cmd) = terminal_command else {
            return false;
        };
        let mut words = cmd.split_whitespace();
        let Some(first) = words.next() else {
            return false;
        };
        if words.next().is_some() {
            return false;
        }
        let name = |s: &str| {
            Path::new(s)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| s.to_string())
        };
        name(first) == name(&self.shell)
    }
}

/// The shell behind `restart`: leave the session's name where the terminal's shell will find it
/// (`$XDG_RUNTIME_DIR/zellij-restart`), then kill the session from a process of its own — the
/// kill takes this server and everything it started with it.
const RESTART_SCRIPT: &str = r#"printf '%s\n' "$1" > "${XDG_RUNTIME_DIR:-/tmp}/zellij-restart" && setsid -f sh -c 'sleep 0.3; exec zellij kill-session "$1"' sh "$1" </dev/null >/dev/null 2>&1"#;

/// `Ctrl s`, `Ctrl r`: save this session, then kill it. Bringing it back (`zellij attach <name>
/// --force-run-commands`) is the job of the shell the client returns to — see README, "Restart":
/// nothing inside the session outlives the kill, and only that shell has the terminal.
fn restart() {
    let name = match get_session_list() {
        Ok(list) => list
            .live_sessions
            .into_iter()
            .find(|s| s.is_current_session)
            .map(|s| s.name),
        Err(e) => {
            notice(&format!("restart FAILED: {e}"));
            return;
        }
    };
    let Some(name) = name else {
        notice("restart FAILED: this session is not in the list");
        return;
    };
    if let Err(e) = save_session() {
        notice(&format!(
            "restart FAILED, the session could not be saved: {e}"
        ));
        return;
    }
    notice(&format!("restarting {name}"));
    run_command(
        &["sh", "-c", RESTART_SCRIPT, "lvim-zsession", &name],
        BTreeMap::new(),
    );
}

fn notice(msg: &str) {
    eprintln!("lvim-zsession: {msg}");
    pipe_message_to_plugin(MessageToPlugin::new("notice").with_payload(msg.to_string()));
}
