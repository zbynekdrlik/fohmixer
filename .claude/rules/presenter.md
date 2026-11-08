---
paths:
  - "crates/fohmixer-hub/src/presenter.rs"
  - "crates/fohmixer-hub/src/presenter/**"
  - "crates/fohmixer-hub/src/router/presenter.rs"
  - "crates/fohmixer-hub/tests/it/presenter.rs"
  - "crates/fohmixer-hub/tests/it/support/presenter.rs"
  - "crates/fohmixer-ui/src/behave/presenter.rs"
  - "crates/fohmixer-ui/src/pages/presenter.rs"
  - "crates/fohmixer-ui/src/store/live/presenter.rs"
  - "crates/fohmixer-ui/src/components/hold_key.rs"
  - "crates/fohmixer-ui/presenter.css"
  - "e2e/tests/presenter.spec.ts"
  - "e2e/harness/fake_presenter.py"
---

# Presenter's stage tab (#90; the owner's approved `docs/mockups/presenter-stage-v1.html`)

Decisions: #90's DESIGN comment of 2026-11-08 (the main session's) and the lane's DECIDED of the same day (the root paths, reads only).

- **Presenter's stage page lives on fohmixer's own origin, at Presenter's own root paths.** Its WASM shell picks itself by `location.pathname == "/stage"` and calls root paths (`/stage/…`, `/stage-displays`, `/live/ws` built from `location.host`, assets under `/ui-pkg/`, Trunk's `public_url`), so a path prefix renders "Page not found". `presenter::EXACT` / `UNDER` are the list (read from Presenter's source, `presenter-ui/src/{lib.rs,api/stage.rs,ws/stage.rs,components/stage/*}`), passed by `presenter::dispatch`, an outer layer of both listeners with its own host check, redirect, fohmixer's Access check and `frame_headers` (`X-Frame-Options: SAMEORIGIN`, `frame-ancestors 'self'`): fohmixer's own `X-Frame-Options: DENY` layer would otherwise block the frame. A new path the stage page starts to load goes into the list; never a catch-all.
- **Reads only:** GET/HEAD and the `/live/ws` upgrade (AbleSet's byte tunnel, `ableset::pipe`/`upgraded`, made `pub(crate)`); any other method 405, another upgrade 400, a dot segment (plain or `%2e`) 400, all by the hub. Presenter has no login: its write API must never be reachable through fohmixer. NDI video (WHEP is a POST) does not play in the preview. The one write is the hub's own `POST /stage/layout` (`presenter::link::Api::switch`, user agent `fohmixer-hub/<version>`).
- **The link** (`presenter/link.rs`): one session = the socket `/live/ws?surface=fohmixer&preview=1` (a preview is never counted as a stage display), then `GET /stage-displays` + `GET /stage/layout` (after the socket, so no switch is missed between), then each `{"type":"stage_layout","code"}`; ping every 5 s, 15 s of silence ends it, `RETRY` 2 s. Up/down is AbleSet's `link_step` (down after 2 failures in a row: a session's end and the next failed try), a `presenter_link` record (warn class). `RouterMsg::PresenterLink` → `router/presenter.rs` tells every page `ServerMsg::Presenter {online, layouts, current}` only on a change (kept while down).
- **A switch:** the page's `presenter_switch {code, seq}` → `switch_refusal` (not up, not listed, longer than `MAX_CODE`) or the switch task → `RouterMsg::PresenterSwitched` → the page's `presenter_ack` and one `presenter_switch` record (`client`, `peer`, `code`, `from` = the layout on stage when the page asked, carried in `SwitchRequest`: Presenter's own event often comes before its answer; `ok`, `on_stage` or `error`, `ms`, none when the hub refused). Never queued, never resent.
- **The UI:** `Extra::Presenter`, `PresenterTab` in `surface.rs` (under IEM MIXER's row), `PresenterView` over the left half. The preview frame (no `sandbox`: one with `allow-scripts allow-same-origin` is liftable by a same-origin frame, and both engines warn of it on the console, which the guard fails) is mounted only while Presenter is up (`behave::presenter::preview`), 1920 × 1080 scaled by the measured width, `pointer-events: none`. The stage area has an explicit `width` (`calc(100% - 20px)`), never a stretched width with margins: WebKit took the 16:9 height from the pane's width before the margins (17:10, measured). The keys are `components::hold_key::HoldKey` (the rail's `rail_press`, shared with the REC/ARM keys): `can` decides the fill, `wanted` what the lift switches to, `act` sends it; `release — switch` shows from `data-full` in CSS; dim keys take no touch (`pointer-events: none`).
- **Tests:** `tests/it/presenter.rs` with the in-process fake `support/presenter.rs`; the E2E harness's `fake_presenter.py` (`--fake-presenter-port`, CI 39195; control `/presenter/{down,up,switch,clear}`, `GET /presenter`); `e2e/tests/presenter.spec.ts`. Locally: the harness on ports below 32768 (the ephemeral range is not reserved on the dev box), `PLAYWRIGHT_BROWSERS_PATH=/opt/ms-playwright` (never `playwright install` in the account), `NODE_OPTIONS=--experimental-websocket` on Node 20 (`LiveClient`).
