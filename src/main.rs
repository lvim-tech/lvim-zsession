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

use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use zellij_tile::prelude::*;

// zellij's `default_shell` is "zsh" in this setup; override with the plugin configuration key
// `shell` if it ever changes. Must match what zellij spawns for a plain pane (name, no args) so
// the serializer keeps treating our replacement shells as plain panes.
const DEFAULT_SHELL: &str = "zsh";

#[derive(Default)]
struct State {
    shell: String,
    ours: HashSet<u32>,    // terminal ids of the shells this plugin spawned
    handled: HashSet<u32>, // exited panes already acted on (replacement/close in flight)
}

register_plugin!(State);

impl ZellijPlugin for State {
    fn load(&mut self, configuration: BTreeMap<String, String>) {
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
                    if self.ours.contains(&pane.id) || self.is_shell(pane.terminal_command.as_deref())
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
            Event::CommandPaneExited(id, _, _) => {
                if self.handled.insert(id) {
                    close_pane_with_id(PaneId::Terminal(id));
                }
            }
            _ => {}
        }
        false
    }

    fn pipe(&mut self, message: PipeMessage) -> bool {
        if message.name == "save" {
            let outcome = match save_session() {
                Ok(()) => "session saved".to_string(),
                Err(e) => format!("session save FAILED: {e}"),
            };
            notice(&outcome);
        }
        false
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
                eprintln!("lvim-zsession: pane {pane_id} exited, shell {new_id} opened in its place");
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

fn notice(msg: &str) {
    eprintln!("lvim-zsession: {msg}");
    pipe_message_to_plugin(MessageToPlugin::new("notice").with_payload(msg.to_string()));
}
