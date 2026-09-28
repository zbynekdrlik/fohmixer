---
paths:
  - "crates/fohmixer-ui/**"
  - ".cargo/mutants.toml"
---

# fohmixer-ui: where logic lives, and how it stays mutation-proof (#8)

## Pure decisions, thin browser glue

- Every decision is a pure, natively tested function or state machine: `behave/*` (TouchOSC behaviour and the redesign's scale, dB readout, colour, peak, SOLO ✕), `binding` (what is subscribed, the page selection), `flow` (the strip width), `net` (handshake, reconnect schedule, HTTP answers), `store.rs` (`Slot`, `Readiness`, `Wanted`, `next_range`, …) and `store/conn.rs` (`Conn`: the connection's state machine — hello, reconnect, watchdog tick, layout and instance changes).
- The glue carries decisions out and decides nothing: `store/live.rs` (socket, timers, signals), `dom.rs`, `raf.rs`, the components. It is excluded from mutation in `.cargo/mutants.toml` one function at a time, with a reason. New logic that lands in glue is a finding: extract it into a pure function and test it (the #8 review found three bugs hiding in the old untested `LiveStore`).
- Every bound control: its subscriptions come from the `binding` functions `control_subs` uses (`strip_subs` with the track colour, `solo_sub`, `mute_sub`, `param_subs`), its slot from `components::slot_of` (an unparseable path is a `Slot::Error`), `data-binding` + `aria-disabled` from one `Memo` of `Readiness` (red when unresolved, I5), and its event handler gates on the untracked readiness (`readiness_now`), not only on its own value.
- Moving parts register with `raf::animate(root, |el| Box::new(|now, step| …))`; handlers that capture a pointer also handle `on:lostpointercapture` (ends the touch like a cancel).
- **A text never overflows its box (#9), and nothing measures text (#21):** the strip's texts are sized in CSS from the strip width (`--strip-w`: the name button's font `clamp(11px, strip-w × 0.2, 15px)`, the instance tag's `clamp(7px, strip-w / 9, 8.5px)`), with an ellipsis as the last guard. Measuring text per element (the old `dom::fit_text`) forced a layout per text; after a width change and the fonts' arrival WebKit stopped drawing frames for a second (the CI iPad TechAlert blink). The E2E `clipped()` check keeps every text inside its box.
- **Style written from an effect stays off the reactive `style=`:** a reactive `style=move || …` rewrites the whole attribute and drops what `dom::set_style` wrote. The frame loop writes only elements whose `style` is not reactive (the fader, the pan, the meter's parts); the name button's colour is its reactive `style` (`--tc`, `--tt`), and nothing else writes it.
- **Placement is the stylesheet's (#21, layout schema 2):** rows by `weight`, sections left to right; the one number it cannot decide, the strip width all rows share, is `flow.rs` (`--strip-w` on `.rows`): keep its `METRICS` equal to the stylesheet's gaps and paddings (`.row` gap, `.group-body` padding + border and gap, `.group.buttons` min width). A pager counts as its widest sub-page, so switching it never moves the fixed groups. Moving parts draw from CSS variables the frame loop writes: `--p` on a fader (the cap rides a track-tall `.fader-rail` translated by `(1 - p) * 100%` of its own height, so nothing is measured per frame; never container units for a per-frame value: WebKit resolved `cqh` with a layout pass per frame and the CI iPad project stopped drawing frames, #21) and on a pan (`--lo`, `--w` for its bar), a meter's cover `scaleY` and its peak layer's `translateY` (`--pk`). A fader's travel is its track's whole height (TouchOSC's 1:1).
- **Paint little per frame (#21):** the iPad is WebKit, and every meter moves every frame. On the CI runner's software WebKit the surface drew 2.5 fps until the stylesheet lost its endless animations (a pulsing dot, a pulsing pill), blurred `box-shadow`s (a lit mute's glow, the fader cap's) and rounded `overflow: hidden` clips around moving parts (strip, mute, meter bar); it now draws ~22 fps, and the meters are still the largest cost (63 fps without them). Moving layers carry `will-change: transform`. A state shows by colour and border, never by a looping animation or a blur. `e2e/tests/perf.spec.ts` bounds the first paint, every gap after it and the steady rate. To find the next culprit, measure frames per second with one stylesheet suspect switched off at a time (a `<style>` override per variant, 2 s of `requestAnimationFrame` counts each).

## Mutation-proof by design (cargo-mutants 27 mutates every comparison, `!`, arithmetic, match guard, and constants in const expressions)

- A threshold is a one-line helper (`is_emergency`, `short_of_step`, `near_centre`, `within_epsilon`) tested at the exact float boundary and the next float (`0.03` vs `0.030000000000000002`), so `<`/`<=` cannot survive.
- Prefer forms without equivalent mutants: `clamp`, `min`/`max`, `total_cmp`, `is_sign_positive`, `copysign` instead of hand-written comparisons that give the same result at the boundary.
- A constant built in a const expression (`FRAME_MS = 1000.0 / 60.0`) needs a test with a literal value (50 ms), not one that uses the constant on both sides.
- Fader-shaping traces come from the original `fader_script.lua` 2.5.4 run under Lua 5.4 (lupa; `math.pow = function(a, b) return a ^ b end`). Pick start levels far from 0 dB: near 0 dB, `a - b` and `a + b` of dB levels agree and a mutant survives (#8: the forcing check at start 0.7294 ≈ 0 dB).

## Trunk

WebKit warns ("preloaded using link preload but not used within a few seconds") about Trunk's default `<link rel="preload">` of the WASM, which fails every iPad test on the console guard. `Trunk.toml` sets `pattern_preload` to a `modulepreload` of the JS only; the `wasm` CI job checks the built `index.html` has no WASM preload.
