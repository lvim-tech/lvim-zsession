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

The plugin runs in the background, never intercepts the keyboard and renders nothing.

## Install

With [lvim-zpm](https://github.com/lvim-tech/lvim-zpm), add the repo to `~/.config/zellij/zpm.kdl`:

```kdl
plugins {
    "lvim-tech/lvim-zsession"
}
```

The shipped `zpm.kdl` carries the permissions and the keybind (tmux mode: `Ctrl s`, then `s` =
save now). Preload the plugin so it is always running:

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
save is broadcast as a `notice` pipe for status bars such as lvim-zbar).

## Building

```sh
cargo build --release --target wasm32-wasip1
```

The built `lvim-zsession.wasm` is shipped in the repo, so building is only needed after changes.
