// The session panel: zellij's session-manager, with what it leaves out.
//
// The built-in list shows tabs, panes and clients only for live sessions — a resurrectable one is
// a name and an age, because `SessionUpdate` gives no more. zellij reads its saved layout for us
// (`get_saved_session_preview` → `SavedSessionPreview`), and that says which tabs and panes it had
// and which of them ran `claude`. The agents of a live session come from
// lvim-zclaude, which counts them per zellij session in its summary pipe (lines 3+).
//
// Everything here is plain data and text; main.rs does the zellij calls.

use std::collections::BTreeMap;
use std::time::Duration;
use zellij_tile::prelude::{AttentionStatus, GlobalAttention, SavedSessionPreview, SavedTabPreview};

use crate::ui::{Attention, Tone};

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

/// What a saved session brings back, counted from zellij's own reading of its layout.
pub fn saved_counts(preview: &SavedSessionPreview) -> Saved {
    let panes = preview.tabs.iter().flat_map(|t| t.panes.iter());
    Saved {
        tabs: preview.tabs.len(),
        panes: panes.clone().count(),
        agents: panes.filter(|p| p.command.as_deref().is_some_and(runs_claude)).count(),
    }
}

/// A saved session's tabs in order, and the one it was left on.
pub fn saved_tabs(preview: &SavedSessionPreview) -> Vec<(String, bool)> {
    preview
        .tabs
        .iter()
        .map(|t| (t.name.clone(), t.focused))
        .collect()
}

/// A saved pane by what it runs: its title, its command, or `shell` in its folder.
pub fn saved_pane_title(pane: &zellij_tile::prelude::SavedPanePreview) -> String {
    pane.title
        .clone()
        .filter(|t| !t.trim().is_empty())
        .or_else(|| pane.command.clone())
        .unwrap_or_else(|| "shell".to_string())
}

/// What a saved tab brings back: one line per pane (`▸` on the chosen one) with its command and
/// folder, then the chosen pane's last screen when zellij kept it.
pub fn saved_tab_lines(tab: &SavedTabPreview, chosen: usize) -> Vec<String> {
    let mut lines: Vec<String> = tab
        .panes
        .iter()
        .enumerate()
        .map(|(i, pane)| {
            let mark = if i == chosen { "▸" } else { " " };
            let what = match &pane.command {
                Some(command) => format!("$ {command}"),
                None => "shell".to_string(),
            };
            match &pane.cwd {
                Some(cwd) => format!("{mark} {what}   {cwd}"),
                None => format!("{mark} {what}"),
            }
        })
        .collect();
    if let Some(contents) = tab.panes.get(chosen).and_then(|p| p.contents.as_ref()) {
        lines.push(String::new());
        lines.extend(contents.lines().map(str::to_string));
    }
    lines
}

/// zellij's attention per session, reduced to the one state the column shows: what needs a
/// person first, then what failed, what finished, what works. Muted and idle sessions show none.
pub fn attention(global: &GlobalAttention) -> BTreeMap<String, Attention> {
    global
        .sessions
        .iter()
        .filter(|s| !s.attention.muted)
        .filter_map(|s| {
            let c = &s.attention.counts;
            let (symbol, word, count, tone) = match s.attention.status {
                AttentionStatus::NeedsYou => ("●", "needs you", c.needs_you, Tone::Waiting),
                AttentionStatus::Failed => ("✗", "failed", c.failed, Tone::Waiting),
                AttentionStatus::Done => ("✓", "done", c.done, Tone::Done),
                AttentionStatus::Working => ("◐", "working", c.working, Tone::Busy),
                AttentionStatus::NewOutput => ("•", "new output", c.new_output, Tone::Muted),
                AttentionStatus::Quiet | AttentionStatus::Idle => return None,
            };
            Some((
                s.name.clone(),
                Attention {
                    symbol,
                    word,
                    count,
                    tone,
                },
            ))
        })
        .collect()
}

/// `claude --resume …`, `/usr/bin/claude` — a pane that comes back as an agent.
fn runs_claude(command: &str) -> bool {
    let program = command.split_whitespace().next().unwrap_or("");
    program.rsplit('/').next() == Some("claude")
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

    use zellij_tile::prelude::{SavedPanePreview, SavedTabPreview};

    fn pane(command: Option<&str>, focused: bool) -> SavedPanePreview {
        SavedPanePreview {
            command: command.map(str::to_string),
            focused,
            ..Default::default()
        }
    }

    #[test]
    fn a_saved_session_counts_its_tabs_panes_and_the_claude_panes() {
        let preview = SavedSessionPreview {
            session_name: "SCHOOL".into(),
            tabs: vec![
                SavedTabPreview {
                    name: "Tab #1".into(),
                    focused: false,
                    panes: vec![pane(Some("claude --continue"), true), pane(None, false)],
                },
                SavedTabPreview {
                    name: "work".into(),
                    focused: true,
                    panes: vec![pane(Some("/usr/bin/claude"), false), pane(Some("claudette"), false)],
                },
            ],
            ..Default::default()
        };
        assert_eq!(
            saved_counts(&preview),
            Saved {
                tabs: 2,
                panes: 4,
                agents: 2
            }
        );
        assert_eq!(
            saved_tabs(&preview),
            [("Tab #1".to_string(), false), ("work".to_string(), true)]
        );
    }

    #[test]
    fn zellij_attention_shows_the_state_that_needs_a_person_first() {
        use zellij_tile::prelude::{AttentionCounts, GlobalSessionAttention, SessionAttention};
        let session = |name: &str, status, muted, counts| GlobalSessionAttention {
            name: name.into(),
            is_current: false,
            attention: SessionAttention {
                status,
                counts,
                muted,
                top: None,
                items: vec![],
            },
        };
        let global = GlobalAttention {
            sessions: vec![
                session("A", AttentionStatus::NeedsYou, false, AttentionCounts { needs_you: 2, ..Default::default() }),
                session("B", AttentionStatus::Working, true, AttentionCounts::default()),
                session("C", AttentionStatus::Idle, false, AttentionCounts::default()),
            ],
            ..Default::default()
        };
        let shown = attention(&global);
        assert_eq!(shown.len(), 1);
        assert_eq!(shown["A"].word, "needs you");
        assert_eq!(shown["A"].count, 2);
    }

    #[test]
    fn a_saved_tab_lists_its_panes_and_marks_the_chosen_one() {
        let tab = SavedTabPreview {
            name: "t".into(),
            focused: true,
            panes: vec![
                SavedPanePreview { command: Some("nvim".into()), cwd: Some("/p".into()), ..Default::default() },
                SavedPanePreview { contents: Some("last line".into()), ..Default::default() },
            ],
        };
        assert_eq!(saved_tab_lines(&tab, 0), ["▸ $ nvim   /p", "  shell"]);
        assert_eq!(saved_tab_lines(&tab, 1), ["  $ nvim   /p", "▸ shell", "", "last line"]);
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
