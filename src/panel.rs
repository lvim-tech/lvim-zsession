// The session panel: zellij's session-manager, with what it leaves out.
//
// The built-in list shows tabs, panes and clients only for live sessions — a resurrectable one is
// a name and an age, because `SessionUpdate` gives no more. Its saved layout is on disk, though
// (`~/.cache/zellij/<contract>/session_info/<name>/session-layout.kdl`), and that says how many
// tabs and panes it had and which of them ran `claude`. The agents of a live session come from
// lvim-zclaude, which counts them per zellij session in its summary pipe (lines 3+).
//
// Everything here is plain data and text; main.rs does the zellij calls.

use std::collections::BTreeMap;
use std::time::Duration;

/// Agents of one live zellij session, from lvim-zclaude.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Agents {
    pub total: usize,
    pub busy: usize,
    pub waiting: usize,
    pub done: usize,
}

/// What a saved layout says about a session that is not running.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Saved {
    pub tabs: usize,
    pub panes: usize,
    /// Panes whose saved command is `claude` — the agents that come back on resurrection.
    pub agents: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Live {
        tabs: usize,
        panes: usize,
        clients: usize,
        current: bool,
    },
    Dead {
        age: Duration,
        saved: Option<Saved>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub name: String,
    pub kind: Kind,
}

impl Row {
    pub fn is_live(&self) -> bool {
        matches!(self.kind, Kind::Live { .. })
    }
    pub fn is_current(&self) -> bool {
        matches!(self.kind, Kind::Live { current: true, .. })
    }
}

/// Current first, then the other live sessions by name, then the resurrectable ones, newest first.
pub fn sort(rows: &mut [Row]) {
    rows.sort_by(|a, b| {
        let rank = |r: &Row| match r.kind {
            Kind::Live { current: true, .. } => 0,
            Kind::Live { .. } => 1,
            Kind::Dead { .. } => 2,
        };
        rank(a)
            .cmp(&rank(b))
            .then_with(|| match (&a.kind, &b.kind) {
                (Kind::Dead { age: x, .. }, Kind::Dead { age: y, .. }) => x.cmp(y),
                _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            })
    });
}

/// The rows whose name holds every character of the filter in order (case-insensitive), so
/// `sch` finds SCHOOL and `lv` finds LVIM.
pub fn filter<'a>(rows: &'a [Row], needle: &str) -> Vec<&'a Row> {
    let needle = needle.to_lowercase();
    rows.iter()
        .filter(|row| {
            let name = row.name.to_lowercase();
            let mut chars = name.chars();
            needle.chars().all(|n| chars.any(|c| c == n))
        })
        .collect()
}

/// lvim-zclaude's summary pipe: lines 3+ are `session<TAB>name<TAB>agents<TAB>busy<TAB>waiting<TAB>done`.
pub fn parse_agents(payload: &str) -> BTreeMap<String, Agents> {
    let mut out = BTreeMap::new();
    for line in payload.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 6 || f[0] != "session" {
            continue;
        }
        let n = |i: usize| f[i].trim().parse::<usize>().unwrap_or(0);
        out.insert(
            f[1].to_string(),
            Agents {
                total: n(2),
                busy: n(3),
                waiting: n(4),
                done: n(5),
            },
        );
    }
    out
}

/// Whether a summary can say anything per session: it has `session` lines, or it says there are
/// no agents at all. A board built before the per-session lines sends only the two bar lines;
/// that says nothing about sessions and must not wipe what a newer board said.
pub fn has_per_session(payload: &str) -> bool {
    if payload.lines().any(|l| l.starts_with("session\t")) {
        return true;
    }
    let pairs = payload.lines().nth(1).unwrap_or("");
    pairs.split_whitespace().any(|kv| kv == "agents=0")
}

/// The shell that dumps every saved layout: `\x1e<name>\t<mtime>\n<file>` per session directory.
pub const SAVED_SCRIPT: &str = r#"for f in "$HOME"/.cache/zellij/*/session_info/*/session-layout.kdl; do [ -f "$f" ] || continue; d=${f%/*}; printf '\036%s\t%s\n' "${d##*/}" "$(stat -c %Y "$f")"; cat "$f"; done"#;

/// The dump of `SAVED_SCRIPT` → what each session's layout holds. A name saved under two zellij
/// contract versions keeps the newer file.
pub fn parse_saved(out: &str) -> BTreeMap<String, Saved> {
    newest(out)
        .into_iter()
        .map(|(name, body)| (name, count_layout(&body)))
        .collect()
}

/// The tabs of each saved layout, in order, with the one that had focus — what the detail line
/// offers to land on.
pub fn parse_saved_tabs(out: &str) -> BTreeMap<String, Vec<(String, bool)>> {
    newest(out)
        .into_iter()
        .map(|(name, body)| (name, tab_names(&body)))
        .collect()
}

/// Each name's layout from the dump, the newer file when a name was saved twice.
fn newest(out: &str) -> BTreeMap<String, String> {
    let mut best: BTreeMap<String, (u64, String)> = BTreeMap::new();
    for chunk in out.split('\x1e').skip(1) {
        let (head, body) = chunk.split_once('\n').unwrap_or((chunk, ""));
        let Some((name, mtime)) = head.split_once('\t') else {
            continue;
        };
        let mtime: u64 = mtime.trim().parse().unwrap_or(0);
        if best.get(name).is_none_or(|(m, _)| mtime >= *m) {
            best.insert(name.to_string(), (mtime, body.to_string()));
        }
    }
    best.into_iter().map(|(k, (_, v))| (k, v)).collect()
}

/// The session's own tabs (`tab name="…"` one level under `layout`), not those of the templates
/// and swap layouts, which sit deeper. A tab without a name is `Tab #n`, as zellij calls it; the
/// flag is the serializer's `focus=true` — the tab the session was on.
pub fn tab_names(kdl: &str) -> Vec<(String, bool)> {
    let mut depth = 0usize;
    let mut names = Vec::new();
    for raw in kdl.lines() {
        let line = raw.trim();
        let word = line
            .split(|c: char| c.is_whitespace() || c == '{')
            .next()
            .unwrap_or("");
        if word == "tab" && depth == 1 {
            let name = line
                .split("name=\"")
                .nth(1)
                .and_then(|rest| rest.split('"').next())
                .map(str::to_string)
                .unwrap_or_else(|| format!("Tab #{}", names.len() + 1));
            names.push((name, line.contains("focus=true")));
        }
        depth += line.matches('{').count();
        depth = depth.saturating_sub(line.matches('}').count());
    }
    names
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Node {
    /// A node whose panes are not the session's (`new_tab_template`, swap layouts, …).
    Ignored,
    Tab,
    Pane {
        child_pane: bool,
        plugin: bool,
        claude: bool,
    },
    Other,
}

/// Tabs, terminal panes and `claude` panes in a serialized layout. A pane counts when it is a
/// leaf (no child panes) and not a plugin (the bars); `floating_panes` count like the others.
/// The serializer writes one node per line, which is all this reads.
pub fn count_layout(kdl: &str) -> Saved {
    let mut saved = Saved::default();
    let mut stack: Vec<Node> = Vec::new();
    for raw in kdl.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        if line == "}" {
            if let Some(Node::Pane {
                child_pane: false,
                plugin: false,
                claude,
            }) = stack.pop()
            {
                if counting(&stack) {
                    saved.panes += 1;
                    saved.agents += usize::from(claude);
                }
            }
            continue;
        }
        let word = line
            .split(|c: char| c.is_whitespace() || c == '{')
            .next()
            .unwrap_or("");
        let opens = line.ends_with('{');
        let ignored = stack.contains(&Node::Ignored);
        match word {
            "tab" if !ignored => {
                saved.tabs += 1;
            }
            "pane" => {
                if let Some(Node::Pane { child_pane, .. }) = stack.last_mut() {
                    *child_pane = true;
                }
            }
            "plugin" => {
                if let Some(Node::Pane { plugin, .. }) = stack.last_mut() {
                    *plugin = true;
                }
            }
            _ => {}
        }
        let claude = word == "pane" && runs_claude(line);
        if word == "pane" && !opens && counting(&stack) {
            saved.panes += 1;
            saved.agents += usize::from(claude);
        }
        if opens {
            stack.push(match word {
                "tab" if !ignored => Node::Tab,
                "pane" => Node::Pane {
                    child_pane: false,
                    plugin: false,
                    claude,
                },
                "new_tab_template"
                | "tab_template"
                | "pane_template"
                | "swap_tiled_layout"
                | "swap_floating_layout"
                | "default_tab_template" => Node::Ignored,
                _ => Node::Other,
            });
        }
    }
    saved
}

fn counting(stack: &[Node]) -> bool {
    stack.contains(&Node::Tab) && !stack.contains(&Node::Ignored)
}

fn runs_claude(line: &str) -> bool {
    let Some(rest) = line.split("command=\"").nth(1) else {
        return false;
    };
    let cmd = rest.split('"').next().unwrap_or("");
    cmd.rsplit('/').next() == Some("claude")
}

/// `5s`, `12m`, `3h`, `4d`, `2mo` — one unit, like a list wants it.
pub fn age(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m", s / 60),
        3600..=86_399 => format!("{}h", s / 3600),
        86_400..=2_591_999 => format!("{}d", s / 86_400),
        _ => format!("{}mo", s / 2_592_000),
    }
}

/// Cut to `width` characters, with `…` when something was cut.
pub fn fit(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        return s.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut out: String = s.chars().take(width - 1).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCHOOL: &str = r#"layout {
    cwd "/mnt/storage/biserstoilov/school"
    tab name="Tab #1" hide_floating_panes=true {
        pane size=1 borderless=true {
            plugin location="file:/x/lvim-zbar.wasm" {
                bar "hints"
            }
        }
        pane split_direction="vertical" {
            pane command="claude" size="50%" {
                args "--continue"
                start_suspended true
            }
            pane contents_file="initial_contents_1" size="50%"
        }
        pane size=1 borderless=true {
            plugin location="file:/x/lvim-zbar.wasm"
        }
    }
    tab name="Tab #2" hide_floating_panes=true {
        pane size=1 borderless=true {
            plugin location="file:/x/lvim-zbar.wasm" {
                bar "hints"
            }
        }
        pane contents_file="initial_contents_2"
        floating_panes {
            pane command="/usr/bin/claude" x=1 y=1
        }
        pane size=1 borderless=true {
            plugin location="file:/x/lvim-zbar.wasm"
        }
    }
    new_tab_template {
        pane size=1 borderless=true {
            plugin location="file:/x/lvim-zbar.wasm"
        }
        pane cwd="/mnt/storage/biserstoilov/school"
    }
    swap_tiled_layout name="vertical" {
        tab max_panes=5 {
            pane
            pane
        }
    }
}
"#;

    #[test]
    fn a_saved_layout_counts_its_tabs_its_terminal_panes_and_its_claude_panes() {
        assert_eq!(
            count_layout(SCHOOL),
            Saved {
                tabs: 2,
                panes: 4,
                agents: 2
            }
        );
    }

    #[test]
    fn a_saved_layout_names_its_own_tabs_and_not_the_templates() {
        let names =
            |kdl: &str| -> Vec<String> { tab_names(kdl).into_iter().map(|(n, _)| n).collect() };
        assert_eq!(names(SCHOOL), ["Tab #1", "Tab #2"]);
        let kdl = "layout {\n    tab {\n    }\n    tab name=\"work\" focus=true {\n    }\n}\n";
        assert_eq!(
            tab_names(kdl),
            [("Tab #1".to_string(), false), ("work".to_string(), true)]
        );
    }

    #[test]
    fn the_dump_keeps_the_newer_layout_of_a_name_saved_twice() {
        let dump = "\x1eSCHOOL\t100\nlayout {\n    tab {\n        pane\n    }\n}\n\
                    \x1eSCHOOL\t200\nlayout {\n    tab {\n        pane\n        pane\n    }\n    tab {\n        pane\n    }\n}\n\
                    \x1eGO\t5\nlayout {\n}\n";
        let saved = parse_saved(dump);
        assert_eq!(
            saved["SCHOOL"],
            Saved {
                tabs: 2,
                panes: 3,
                agents: 0
            }
        );
        assert_eq!(saved["GO"], Saved::default());
    }

    #[test]
    fn zclaude_lines_become_agents_per_session_and_the_bar_lines_are_skipped() {
        let payload = "2 agents · 1 busy\nagents=2 busy=1\nsession\tRUST\t2\t1\t0\t1\n";
        let a = parse_agents(payload);
        assert_eq!(a.len(), 1);
        assert_eq!(
            a["RUST"],
            Agents {
                total: 2,
                busy: 1,
                waiting: 0,
                done: 1
            }
        );
    }

    #[test]
    fn a_summary_without_session_lines_counts_only_when_it_says_no_agents() {
        assert!(has_per_session(
            "1 agent\nagents=1\nsession\tX\t1\t0\t0\t0\n"
        ));
        assert!(!has_per_session("4 agents\nagents=4 busy=2\n"));
        assert!(has_per_session("0 agents\nagents=0 busy=0\n"));
    }

    fn live(name: &str, current: bool) -> Row {
        Row {
            name: name.into(),
            kind: Kind::Live {
                tabs: 1,
                panes: 2,
                clients: 0,
                current,
            },
        }
    }
    fn dead(name: &str, secs: u64) -> Row {
        Row {
            name: name.into(),
            kind: Kind::Dead {
                age: Duration::from_secs(secs),
                saved: None,
            },
        }
    }

    #[test]
    fn this_session_comes_first_then_live_ones_then_the_saved_newest_first() {
        let mut rows = vec![
            dead("OLD", 900),
            live("LUA", false),
            dead("NEW", 10),
            live("RUST", true),
            live("Go", false),
        ];
        sort(&mut rows);
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["RUST", "Go", "LUA", "NEW", "OLD"]);
    }

    #[test]
    fn the_filter_matches_letters_in_order() {
        let rows = vec![
            live("SCHOOL", false),
            live("LVIM", false),
            live("LUA", false),
        ];
        let names: Vec<&str> = filter(&rows, "lv")
            .iter()
            .map(|r| r.name.as_str())
            .collect();
        assert_eq!(names, ["LVIM"]);
        assert_eq!(filter(&rows, "").len(), 3);
        assert_eq!(filter(&rows, "sl").len(), 1);
    }

    #[test]
    fn fit_cuts_by_characters() {
        assert_eq!(fit("Сесия", 10), "Сесия");
        assert_eq!(fit("Сесията", 4), "Сес…");
    }
}
