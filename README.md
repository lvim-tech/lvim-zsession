# lvim-zsession

Session utilities for [zellij](https://zellij.dev): **save the session to disk from a keybind**.

zellij serializes sessions on an interval, and `zellij action save-session` exists on the CLI —
but there is no `SaveSession` keybind **action**, so the only interactive way to snapshot a
session for later resurrection is a detour. This plugin closes the gap: a key pipes `save` to it,
the session is serialized immediately — no popup, no transient pane. It is stateless, launches
lazily on the first press, never intercepts the keyboard and renders nothing.

## Install

With [lvim-zpm](https://github.com/lvim-tech/lvim-zpm), add the repo to `~/.config/zellij/zpm.kdl`:

```kdl
plugins {
    "lvim-tech/lvim-zsession"
}
```

Done — the shipped `zpm.kdl` here carries the keybind (tmux mode: `Ctrl s`, then `s`) and the
permission; the manager applies both on every session start.

Manual install instead: copy `lvim-zsession.wasm` to `~/.config/zellij/plugins/` and bind a key
yourself:

```kdl
bind "s" { MessagePlugin "file:~/.config/zellij/plugins/lvim-zsession.wasm" { name "save"; }; SwitchToMode "normal"; }
```

The save's outcome is written to the zellij log (`lvim-zsession: session saved`).

## Building

```sh
cargo build --release --target wasm32-wasip1
```

The built `lvim-zsession.wasm` is shipped in the repo, so building is only needed after changes.
