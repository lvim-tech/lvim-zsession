// lvim-zsession — session utilities for zellij, starting with the one thing zellij gives no
// keybind ACTION for: save the session to disk NOW.
//
// zellij serializes sessions on an interval and `zellij action save-session` exists on the CLI —
// but nothing can be BOUND to a key: there is no SaveSession keybind action. This plugin closes
// that gap: bind a key to a MessagePlugin pipe named "save" and the session is serialized
// immediately (for later resurrection) — no popup, no transient pane.
//
// The plugin is stateless and LAZY: the first keypress's pipe launches ONE background instance
// (a pipe launches its target when none is running), and since the call needs no application
// state, even that launching press already saves. It never intercepts the keyboard, subscribes to
// nothing and renders nothing.

use std::collections::BTreeMap;
use zellij_tile::prelude::*;

#[derive(Default)]
struct State;

register_plugin!(State);

impl ZellijPlugin for State {
    fn load(&mut self, _configuration: BTreeMap<String, String>) {
        // SaveSession is gated behind ReadApplicationState (measured against zellij 0.46 — the
        // denial names it explicitly), counterintuitive as that reads.
        request_permission(&[PermissionType::ReadApplicationState]);
    }

    fn pipe(&mut self, message: PipeMessage) -> bool {
        if message.name == "save" {
            // The outcome goes to the zellij log — a background plugin has no pane, and a save is
            // silent by design; the log line is there when one wonders whether it happened.
            match save_session() {
                Ok(()) => eprintln!("lvim-zsession: session saved"),
                Err(e) => eprintln!("lvim-zsession: session save failed: {e}"),
            }
        }
        false
    }
}
