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
    /// SESSION › TAB › PANE: the session, its tabs (the chosen one marked), and on the line under
    /// them the chosen tab's panes (the chosen one marked).
    Tabs {
        name: String,
        tabs: Vec<String>,
        chosen: Option<usize>,
        panes: Vec<String>,
        pane: Option<usize>,
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
    /// Under the tabs line: what the selected session's chosen tab shows.
    pub preview: Option<&'a Preview>,
    /// zellij's own attention per live session: `(status word, how many, colour slot)`.
    pub attention: &'a std::collections::BTreeMap<String, Attention>,
}

/// What a session's chosen tab looks like: a title and its lines, styled as the pane draws them.
pub struct Preview {
    pub title: String,
    pub lines: Vec<String>,
}

/// zellij's attention for one session, already reduced to what the column shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attention {
    pub symbol: &'static str,
    pub word: &'static str,
    pub count: usize,
    pub tone: Tone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Waiting,
    Busy,
    Done,
    Muted,
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

    // Columns: ST, SESSION (what is left), TABS, PANES, CLIENTS, AGENTS, ATTENTION, LAST.
    let (w_st, w_tabs, w_panes, w_clients, w_agents, w_attn, w_last) = (3, 6, 7, 9, 18, 15, 10);
    let fixed = 2 + w_st + w_tabs + w_panes + w_clients + w_agents + w_attn + w_last;
    let w_name = cols.saturating_sub(fixed).clamp(8, 32);
    let header = format!(
        "  {}{}{}{}{}{}{}{}",
        pad("ST", w_st),
        pad("SESSION", w_name),
        pad("TABS", w_tabs),
        pad("PANES", w_panes),
        pad("CLIENTS", w_clients),
        pad("AGENTS", w_agents),
        pad("ATTENTION", w_attn),
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

    // Title, headings, the tabs line, the panes line and message/input are five lines that are
    // always there; the help is as many as it wraps to at this width. What is left is the list's
    // and the preview's: the list takes what it needs up to three fifths, the preview the rest.
    let room = rows.saturating_sub(5 + help_height(view, cols)).max(1);
    let needed = lines.len().max(1);
    let (body, preview_rows) = if view.preview.is_some() && room >= 10 {
        let list = needed.min(room - room * 2 / 5).max(3);
        (list, room - list)
    } else {
        (room, 0)
    };
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
                    (w_st, w_name, w_tabs, w_panes, w_clients, w_agents, w_attn, w_last),
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
    println!("{}", panes_line(view, cols));
    if let Some(preview) = view.preview.filter(|_| preview_rows > 0) {
        draw_preview(view, preview, preview_rows, cols);
    }
    footer(view, cols);
}

/// `  ── Tab #1 › nvim ───` and the last lines of the pane under it, in the pane's own colours.
/// Each line is cut by what is seen and ends with a full reset, so no colour leaks past it.
fn draw_preview(view: &View, preview: &Preview, rows: usize, cols: usize) {
    let pal = view.palette;
    let title = panel::fit(&preview.title, cols.saturating_sub(8));
    let rule = "─".repeat(cols.saturating_sub(title.chars().count() + 6));
    println!(
        "  {} {} {}",
        fg(pal.map(|p| p.muted), "──"),
        fg(pal.map(|p| p.title), &title),
        fg(pal.map(|p| p.muted), &rule)
    );
    let body = rows.saturating_sub(1);
    let lines = compact(trim_blank_tail(&preview.lines));
    let start = lines.len().saturating_sub(body);
    let mut drawn = 0;
    for line in &lines[start..] {
        println!("  {}", cut_ansi(line, cols.saturating_sub(2)));
        drawn += 1;
    }
    for _ in drawn..body {
        println!();
    }
}

/// A screen dump ends in the empty rows under the prompt; the preview shows the rows with text.
fn trim_blank_tail(lines: &[String]) -> &[String] {
    let end = lines
        .iter()
        .rposition(|l| !strip_ansi(l).trim().is_empty())
        .map_or(0, |i| i + 1);
    &lines[..end]
}

/// The screen with every run of hollow rows (spaces and frame lines only, `│        │`) cut to
/// one. A full-screen program (secmon's TUI, htop) fills its pane: its tabs and data at the top,
/// a key line at the bottom, and between them rows of empty frame. The preview shows the bottom
/// of the screen — right for a shell, where the latest output is — and of such a program it
/// showed the empty frame only. Without the hollow runs the top and the bottom fit together.
fn compact(lines: &[String]) -> Vec<&String> {
    let mut out: Vec<&String> = Vec::with_capacity(lines.len());
    let mut previous_hollow = false;
    for line in lines {
        let hollow = is_hollow(line);
        if !(hollow && previous_hollow) {
            out.push(line);
        }
        previous_hollow = hollow;
    }
    out
}

/// Nothing on the row but spaces and box-drawing characters (U+2500–U+257F).
fn is_hollow(line: &str) -> bool {
    strip_ansi(line)
        .chars()
        .all(|c| c.is_whitespace() || ('\u{2500}'..='\u{257f}').contains(&c))
}

/// Cut a **styled** line to `width` visible characters: the escapes are copied and not counted,
/// and a styled line ends in a full reset. (lvim-zclaude's `cut_ansi`.)
pub fn cut_ansi(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let pieces = pieces(text);
    let visible = pieces.iter().filter(|p| matches!(p, Piece::Char(_))).count();
    let styled = pieces.iter().any(|p| matches!(p, Piece::Escape(_)));
    let keep = if visible <= width { visible } else { width - 1 };
    let mut out = String::new();
    let mut seen = 0;
    for piece in &pieces {
        match piece {
            Piece::Escape(escape) => out.push_str(escape),
            Piece::Char(c) if seen < keep => {
                out.push(*c);
                seen += 1;
            }
            Piece::Char(_) => {}
        }
    }
    if visible > width {
        out.push('…');
    }
    if styled {
        out.push_str("\u{1b}[0m");
    }
    out
}

pub fn strip_ansi(text: &str) -> String {
    pieces(text)
        .into_iter()
        .filter_map(|p| match p {
            Piece::Char(c) => Some(c),
            Piece::Escape(_) => None,
        })
        .collect()
}

enum Piece {
    Escape(String),
    Char(char),
}

fn pieces(text: &str) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            // A carriage return or another control character would move the cursor.
            if !c.is_control() {
                out.push(Piece::Char(c));
            }
            continue;
        }
        let mut escape = String::from(c);
        match chars.next() {
            Some('[') => {
                escape.push('[');
                for d in chars.by_ref() {
                    escape.push(d);
                    if ('\u{40}'..='\u{7e}').contains(&d) {
                        break;
                    }
                }
            }
            Some(']') => {
                // An OSC (a hyperlink, a title) is dropped: it says nothing a preview shows.
                while let Some(d) = chars.next() {
                    if d == '\u{7}' {
                        break;
                    }
                    if d == '\u{1b}' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
                continue;
            }
            Some(d) => escape.push(d),
            None => {}
        }
        // Only colours and styles (`…m`) are kept: a cursor move would draw the line elsewhere.
        if escape.ends_with('m') {
            out.push(Piece::Escape(escape));
        }
    }
    out
}

/// `  RUST  Tab #1  [Tab #2]` — in the session-name yellow, the chosen tab on the selection
/// background. Tabs that do not fit are left out from the end, never the chosen one.
fn detail_line(view: &View, cols: usize) -> String {
    let pal = view.palette;
    let yellow = pal.map(|p| p.name);
    match &view.detail {
        Detail::Text(text) => fg(yellow, &panel::fit(&format!("  {text}"), cols)),
        Detail::Tabs { name, tabs, chosen, .. } => {
            let out = format!("  {BOLD}{}{UNBOLD}  ", fg(yellow, &panel::fit(name, 32)));
            let used = 4 + name.chars().count().min(32);
            out + &chips(pal, yellow, tabs, *chosen, Chosen::Background, used, cols)
        }
    }
}

/// `       › nvim  zsh  claude` — the chosen tab's panes, under the tabs and in the theme's blue;
/// the chosen one only in another colour (bold, the accent of the keys), no background — the
/// selection background is the tab's. Empty while there are none to show (a new-session row, a
/// live tab whose preview has not come yet).
fn panes_line(view: &View, cols: usize) -> String {
    let Detail::Tabs { name, panes, pane, .. } = &view.detail else { return String::new() };
    if panes.is_empty() {
        return String::new();
    }
    let pal = view.palette;
    let indent = 4 + name.chars().count().min(32);
    let lead = format!("{}› ", " ".repeat(indent.saturating_sub(2)));
    let out = fg(pal.map(|p| p.muted), &lead);
    let chosen = Chosen::Colour(pal.map(|p| p.key));
    out + &chips(pal, pal.map(|p| p.live), panes, *pane, chosen, indent, cols)
}

/// How the chosen name stands out: on the selection background (a tab), or in its own colour (a
/// pane).
#[derive(Clone, Copy)]
enum Chosen {
    Background,
    Colour(Option<PaletteColor>),
}

/// Names side by side, ` name ` each, the chosen one marked as `how` says. Names that do not fit
/// are left out from the end, never the chosen one.
fn chips(
    pal: Option<Palette>,
    colour: Option<PaletteColor>,
    names: &[String],
    chosen: Option<usize>,
    how: Chosen,
    mut used: usize,
    cols: usize,
) -> String {
    let mut out = String::new();
    for (i, name) in names.iter().enumerate() {
        let cell = format!(" {} ", panel::fit(name, 24));
        let width = cell.chars().count() + 1;
        if used + width > cols && chosen.is_none_or(|c| c < i) {
            out.push_str(&fg(pal.map(|p| p.muted), "…"));
            break;
        }
        used += width;
        if Some(i) == chosen {
            match (how, pal) {
                (Chosen::Colour(accent), Some(_)) => {
                    out.push_str(&format!("{BOLD}{}{UNBOLD}", fg(accent, &cell)))
                }
                (Chosen::Background, Some(p)) => out.push_str(&format!(
                    "{}{BOLD}{}{UNBOLD}{RESET_BG}",
                    ansi(p.selected, 48),
                    fg(colour, &cell)
                )),
                (_, None) => out.push_str(&format!("\u{1b}[7m{cell}\u{1b}[27m")),
            }
        } else {
            out.push_str(&fg(colour, &cell));
        }
        out.push(' ');
    }
    out
}

type Widths = (usize, usize, usize, usize, usize, usize, usize, usize);

fn item_line(view: &View, item: &Item, cols: usize, w: Widths) -> String {
    let pal = view.palette;
    let (w_st, w_name, w_tabs, w_panes, w_clients, w_agents, w_attn, w_last) = w;
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
    let (attn_text, attn_colour) = match view.attention.get(&row.name).filter(|_| row.is_live()) {
        Some(a) if a.count > 1 => (format!("{} {} {}", a.symbol, a.count, a.word), tone(pal, a.tone)),
        Some(a) => (format!("{} {}", a.symbol, a.word), tone(pal, a.tone)),
        None => ("—".to_string(), pal.map(|p| p.muted)),
    };
    let line = format!(
        "  {}{BOLD}{}{UNBOLD}{}{}{}{}{}{}",
        fg(glyph_colour, &pad(glyph, w_st)),
        fg(name_colour, &pad(&row.name, w_name)),
        fg(pal.map(|p| p.count), &pad(&tabs, w_tabs)),
        fg(pal.map(|p| p.count), &pad(&panes, w_panes)),
        fg(pal.map(|p| p.count), &pad(&clients, w_clients)),
        fg(agents_colour, &pad(&agents_text, w_agents)),
        fg(attn_colour, &pad(&attn_text, w_attn)),
        fg(last_colour, &pad(&last, w_last)),
    );
    let used = 2 + w_st + w_name + w_tabs + w_panes + w_clients + w_agents + w_attn + w_last;
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

fn tone(pal: Option<Palette>, tone: Tone) -> Option<PaletteColor> {
    pal.map(|p| match tone {
        Tone::Waiting => p.waiting,
        Tone::Busy => p.busy,
        Tone::Done => p.done,
        Tone::Muted => p.muted,
    })
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
    // The last line is printed without a newline: one more would scroll the pane by a line.
    let lines = help_lines(pal, help_keys(view), cols);
    let last = lines.len() - 1;
    for (index, line) in lines.iter().enumerate() {
        if index == last {
            print!("{line}");
        } else {
            println!("{line}");
        }
    }
}

fn help_keys(view: &View) -> &'static [(&'static str, &'static str)] {
    if view.input.is_some() {
        &[("Enter", "confirm"), ("Esc", "cancel")]
    } else {
        &[
            ("↑↓", "move"),
            ("Enter", "open"),
            ("←→", "tab"),
            ("Tab", "pane"),
            ("Ctrl o", "agents"),
            ("Ctrl n", "new"),
            ("Del", "kill/delete"),
            ("Ctrl r", "rename"),
            ("Ctrl a", "save"),
            ("Ctrl x", "detach others"),
            ("Esc", "close"),
        ]
    }
}

/// How many lines the help takes at this width; `draw` leaves them out of the list.
fn help_height(view: &View, cols: usize) -> usize {
    help_lines(None, help_keys(view), cols).len()
}

/// `Help: <Enter> - open, <←→> - tab, …`, **wrapped** to the width: a key and its word are never
/// split, and the next line starts under the first key. Nothing is dropped — a key that is not
/// on the screen is a key nobody learns.
fn help_lines(pal: Option<Palette>, keys: &[(&str, &str)], cols: usize) -> Vec<String> {
    const LEAD: &str = "  Help: ";
    let lead = LEAD.chars().count();
    let room = cols.saturating_sub(lead).max(1);
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    let mut width = 0usize;
    for (index, (key, what)) in keys.iter().enumerate() {
        let comma = index + 1 < keys.len();
        let seen = key.chars().count() + 5 + what.chars().count() + usize::from(comma);
        if width > 0 && width + 1 + seen > room {
            lines.push(std::mem::take(&mut line));
            width = 0;
        }
        if width > 0 {
            line.push(' ');
            width += 1;
        }
        line.push_str(&keycap(pal, key, what));
        if comma {
            line.push_str(&fg(pal.map(|p| p.muted), ","));
        }
        width += seen;
    }
    lines.push(line);
    lines
        .into_iter()
        .enumerate()
        .map(|(index, keys)| {
            if index == 0 {
                format!("  {}{keys}", fg(pal.map(|p| p.heading), "Help: "))
            } else {
                format!("{}{keys}", " ".repeat(lead))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The help wraps instead of losing its words: every key and word is on the screen and no
    /// line is wider than the pane.
    #[test]
    fn help_wraps_to_the_width() {
        let keys = [("↑↓", "move"), ("Enter", "open"), ("Ctrl x", "detach others"), ("Esc", "close")];
        let lines = help_lines(None, &keys, 40);
        assert!(lines.len() > 1, "{lines:?}");
        assert!(lines.iter().all(|line| line.chars().count() <= 40), "{lines:?}");
        let all = lines.join("\n");
        for (key, what) in keys {
            assert!(all.contains(&format!("<{key}> - {what}")), "{all}");
        }
        assert_eq!(help_lines(None, &keys, 200).len(), 1);
    }

    /// A pane's screen keeps its colours but loses what would move the cursor, and is cut by
    /// what is seen.
    #[test]
    fn a_preview_line_keeps_colour_and_drops_cursor_moves() {
        let line = "\u{1b}[31mred\u{1b}[0m \u{1b}[2Kplain\r\u{1b}]8;;http://x\u{1b}\\link";
        assert_eq!(strip_ansi(line), "red plainlink");
        let cut = cut_ansi(line, 6);
        assert_eq!(strip_ansi(&cut), "red p…");
        assert!(cut.starts_with("\u{1b}[31m") && cut.ends_with("\u{1b}[0m"));
        assert!(!cut.contains("[2K"));
    }

    #[test]
    fn the_blank_rows_under_a_prompt_are_not_previewed() {
        let lines: Vec<String> = ["$ ls", "a b", "", "\u{1b}[0m  "].map(String::from).to_vec();
        assert_eq!(trim_blank_tail(&lines).len(), 2);
    }

    #[test]
    fn the_empty_frame_of_a_full_screen_program_is_cut_to_one_row() {
        let lines: Vec<String> = [
            "┌ Alerts ─┐",
            "│ 21s ago │",
            "│         │",
            "\u{1b}[2m│         │\u{1b}[0m",
            "│         │",
            "└─────────┘",
            "[q] quit",
        ]
        .map(String::from)
        .to_vec();
        let shown: Vec<&str> = compact(&lines).into_iter().map(String::as_str).collect();
        assert_eq!(shown, ["┌ Alerts ─┐", "│ 21s ago │", "│         │", "[q] quit"]);
    }
}
