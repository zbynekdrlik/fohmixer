//! What an open reads before `is_editor_open = true` (#71 PR E), pure: the
//! device at the listed path, in one batch through the script, must still be
//! the Pro-Q 4 the page listed, with its editor closed in Live. The router
//! (`router/eq.rs`, the open sequence) sends [`read`] and hands its slots to
//! [`ready`].
//!
//! - **Not a Pro-Q 4, or not the listed name** (a track or a device
//!   inserted, deleted or moved above it, another set loaded, a rename):
//!   refused [`reason::MOVED`]; the page lists again.
//! - **Its editor already open in Live** (opened on the PC, left open by a
//!   guard that failed or by the hub's stop): refused
//!   [`reason::OPEN_ON_PC`]. The hub has no window of it, so it never takes
//!   it and never turns it off (a close without its window would skip the
//!   guard: Pro-Q 4.02 crashes Live when its editor closes with a value
//!   field open).
//! - **Else** the open goes on with the device's `$ref`, read from its
//!   parent's `devices` (the script registers every object it encodes): the
//!   same object for as long as the script's connection lasts, wherever the
//!   device moves. The close turns the editor off through it.

use fohmixer_proto::eq::{PRODUCT, reason};
use serde_json::{Value, json};

use super::Device;
use super::close::EDITOR_OPEN;
use super::walk::{DEVICES, DISPLAY_NAME, PLUGIN, answered, get};

/// A device's name (what the list showed).
pub const NAME: &str = "name";
/// Why an open is refused when the editor's state could not be read.
pub const UNREAD: &str = reason::UNREAD;

/// A device path's parent (its track or chain) and its index among the
/// parent's devices: `live_set tracks 1 devices 0` is (`live_set tracks 1`,
/// 0). None for a path of another form.
pub fn parent_of(path: &str) -> Option<(&str, usize)> {
    let (rest, index) = path.rsplit_once(' ')?;
    let parent = rest.strip_suffix(" devices")?;
    Some((parent, index.parse().ok()?))
}

/// The open's read of the device at `path`, one batch: its product, its
/// name, its editor's state, and its parent's devices (its `$ref`).
pub fn read(path: &str) -> Vec<Value> {
    let parent = parent_of(path).map_or(path, |(parent, _)| parent);
    vec![
        get(path, DISPLAY_NAME),
        get(path, NAME),
        get(path, EDITOR_OPEN),
        get(parent, DEVICES),
    ]
}

/// The device's `$ref` as a command target, from its parent's devices
/// (`slot`): the item at its index, when that item is the one at `path`.
fn reference(slot: Option<&Value>, path: &str) -> Option<Value> {
    let (_, index) = parent_of(path)?;
    let item = slot.and_then(answered)?.get(index)?;
    if item.get("path").and_then(Value::as_str) != Some(path) {
        return None;
    }
    let id = item.get("$ref")?;
    Some(json!({"$ref": id, "class": PLUGIN}))
}

/// The `$ref` id a device target names (the target [`ready`] gives): the id
/// the lists name its path with while it sits there.
pub fn ref_id(target: &Value) -> Option<&str> {
    target.get("$ref").and_then(Value::as_str)
}

/// What the read says (`slots` in [`read`]'s order) of the device the list
/// found at `path` (`listed`: its name and `$ref`): its `$ref` to open and
/// later close it by, or why the open is refused. Another device at the
/// path is refused even when it has the same name (two Pro-Q 4s keeping
/// their default name after a track moved): its `$ref` differs.
pub fn ready(slots: &[Value], path: &str, listed: &Device) -> Result<Value, &'static str> {
    let text = |i: usize| slots.get(i).and_then(answered).and_then(Value::as_str);
    if text(0) != Some(PRODUCT) || text(1) != Some(listed.name.as_str()) {
        return Err(reason::MOVED);
    }
    let Some(target) = reference(slots.get(3), path) else {
        return Err(reason::MOVED);
    };
    if ref_id(&target) != Some(listed.id.as_str()) {
        return Err(reason::MOVED);
    }
    match slots.get(2).and_then(answered).and_then(Value::as_bool) {
        Some(false) => Ok(target),
        Some(true) => Err(reason::OPEN_ON_PC),
        None => Err(UNREAD),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ON_TRACK: &str = "live_set tracks 1 devices 0";
    const IN_CHAIN: &str = "live_set tracks 1 devices 1 chains 0 devices 2";

    fn ok(data: Value) -> Value {
        json!({"ok": true, "data": data})
    }

    fn failed(why: &str) -> Value {
        json!({"ok": false, "error": why})
    }

    fn read_of(target: &str, prop: &str) -> Value {
        json!({"target": target, "name": "get_prop", "args": {"prop": prop}})
    }

    fn device(path: &str, id: &str) -> Value {
        json!({"$ref": id, "path": path, "class": "PluginDevice", "name": "Vox EQ"})
    }

    /// The device a list found: its name and `$ref`.
    fn listed(name: &str, id: &str) -> Device {
        Device {
            name: name.to_string(),
            id: id.to_string(),
        }
    }

    /// The list's `Vox EQ` at `ON_TRACK`.
    fn vox() -> Device {
        listed("Vox EQ", "live_7")
    }

    /// The read's answer: product, name, editor state, parent's devices.
    fn slots(product: Value, name: Value, open: Value, devices: Value) -> Vec<Value> {
        vec![product, name, open, devices]
    }

    /// A good answer for the device at `ON_TRACK` named `Vox EQ`.
    fn good() -> Vec<Value> {
        slots(
            ok(json!("Pro-Q 4")),
            ok(json!("Vox EQ")),
            ok(json!(false)),
            ok(json!([device(ON_TRACK, "live_7")])),
        )
    }

    #[test]
    fn a_device_path_names_its_parent_and_its_index() {
        assert_eq!(parent_of(ON_TRACK), Some(("live_set tracks 1", 0)));
        assert_eq!(
            parent_of(IN_CHAIN),
            Some(("live_set tracks 1 devices 1 chains 0", 2))
        );
        assert_eq!(
            parent_of("live_set return_tracks 0 devices 12"),
            Some(("live_set return_tracks 0", 12))
        );
        assert_eq!(parent_of("live_set tracks 1"), None);
        assert_eq!(parent_of("devices 0"), None);
        assert_eq!(parent_of("live_set tracks 1 devices x"), None);
        assert_eq!(parent_of("live_set tracks 1 chains 0"), None);
        assert_eq!(parent_of("live_set"), None);
    }

    #[test]
    fn the_read_asks_the_device_and_its_parents_devices_in_one_batch() {
        assert_eq!(
            read(IN_CHAIN),
            vec![
                read_of(IN_CHAIN, "class_display_name"),
                read_of(IN_CHAIN, "name"),
                read_of(IN_CHAIN, "is_editor_open"),
                read_of("live_set tracks 1 devices 1 chains 0", "devices"),
            ]
        );
        // A path of another form asks its own devices (none: refused).
        assert_eq!(
            read("live_set tracks 1")[3],
            read_of("live_set tracks 1", "devices")
        );
    }

    #[test]
    fn the_listed_pro_q_with_its_editor_closed_opens_by_its_ref() {
        assert_eq!(
            ready(&good(), ON_TRACK, &vox()),
            Ok(json!({"$ref": "live_7", "class": "PluginDevice"}))
        );
        // In a chain: the item at its index among the chain's devices.
        let devices = json!([
            device("live_set tracks 1 devices 1 chains 0 devices 0", "live_1"),
            device("live_set tracks 1 devices 1 chains 0 devices 1", "live_2"),
            device(IN_CHAIN, "live_3"),
        ]);
        let answer = slots(
            ok(json!("Pro-Q 4")),
            ok(json!("Vox EQ")),
            ok(json!(false)),
            ok(devices),
        );
        assert_eq!(
            ready(&answer, IN_CHAIN, &listed("Vox EQ", "live_3")),
            Ok(json!({"$ref": "live_3", "class": "PluginDevice"}))
        );
    }

    #[test]
    fn a_targets_ref_id_is_its_ref() {
        let target = ready(&good(), ON_TRACK, &vox()).unwrap();
        assert_eq!(ref_id(&target), Some("live_7"));
        assert_eq!(ref_id(&json!({"class": "PluginDevice"})), None);
        assert_eq!(ref_id(&json!({"$ref": 7})), None);
        assert_eq!(ref_id(&json!(ON_TRACK)), None);
    }

    #[test]
    fn another_device_at_the_path_is_moved() {
        let mut other = good();
        other[0] = ok(json!("Pro-C 2"));
        assert_eq!(ready(&other, ON_TRACK, &vox()), Err(reason::MOVED));
        // Renamed, or another Pro-Q 4.
        assert_eq!(
            ready(&good(), ON_TRACK, &listed("Kick EQ", "live_7")),
            Err(reason::MOVED)
        );
        // Nothing there: every read failed.
        let none = slots(
            failed("not found: devices 0"),
            failed("not found: devices 0"),
            failed("not found: devices 0"),
            ok(json!([])),
        );
        assert_eq!(ready(&none, ON_TRACK, &vox()), Err(reason::MOVED));
        // A product or a name that is no text, or missing.
        let mut odd = good();
        odd[1] = ok(json!(7));
        assert_eq!(ready(&odd, ON_TRACK, &vox()), Err(reason::MOVED));
        assert_eq!(ready(&good()[..1], ON_TRACK, &vox()), Err(reason::MOVED));
        assert_eq!(ready(&[], ON_TRACK, &vox()), Err(reason::MOVED));
    }

    #[test]
    fn another_pro_q_of_the_same_name_at_the_path_is_moved() {
        // A track moved: the path names another Pro-Q 4 with the default
        // name the listed one had too; its `$ref` gives it away.
        assert_eq!(
            ready(&good(), ON_TRACK, &listed("Vox EQ", "live_9")),
            Err(reason::MOVED)
        );
    }

    #[test]
    fn a_device_whose_ref_is_not_found_is_moved() {
        for devices in [
            failed("not found"),
            ok(json!([])),
            ok(json!([device("live_set tracks 1 devices 3", "live_7")])),
            ok(json!([{"path": ON_TRACK, "class": "PluginDevice"}])),
        ] {
            let mut answer = good();
            answer[3] = devices.clone();
            assert_eq!(
                ready(&answer, ON_TRACK, &vox()),
                Err(reason::MOVED),
                "{devices}"
            );
        }
        assert_eq!(ready(&good()[..3], ON_TRACK, &vox()), Err(reason::MOVED));
        // A path of another form has no ref to read.
        assert_eq!(
            ready(&good(), "live_set tracks 1", &vox()),
            Err(reason::MOVED)
        );
    }

    #[test]
    fn an_editor_already_open_in_live_is_refused() {
        let mut open = good();
        open[2] = ok(json!(true));
        assert_eq!(ready(&open, ON_TRACK, &vox()), Err(reason::OPEN_ON_PC));
        // Its state not read: neither taken nor turned off.
        for state in [failed("no such property"), ok(json!("yes"))] {
            let mut unread = good();
            unread[2] = state;
            assert_eq!(ready(&unread, ON_TRACK, &vox()), Err(UNREAD));
        }
    }
}
