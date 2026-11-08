// Drawing the session panel — in lvim-zclaude's look: theme colours only, a title line, column
// headings, a heading per group, the selected row on the theme's selection background, a detail
// line, and zellij's own `Help: <key> - what` line at the foot.
//
// Colour goes on a cell only after it has been padded, and the closing code is a foreground reset
// (`39`), not a full reset: a full reset would drop the selected row's background, and colouring
// before padding would count escape bytes as characters.

use crate::panel::{self, Agents, Kind, Row};
use zellij_tile::prelude::{PaletteColor, Styling};

const BOLD: &str = "\u{1b}[1m";
const UNBOLD: &str = "\u{1b}[22m";
const RESET_FG: &str = "\u{1b}[39m";
const RESET_BG: &str = "\u{1b}[49m";

/// lvim-zclaude's palette: every colour a slot of the active zellij theme. The accents come from
/// `multiplayer_user_colors` (the raw palette the lvim theme generator keeps there); the neutral
/// grey is the theme's own foreground.
#[derive(Clone, Copy)]
pub struct Palette {
    pub title: PaletteColor,
    pub muted: PaletteColor,
    pub heading: PaletteColor,
    pub key: PaletteColor,
    pub selected: PaletteColor,
    pub name: PaletteColor,
    pub here: PaletteColor,
    pub live: PaletteColor,
    pub busy: PaletteColor,
    pub waiting: PaletteColor,
    pub done: PaletteColor,
    pub count: PaletteColor,
}

impl Palette {
    pub fn from_style(style: &Styling) -> Palette {
        let p = style.multiplayer_user_colors;
        Palette {
            title: p.player_2,
            muted: style.text_unselected.base,
            heading: p.player_1,
            key: p.player_4,
            selected: style.text_selected.background,
            name: p.player_8,
            here: p.player_3,
            live: p.player_2,
            busy: p.player_3,
            waiting: p.player_9,
            done: p.player_7,
            count: p.player_6,
        }
    }
}

fn ansi(color: PaletteColor, layer: u8) -> String {
    match color {
        PaletteColor::Rgb((r, g, b)) => format!("\u{1b}[{layer};2;{r};{g};{b}m"),
        PaletteColor::EightBit(n) => format!("\u{1b}[{layer};5;{n}m"),
    }
}

fn fg(color: Option<PaletteColor>, text: &str) -> String {
    match color {
        Some(color) => format!("{}{text}{RESET_FG}", ansi(color, 38)),
        None => text.to_string(),
    }
}

fn pad(text: &str, width: usize) -> String {
    let text = panel::fit(text, width);
    let count = text.chars().count();
    format!("{text}{}", " ".repeat(width.saturating_sub(count)))
}

/// One entry of the list: a session, or the row that makes a new one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Session(Row),
    /// `+ new session` — with the name typed in the filter, or none (zellij picks one).
    New(Option<String>),
}

/// The line under the list: the selected session and its tabs, one of them chosen with ←/→ —
/// or, for the new-session row, what Enter will do.
pub enum Detail {
    Tabs {
        name: String,
        tabs: Vec<String>,
        chosen: Option<usize>,
    },
    Text(String),
}

pub struct View<'a> {
    pub items: &'a [Item],
    pub cursor: usize,
    pub filter: &'a str,
    pub agents: &'a std::collections::BTreeMap<String, Agents>,
    pub live: usize,
    pub saved: usize,
    pub palette: Option<Palette>,
    /// Under the list, for the selected row.
    pub detail: Detail,
    /// A question waiting for `y`.
    pub prompt: Option<&'a str>,
    /// The input line: its label and what is typed (rename, new session).
    pub input: Option<(&'a str, &'a str)>,
    /// What the last action did.
    pub message: Option<&'a str>,
}

/// A line in the list: a group heading or an item (with its index into `items`).
enum Line<'a> {
    Heading(&'static str, String),
    Item(usize, &'a Item),
}

pub fn draw(view: &View, rows: usize, cols: usize) {
    let pal = view.palette;

    // Title: what this is, what is typed, the counts.
    let counts = format!("{} running · {} saved", view.live, view.saved);
    let filter = if view.filter.is_empty() {
        String::new()
    } else {
        format!("filter: {}", view.filter)
    };
    let left = "lvim-zsession · sessions";
    let gap = cols.saturating_sub(left.len() + filter.chars().count() + counts.chars().count() + 2);
    println!(
        "{BOLD}{}{UNBOLD} {}{}{}",
        fg(pal.map(|p| p.title), left),
        fg(pal.map(|p| p.name), &filter),
        " ".repeat(gap + 1),
        fg(pal.map(|p| p.muted), &counts)
    );

    // Columns: ST, SESSION (what is left), TABS, PANES, CLIENTS, AGENTS, LAST.
    let (w_st, w_tabs, w_panes, w_clients, w_agents, w_last) = (3, 6, 7, 9, 18, 10);
    let fixed = 2 + w_st + w_tabs + w_panes + w_clients + w_agents + w_last;
    let w_name = cols.saturating_sub(fixed).clamp(8, 32);
    let header = format!(
        "  {}{}{}{}{}{}{}",
        pad("ST", w_st),
        pad("SESSION", w_name),
        pad("TABS", w_tabs),
        pad("PANES", w_panes),
        pad("CLIENTS", w_clients),
        pad("AGENTS", w_agents),
        pad("LAST", w_last)
    );
    println!("{}", fg(pal.map(|p| p.heading), &panel::fit(&header, cols)));

    // The list with its two headings.
    let mut lines: Vec<Line> = Vec::new();
    let mut last_group: Option<bool> = None;
    for (i, item) in view.items.iter().enumerate() {
        if let Item::Session(row) = item {
            let live = row.is_live();
            if last_group != Some(live) {
                last_group = Some(live);
                let (name, what) = if live {
                    let agents: usize = view
                        .items
                        .iter()
                        .filter_map(|it| match it {
                            Item::Session(r) if r.is_live() => {
                                view.agents.get(&r.name).map(|a| a.total)
                            }
                            _ => None,
                        })
                        .sum();
                    (
                        "RUNNING",
                        format!(
                            "{} · {}",
                            plural(view.live, "session"),
                            plural(agents, "agent")
                        ),
                    )
                } else {
                    (
                        "SAVED",
                        format!("{} · Enter brings one back", plural(view.saved, "session")),
                    )
                };
                lines.push(Line::Heading(name, what));
            }
        }
        lines.push(Line::Item(i, item));
    }

    // Title, headings, detail, message/input, help: five lines that are always there.
    let body = rows.saturating_sub(5).max(1);
    let cursor_line = lines
        .iter()
        .position(|l| matches!(l, Line::Item(i, _) if *i == view.cursor))
        .unwrap_or(0);
    let first = cursor_line.saturating_sub(body.saturating_sub(1));
    let mut drawn = 0;
    for line in lines.iter().skip(first).take(body) {
        drawn += 1;
        match line {
            Line::Heading(name, what) => {
                let what = panel::fit(what, cols.saturating_sub(name.len() + 4));
                println!(
                    "{} {BOLD}{}{UNBOLD} {}",
                    fg(pal.map(|p| p.title), "⏷"),
                    fg(pal.map(|p| p.title), name),
                    fg(pal.map(|p| p.muted), &what)
                );
            }
            Line::Item(i, item) => {
                let line = item_line(
                    view,
                    item,
                    cols,
                    (w_st, w_name, w_tabs, w_panes, w_clients, w_agents, w_last),
                );
                let selected = *i == view.cursor;
                match (selected, pal) {
                    (true, Some(p)) => println!("{}{line}{RESET_BG}", ansi(p.selected, 48)),
                    (true, None) => println!("\u{1b}[7m{line}\u{1b}[27m"),
                    _ => println!("{line}"),
                }
            }
        }
    }
    if view.items.is_empty() {
        drawn += 1;
        println!("  {}", fg(pal.map(|p| p.muted), "no session matches"));
    }
    for _ in drawn..body {
        println!();
    }

    println!("{}", detail_line(view, cols));
    footer(view, cols);
}

/// `  RUST  Tab #1  [Tab #2]` — in the session-name yellow, the chosen tab on the selection
/// background. Tabs that do not fit are left out from the end, never the chosen one.
fn detail_line(view: &View, cols: usize) -> String {
    let pal = view.palette;
    let yellow = pal.map(|p| p.name);
    match &view.detail {
        Detail::Text(text) => fg(yellow, &panel::fit(&format!("  {text}"), cols)),
        Detail::Tabs { name, tabs, chosen } => {
            let mut out = format!("  {BOLD}{}{UNBOLD}  ", fg(yellow, &panel::fit(name, 32)));
            let mut used = 4 + name.chars().count().min(32);
            for (i, tab) in tabs.iter().enumerate() {
                let cell = format!(" {} ", panel::fit(tab, 24));
                let width = cell.chars().count() + 1;
                if used + width > cols && chosen.is_none_or(|c| c < i) {
                    out.push_str(&fg(pal.map(|p| p.muted), "…"));
                    break;
                }
                used += width;
                if Some(i) == *chosen {
                    match pal {
                        Some(p) => out.push_str(&format!(
                            "{}{BOLD}{}{UNBOLD}{RESET_BG}",
                            ansi(p.selected, 48),
                            fg(yellow, &cell)
                        )),
                        None => out.push_str(&format!("\u{1b}[7m{cell}\u{1b}[27m")),
                    }
                } else {
                    out.push_str(&fg(yellow, &cell));
                }
                out.push(' ');
            }
            out
        }
    }
}

type Widths = (usize, usize, usize, usize, usize, usize, usize);

fn item_line(view: &View, item: &Item, cols: usize, w: Widths) -> String {
    let pal = view.palette;
    let (w_st, w_name, w_tabs, w_panes, w_clients, w_agents, w_last) = w;
    let row = match item {
        Item::New(name) => {
            let text = match name {
                Some(n) => format!("+ new session «{n}»"),
                None => "+ new session".to_string(),
            };
            let line = format!("  {}", pad(&text, cols.saturating_sub(2)));
            return fg(pal.map(|p| p.key), &line);
        }
        Item::Session(row) => row,
    };
    let (glyph, glyph_colour) = match row.kind {
        Kind::Live { current: true, .. } => ("●", pal.map(|p| p.here)),
        Kind::Live { .. } => ("●", pal.map(|p| p.live)),
        Kind::Dead { .. } => ("○", pal.map(|p| p.muted)),
    };
    let agents = view.agents.get(&row.name);
    let (tabs, panes, clients, agents_text, agents_colour, last, last_colour) = match &row.kind {
        Kind::Live {
            tabs,
            panes,
            clients,
            current,
        } => {
            let (text, colour) = agents_cell(agents, pal);
            let (last, colour_last) = if *current {
                ("here".to_string(), pal.map(|p| p.here))
            } else if *clients > 0 {
                ("attached".to_string(), pal.map(|p| p.live))
            } else {
                ("detached".to_string(), pal.map(|p| p.muted))
            };
            (
                tabs.to_string(),
                panes.to_string(),
                clients.to_string(),
                text,
                colour,
                last,
                colour_last,
            )
        }
        Kind::Dead { age, saved } => {
            let (tabs, panes, claude) = match saved {
                Some(s) => (s.tabs.to_string(), s.panes.to_string(), s.agents),
                None => ("·".to_string(), "·".to_string(), 0),
            };
            let text = if claude > 0 {
                format!("{claude} saved")
            } else {
                "—".to_string()
            };
            (
                tabs,
                panes,
                "—".to_string(),
                text,
                pal.map(|p| p.muted),
                panel::age(*age) + " ago",
                pal.map(|p| p.muted),
            )
        }
    };
    let name_colour = if row.is_live() {
        pal.map(|p| p.name)
    } else {
        pal.map(|p| p.muted)
    };
    let line = format!(
        "  {}{BOLD}{}{UNBOLD}{}{}{}{}{}",
        fg(glyph_colour, &pad(glyph, w_st)),
        fg(name_colour, &pad(&row.name, w_name)),
        fg(pal.map(|p| p.count), &pad(&tabs, w_tabs)),
        fg(pal.map(|p| p.count), &pad(&panes, w_panes)),
        fg(pal.map(|p| p.count), &pad(&clients, w_clients)),
        fg(agents_colour, &pad(&agents_text, w_agents)),
        fg(last_colour, &pad(&last, w_last)),
    );
    let used = 2 + w_st + w_name + w_tabs + w_panes + w_clients + w_agents + w_last;
    format!("{line}{}", " ".repeat(cols.saturating_sub(used)))
}

/// `2 · 1 waiting` — the agents of a live session and the one state that matters most.
fn agents_cell(agents: Option<&Agents>, pal: Option<Palette>) -> (String, Option<PaletteColor>) {
    let Some(a) = agents.filter(|a| a.total > 0) else {
        return ("—".to_string(), pal.map(|p| p.muted));
    };
    let (n, word, colour) = if a.waiting > 0 {
        (a.waiting, "waiting", pal.map(|p| p.waiting))
    } else if a.busy > 0 {
        (a.busy, "busy", pal.map(|p| p.busy))
    } else if a.done > 0 {
        (a.done, "done", pal.map(|p| p.done))
    } else {
        (a.total, "idle", pal.map(|p| p.muted))
    };
    // One agent: its state. More: how many, and how many in the state that matters most.
    let text = if a.total == 1 {
        format!("1 {word}")
    } else {
        format!("{} · {n} {word}", a.total)
    };
    (text, colour)
}

fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

fn keycap(pal: Option<Palette>, key: &str, what: &str) -> String {
    format!(
        "{}{}",
        fg(pal.map(|p| p.key), &format!("<{key}>")),
        fg(pal.map(|p| p.muted), &format!(" - {what}"))
    )
}

fn footer(view: &View, cols: usize) {
    let pal = view.palette;
    if let Some(prompt) = view.prompt {
        let room = cols.saturating_sub(36);
        println!(
            "  {} {}",
            fg(pal.map(|p| p.waiting), &panel::fit(prompt, room)),
            keycap(pal, "y", "yes · anything else cancels")
        );
    } else if let Some((label, input)) = view.input {
        let room = cols.saturating_sub(label.chars().count() + 4);
        println!(
            "  {}{}{}",
            fg(pal.map(|p| p.key), label),
            fg(pal.map(|p| p.name), &panel::fit(input, room)),
            fg(pal.map(|p| p.key), "▏")
        );
    } else {
        let msg = view.message.unwrap_or("");
        println!(
            "{}",
            fg(pal.map(|p| p.done), &panel::fit(&format!("  {msg}"), cols))
        );
    }
    let keys: &[(&str, &str)] = if view.input.is_some() {
        &[("Enter", "confirm"), ("Esc", "cancel")]
    } else {
        &[
            ("↑↓", "move"),
            ("Enter", "open"),
            ("←→", "tab"),
            ("Ctrl n", "new"),
            ("Del", "kill/delete"),
            ("Ctrl r", "rename"),
            ("Ctrl a", "save"),
            ("Ctrl x", "detach others"),
            ("Esc", "close"),
        ]
    };
    let plain: usize = 6 + keys
        .iter()
        .map(|(k, w)| k.chars().count() + 6 + w.chars().count())
        .sum::<usize>();
    let line = if plain + 2 <= cols {
        let caps: Vec<String> = keys.iter().map(|(k, w)| keycap(pal, k, w)).collect();
        format!(
            "{}{}",
            fg(pal.map(|p| p.heading), "Help: "),
            caps.join(&fg(pal.map(|p| p.muted), ", "))
        )
    } else {
        let short: Vec<String> = keys
            .iter()
            .map(|(k, _)| fg(pal.map(|p| p.key), &format!("<{k}>")))
            .collect();
        short.join(" ")
    };
    print!("  {line}");
}
