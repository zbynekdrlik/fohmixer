---
paths:
  - "crates/fohmixer-ui/**"
---

# Leptos `view!` macro gotchas (fohmixer-ui; from iemmixer @ 22372bc)

## Never put a bare comparison inline in a `view!` attribute or `when=`

The `view!` macro tokenizes `>` / `>=` / `<` as tag boundaries. An inline comparison such as `<Show when=move || snapshots.get().len() >= 50>` does not fail to parse — it mis-tokenizes and surfaces as an unrelated `E0308 mismatched types` deep in the expansion (an iemmixer incident). Bind the predicate to a named closure first (`let at_limit = move || snapshots.get().len() >= MAX_SNAPSHOTS;`), then write `<Show when=at_limit>`. Same for `class:=`, `style:=` and any attribute expression with `<`, `>`, `>=`, `<=`.

## Signal writes after an await or in a JS callback use `try_*`

`scripts/check_disposal_safety.py` (the `integrity` CI job) rejects `.set()` / `.update()` inside `spawn_local` blocks and `Closure::wrap` callbacks — the component may already be disposed. Use `try_set` / `try_update`.

## An `on_cleanup` never touches the component's own signals

A component's `on_cleanup` runs while its view still exists: a reactive child's view (a `move ||` list of keys) is dropped only when its render effect's task is next polled, a microtask later. A write there to a signal the component bound itself (`let held = RwSignal::new(..)`), even through a helper, wakes the dying view's render effects, which then read the signal after the owner disposed it: "Tried to access a reactive value that has already been disposed" at `With::with` (#52 checkpoint C: the deck page's cleanup lifted the held keys through `leave_keys`, whose `show_held` wrote `held`; three panics on every tab leave with a key held). Do the cleanup's work with stored values and the store, and leave the view's own state alone (`pages/deck.rs` `lift_keys`). `scripts/check_disposal_safety.py` flags an `on_cleanup(` call that names such a signal (`// disposal-safe: <reason>` for a proven read).

## Never name a component prop `slot`

`<FaderView slot=… />` makes Leptos treat the attribute as a slot and fails with "slots cannot be used inside HTML elements" deep in the expansion (#8). The fohmixer controls call their slot prop `state` (`let slot = state;` inside).

## A value used by a child and by an attribute: clone it first

`view!` may move a value into a child expression before an attribute on the same element reads it (E0382 on `label`/`name`, #8). Bind the attribute's copy to its own variable before the macro (`let label_attr = label.clone();`, `data-label=label_attr`).

## A control's reactive reads are `try_*`: a dying view's effects run once more

A `RenderEffect` keeps its view's state in a value its own async task holds too, so a replaced view's state (and every effect inside it) lives until that task is next polled; inside keyed lists (`For` in `For`, the surface's lines, runs and cells since #63) each level adds a poll. Meanwhile the view's owner is already cleaned: its signals and handles are disposed. If a source that is still alive changes in that window (a store slot that `set_wanted` turns `Pending` after a layout change), the dying view's effects that subscribed to it run and read their disposed handles: "Tried to access a reactive value that has already been disposed" (CI of 5cb4d83..09ebde4; the reactive_graph debug build named `MuteView`'s `slot.with` reading a `store.slot` handle). So every read in a control and in the surface's view closures and memos is a `try_*` with a harmless default (`slot.try_with(Slot::flag).flatten()`, `state.try_get().map_or("waiting", |s| s.name())`, `failed.try_get().unwrap_or(false)`). A disposed-value panic in a release build names nothing: a one-off `[profile.release.package.reactive_graph] debug-assertions = true` in the workspace `Cargo.toml` makes it name the signal's definition and its reader (`--cfg leptos_debuginfo` does not compile with reactive_graph 0.1.8); remove it again before the merge.
