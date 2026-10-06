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
