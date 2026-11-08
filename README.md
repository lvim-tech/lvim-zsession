# lvim-zsession

A small companion to [zellij](https://zellij.dev)'s own session resurrection.

zellij serializes your sessions (`session_serialization true`) and brings them back by name after
a logout or reboot. A pane that was running a program (nvim, claude, ...) comes back as a
*command pane*: press ENTER and the program runs again. The wart: there is no shell underneath
it, so when you later quit the program you are left with a dead pane — a blank screen after a
TUI drops its alternate screen — and no prompt to start the next thing from.

This plugin fixes exactly that, live:

- When a command pane's program exits, the pane is replaced **in place** by an interactive
  shell, in the program's own directory. You land at a prompt where the program was.
- The shell is spawned the way zellij spawns its own panes (bare default shell), so the
  serializer keeps treating it as a plain pane and whatever you run in it is resurrected
  natively next time.
- Quitting that shell (Ctrl-D) closes the pane like any terminal. Panes that were resurrected
  but not started yet ("Waiting to run") are left alone.

It also answers a `save` pipe with `save_session()`: serialize the session to disk **right
now** (e.g. before logging out) — zellij has no keybind action for that.

That instance runs in the background, never intercepts the keyboard and renders nothing.

## The session panel

Loaded a second time with `panel "on"` in its configuration, the same wasm is a floating
session list — zellij's session-manager with what it leaves out:

- **live sessions**: tabs, panes, connected clients, and the agents running in them with what
  they are doing (busy / waiting / done) — from
  [lvim-zclaude](https://github.com/lvim-tech/lvim-zclaude)'s summary pipe;
- **saved (resurrectable) sessions**: zellij gives only a name and an age; the panel reads the
  saved layout (`~/.cache/zellij/*/session_info/<name>/session-layout.kdl`) and shows its tabs,
  terminal panes and the `claude` panes that come back on resurrection.

It is drawn like lvim-zclaude's board — the same size (80%×70%, `x`/`y`/`width`/`height` in the
configuration change it), colours taken only from the active zellij theme, columns ST · SESSION ·
TABS · PANES · CLIENTS · AGENTS · LAST under a RUNNING and a SAVED heading, a detail line for the
selected row and zellij's `Help:` line at the foot. That line shows the selected session's tabs
(a saved session's from its layout) in the theme's yellow: `←`/`→` choose one, and `Enter` goes to
the session with that tab focused (in this session: just that tab).

Keys: type to filter (letters in order), `↑`/`↓` (`Tab`, `Ctrl j`/`Ctrl k`), `Enter` open or
resurrect, `Ctrl n` new session (asks for a name; empty lets zellij pick one) — or `Enter` on the
last row, `+ new session «what you typed»`, `Del` kill a live session / delete a saved one (asks
once more), `Ctrl r` rename this session, `Ctrl a` save this session to disk now, `Ctrl x`
disconnect other clients, `Esc` close.

```kdl
tmux {
    bind "s" {
        LaunchOrFocusPlugin "file:/home/you/.config/zellij/zpm/plugins/lvim-zsession/lvim-zsession.wasm" {
            floating true
            move_to_focused_tab true
            panel "on"
        }
        SwitchToMode "normal"
    }
}
```

The bind lives in `config.kdl`, not in `zpm.kdl`: a `LaunchOrFocusPlugin` applied live through
`reconfigure()` resolves the `file:` path against the pane's directory (zellij 0.46) and fails.
The panel instance does none of the background work, so the two never replace the same pane.

## Install

With [lvim-zpm](https://github.com/lvim-tech/lvim-zpm), add the repo to `~/.config/zellij/zpm.kdl`:

```kdl
plugins {
    "lvim-tech/lvim-zsession"
}
```

The shipped `zpm.kdl` carries the permissions; the panel's keybind goes into `config.kdl` (see
above), and a `save` pipe (`MessagePlugin … { name "save"; }`) still saves without the panel.
Preload the plugin so it is always running:

```kdl
load_plugins {
    "file:/home/you/.config/zellij/zpm/plugins/lvim-zsession/lvim-zsession.wasm"
}
```

Keep `session_serialization true` (the default). `serialization_interval` is in milliseconds
and defaults to 60000 — a session created and abandoned within a minute is lost; `5000` is
cheap. Optional plugin configuration: `shell "zsh"` if your `default_shell` differs.

Permissions: `ReadApplicationState`, `ChangeApplicationState`, `RunCommands` (zellij gates
`OpenCommandPaneInPlaceOfPaneId` behind it), `MessageAndLaunchOtherPlugins` (the outcome of a
save is broadcast as a `notice` pipe for status bars such as lvim-zbar; the panel asks
lvim-zclaude for its summary), `ReadCliPipes` (lvim-zclaude answers with `zellij pipe`).

## Building

```sh
cargo build --release --target wasm32-wasip1
```

The built `lvim-zsession.wasm` is shipped in the repo, so building is only needed after changes.
