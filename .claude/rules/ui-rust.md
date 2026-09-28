---
paths:
  - "crates/fohmixer-ui/**"
  - ".cargo/mutants.toml"
---

# fohmixer-ui: where logic lives, and how it stays mutation-proof (#8)

## Pure decisions, thin browser glue

- Every decision is a pure, natively tested function or state machine: `behave/*` (TouchOSC behaviour), `binding` (what is subscribed), `stage` (geometry), `net` (handshake, reconnect schedule, HTTP answers), `store.rs` (`Slot`, `Readiness`, `Wanted`, `next_range`, …) and `store/conn.rs` (`Conn`: the connection's state machine — hello, reconnect, watchdog tick, layout and instance changes).
- The glue carries decisions out and decides nothing: `store/live.rs` (socket, timers, signals), `dom.rs`, `raf.rs`, the components. It is excluded from mutation in `.cargo/mutants.toml` one function at a time, with a reason. New logic that lands in glue is a finding: extract it into a pure function and test it (the #8 review found three bugs hiding in the old untested `LiveStore`).
- Every bound control: its subscriptions come from the `binding` functions `item_subs` uses (`strip_subs`, `solo_sub`, `mute_sub`, `param_subs`), its slot from `components::slot_of` (an unparseable path is a `Slot::Error`), `data-binding` + `aria-disabled` from one `Memo` of `Readiness` (red when unresolved, I5), and its event handler gates on the untracked readiness (`readiness_now`), not only on its own value.
- Moving parts register with `raf::animate(root, |el| Box::new(|now, step| …))`; handlers that capture a pointer also handle `on:lostpointercapture` (ends the touch like a cancel).
- **A text never overflows its box (#9):** a tab title or a strip text is fitted by `dom::fit_text(el, base)` (`None`: the stylesheet's size): it sets the base, measures the laid-out text (`Range`) against the box in page px (the stage's scale cancels out) and applies `stage::fitted_font` (96 % of the room, never up). The text is written by the same `Effect` that fits it (`components/strip.rs` `FittedText`), so the measurement is of the text shown. A vertical tab bar turns its titles (`stage::tabbar_class`: `writing-mode`, upwards on the left).
- **Style written from an effect stays off the reactive `style=`:** a reactive `style=move || …` rewrites the whole attribute and drops what `dom::set_style` wrote. A tab's `style` holds only its box; its colour (`tab_background`) and fitted size are set by an effect.
- A nested pager is a layer of its own (`z-index: stage::pager_z`, its pages' lowest item z), holding its fill, tab bar and pages; page and pager fills (`fill_style`) sit at z 0 under the items (the importer numbers items from 1).

## Mutation-proof by design (cargo-mutants 27 mutates every comparison, `!`, arithmetic, match guard, and constants in const expressions)

- A threshold is a one-line helper (`is_emergency`, `short_of_step`, `near_centre`, `within_epsilon`) tested at the exact float boundary and the next float (`0.03` vs `0.030000000000000002`), so `<`/`<=` cannot survive.
- Prefer forms without equivalent mutants: `clamp`, `min`/`max`, `total_cmp`, `is_sign_positive`, `copysign` instead of hand-written comparisons that give the same result at the boundary.
- A constant built in a const expression (`FRAME_MS = 1000.0 / 60.0`) needs a test with a literal value (50 ms), not one that uses the constant on both sides.
- Fader-shaping traces come from the original `fader_script.lua` 2.5.4 run under Lua 5.4 (lupa; `math.pow = function(a, b) return a ^ b end`). Pick start levels far from 0 dB: near 0 dB, `a - b` and `a + b` of dB levels agree and a mutant survives (#8: the forcing check at start 0.7294 ≈ 0 dB).

## Trunk

WebKit warns ("preloaded using link preload but not used within a few seconds") about Trunk's default `<link rel="preload">` of the WASM, which fails every iPad test on the console guard. `Trunk.toml` sets `pattern_preload` to a `modulepreload` of the JS only; the `wasm` CI job checks the built `index.html` has no WASM preload.
