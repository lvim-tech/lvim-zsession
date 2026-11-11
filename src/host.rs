// The plugin's exports to zellij — `load`, `update`, `pipe`, `render`, `plugin_version`,
// `zellij_plugin_contract` — written out instead of `register_plugin!(State)`.
//
// This is the `register_plugin!` macro of `zellij-tile` (git rev 644d78c), line for line, with ONE
// change: `update` decodes a key press itself (`key_event`), with the whole character.
//
// Why: zellij sends the key as protobuf `MainKey::Char(i32)`, the whole code point, and the library
// decodes it with `char_index as u8 as char` (`zellij-utils/src/plugin_api/key.rs`) — inside this
// plugin. `Ctrl д` (the key of `Ctrl o` on a Bulgarian layout, U+0434) would arrive as `Ctrl 4`,
// and a Cyrillic session name typed into the panel would come out as digits and punctuation.
// lvim-zclaude carries the same fix (`keys.rs`).
//
// **When `zellij-tile` is upgraded, compare this file with the new macro** (`src/lib.rs` of the
// crate) and carry over whatever changed.

use crate::State;
use std::cell::RefCell;
use std::collections::BTreeMap;
use zellij_tile::prelude::*;
use zellij_tile::shim::plugin_api::action::ProtobufPluginConfiguration;
use zellij_tile::shim::plugin_api::event::{ProtobufEvent, ProtobufEventPayload, ProtobufEventType};
use zellij_tile::shim::plugin_api::key::{ProtobufKey, ProtobufMainKey};
use zellij_tile::shim::prost::Message;

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// The macro's `main`, called from the crate's own (a binary's `main` must live in `main.rs`).
pub fn main() {
    std::panic::set_hook(Box::new(|info| {
        report_panic(info);
    }));
}

#[no_mangle]
fn load() {
    STATE.with(|state| {
        let protobuf_bytes: Vec<u8> = zellij_tile::shim::object_from_stdin().unwrap();
        let protobuf_configuration =
            ProtobufPluginConfiguration::decode(protobuf_bytes.as_slice()).unwrap();
        let configuration: BTreeMap<String, String> =
            BTreeMap::try_from(&protobuf_configuration).unwrap();
        state.borrow_mut().load(configuration);
    });
}

#[no_mangle]
pub fn update() -> bool {
    STATE.with(|state| {
        let protobuf_bytes: Vec<u8> = zellij_tile::shim::object_from_stdin().unwrap();
        // The one place that differs from the macro.
        if let Some(event) = key_event(&protobuf_bytes) {
            return state.borrow_mut().update(event);
        }
        let Some((event, mouse_modifiers)) =
            zellij_tile::shim::plugin_api::event::plugin_event_from_protobuf_bytes(&protobuf_bytes)
        else {
            return false;
        };
        zellij_tile::shim::set_mouse_modifiers(mouse_modifiers);
        state.borrow_mut().update(event)
    })
}

#[no_mangle]
pub fn pipe() -> bool {
    STATE.with(|state| {
        let protobuf_bytes: Vec<u8> = zellij_tile::shim::object_from_stdin().unwrap();
        let Some(pipe_message) =
            zellij_tile::shim::plugin_api::pipe_message::pipe_message_from_protobuf_bytes(
                &protobuf_bytes,
            )
        else {
            return false;
        };
        state.borrow_mut().pipe(pipe_message)
    })
}

#[no_mangle]
pub fn render(rows: i32, cols: i32) {
    STATE.with(|state| {
        state.borrow_mut().render(rows as usize, cols as usize);
    });
}

#[no_mangle]
pub fn plugin_version() {
    println!("{}", zellij_tile::prelude::VERSION);
}

#[no_mangle]
pub fn zellij_plugin_contract() {}

/// A key press, decoded with its whole character; `None` for any other event.
fn key_event(bytes: &[u8]) -> Option<Event> {
    let event = ProtobufEvent::decode(bytes).ok()?;
    if event.name != ProtobufEventType::Key as i32 {
        return None;
    }
    let Some(ProtobufEventPayload::KeyPayload(key)) = event.payload else { return None };
    key_from_protobuf(key).map(Event::Key)
}

/// The wire key → `KeyWithModifier`. The modifiers and named keys are decoded by the library as
/// ever, with an ASCII letter standing in for the character; the real character goes back after.
fn key_from_protobuf(key: ProtobufKey) -> Option<KeyWithModifier> {
    let Some(ProtobufMainKey::Char(code)) = key.main_key else {
        return KeyWithModifier::try_from(key).ok();
    };
    let character = u32::try_from(code).ok().and_then(char::from_u32)?;
    let placeholder = ProtobufKey { main_key: Some(ProtobufMainKey::Char('a' as i32)), ..key };
    let mut decoded = KeyWithModifier::try_from(placeholder).ok()?;
    decoded.bare_key = BareKey::Char(character);
    Some(decoded)
}
