# Stream Deck Tab Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A "Stream Deck" tab in the fohmixer PWA that shows Bitfocus Companion's keys and presses them, the hub being one more Stream Deck surface on Companion's Satellite API (#52).

**Architecture:** A new hub task (`companion.rs` pure protocol + `companion/client.rs`, shaped like `live/client.rs`) holds one TCP connection to Companion's Satellite API, registers the surface, pings, forwards presses and reports key states as events. The router owns all deck state (`deck.rs`, `deck/holders.rs`, glue in `router/deck.rs`): the key cache, the viewers, the holder set, the silent-client release, and fans key states to viewers only through each client's `Outbox` (deck items written last). The page adds a `Deck` tab (`pages/deck.rs`) driven by a pure press state machine (`behave/deck.rs`), the store's deck state (`store/deck.rs` + `store/live/deck.rs`) and the flight recorder's `deck` events; the forensics timeline gets a deck section, the installer a `-CompanionHost`, CI a real Companion 5.0.7 job.

**Tech Stack:** Rust 1.98.1 (tokio with `io-util`, axum, serde_json, Leptos 0.7.8, web-sys 0.3.91), Python 3.11 stdlib (forensics, E2E harness, fake Companion), Playwright 1.58.2 (Chromium + WebKit iPad), Windows PowerShell 5.1, GitHub Actions with the `ghcr.io/bitfocus/companion/companion:v5.0.7` service container.

**Spec:** docs/superpowers/specs/2026-10-06-streamdeck-tab-design.md

## Global Constraints

- The spec is the contract; the deviations below ("Spec gaps settled by the probe") are flagged for the main session, each with its evidence.
- Tier 0: no local cargo compilation (no build/test/check/clippy/run, no `trunk`); locally only `cargo fmt --all`, `cargo metadata`/`tree`, `ruff check .`, `ruff format --check …`, the Python unit tests, `python3 scripts/check_integrity.py`, `python3 scripts/check_version.py`, `npx playwright test --list`. Rust, wasm, Playwright, PowerShell and mutation results come from CI.
- Version: `dev` is `0.1.0-dev.34`, `master` `0.1.0-dev.33`: no bump inside this PR; the bump to `0.1.0-dev.35` is the first commit on `dev` after the merge.
- One feature, one PR `dev` → `master`, merged with `gh pr merge --merge` (merge commit; no squash, no rebase, no force push, no history rewrite). Open the PR right after the first push (mutation runs on `pull_request` only).
- Every commit message ends with the line `Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>`; it names only its ticket as `(#52)`; the PR body ends with `🤖 Generated with [Claude Code](https://claude.com/claude-code)`.
- Public repo: no host names, IPs, Windows user names, people's or track names, PINs, keys in any file, commit, ticket or log; test data is synthetic ("Light A", "Scene 1", "Hold C", hosts `10.0.0.7`, `companion.example.org`, `127.0.0.1`).
- Before every push, exit 0 from: `python3 scripts/denylist_scan.py --denylist ~/.local/share/fohmixer-private/denylist.txt --identities scripts/allowed-identities.txt --boundary scripts/denylist-boundary.txt --accepted scripts/denylist-accepted.txt --tree HEAD --commits HEAD` (never print or copy the list).
- Satellite API: TCP port 16622; `CONNECT_TIMEOUT` 3 s; `ADD-DEVICE OK` within 3 s; `PING <n>` every 2 s; the link is lost after more than 5 s without an inbound line (5000 ms not lost, 5001 ms lost), on a read or write error, or on a line over 256 KiB; reconnect with `Backoff` (250 ms doubling to 2 s, reset only after a session that reached `ADD-DEVICE OK`); only the first failure of an outage warns.
- `ADD-DEVICE DEVICEID="fohmixer-<n>" SERIAL="fohmixer" PRODUCT_NAME="fohmixer" KEYS_TOTAL=<columns×rows> KEYS_PER_ROW=<columns> BITMAPS=<bitmap_px> BITMAP_FORMAT=webp COLORS=hex TEXT=0 BRIGHTNESS=0`, `<n>` the task's own connection-attempt counter; API version major 1 and minor ≥ 12 only.
- `KEY-PRESS DEVICEID="fohmixer-<n>" KEY=<k> PRESSED=1|0`, written by the writer task at once; answers matched first in first out; `REMOVE-DEVICE` on the graceful stop, bounded at 500 ms.
- `[companion]`: `host` required; `port` 16622; `columns` 8 (1..=16); `rows` 4 (1..=8); `bitmap_px` 144 (32..=288); `title` "Stream Deck" (1..=24 characters); `#[serde(deny_unknown_fields)]`; every field spelled out in literals.
- Silence: a client holding a deck key and silent for 2000 ms or more is released (1999 ms is not).
- `deck_key` records: on a pressed-flag change; every change within 10 s (10 000 ms inclusive) after a press on the key; else at most once per 1000 ms per key while a client views the tab; `deck_keys` summary every 60 s while the link is up; images never logged (FNV-1a 64 hash and byte length only).
- Outbox: `DeckKeys` coalesced to the latest per key in a `BTreeMap`, written last by `take()` (after `ack`), counted by `is_empty()`; `Deck` and `DeckAck` are ordered replies.
- Mutation-proof code: no hand-stepped index scans; decisions in small pure functions tested at the exact boundary and the next value; logging through `if let Some(..) = decision(..)`; one counter per kind of id; no struct-update base (`..X::default()`) in any literal the diff adds; browser-only glue excluded per function in `.cargo/mutants.toml` with a reason.
- Leptos: no bare comparison inside `view!` attributes or `when=`; signal writes from callbacks and timers through `try_set`/`try_update`; every tap target has `use:owns_touches=<named Vec>` in the same start tag as its `on:pointerdown`; no `on:click` or other mouse event on the surface.
- E2E: specs import `test`/`expect` from `./support/fixtures` (console guard: zero errors/warnings unless a test declares one), run in both `chromium` and `ipad` (WebKit), `retries: 0`, no `.skip`/`.only`/`.fixme`/`test.fail`; a test that changes shared state puts it back in `finally`/`afterEach`.
- CI: every `uses:` pinned to a full SHA with its `# vX.Y.Z` comment, no `continue-on-error`, GitHub-hosted runners only; force-kill verbs (`.kill(`, `.terminate(`, `taskkill`, `Stop-Process`, …) are banned in code trees, the E2E Python included.
- PowerShell: Windows PowerShell 5.1, ASCII only, no `??`/`?.`/ternary, no `[` in a `-like` pattern.
- Windows work on the Ableton PC only through the `mcp__win-<pc>__*` tools; no Companion key is pressed during the live check.

## Review Focus

1. **Companion's real line format** (trailing space on every line, quoted data URLs holding `=`, `+`, `/`, a bare `ERROR MESSAGE=` that is no press answer, `BRIGHTNESS` and other unknown commands, `KEY-STATE` without `COLOR`, raw rgb bitmaps). Owner: Task 3 — `classify_reads_companion_5_0_7_lines` pins the probed lines; Task 4 — `a_forwarded_press_is_answered_with_companions_round_trip` sends a bare `ERROR` before the `KEY-PRESS ERROR`.
2. **A finger down when a link drops** (the page's socket closes, the tablet goes silent on the Wi-Fi, the hub's own Companion link drops, the hub stops): Companion keeps a key held across a surface's removal (probe), so a lost release means a light stuck on. Owner: Task 8 — `a_closed_page_socket_releases_its_keys`, `a_silent_holding_page_is_released_after_two_seconds_not_before`, `a_key_held_when_companion_was_lost_is_released_after_the_reconnect`, `the_stop_releases_held_keys_then_removes_the_device`; Task 17 — the real Companion's `hold_c` back to `up`.
3. **Shared keys** (two fingers on one key, two tablets, lifts in the other order, an up from a client that never held the key): Companion must see exactly one down and one up. Owner: Task 5 — `Holders` tests; Task 8 — `two_fingers_on_one_key_give_companion_one_down_and_one_up`; Task 10 — multi-pointer `Presses` tests.
4. **Companion away at the moment of a press** (refusing connections, mid-reconnect, a press still waiting for its OK): a light must never switch late. Owner: Task 4 — offline answers without a session and on a lost link; Task 8 — `presses_while_companion_is_away_are_refused_at_once_and_never_sent_later`; Task 16 — the red flash with no later send, for Companion down and for the page's own socket down.
5. **Flooding** (blinking feedback, all 32 images on every page change, a huge line): faders must keep priority and the log its cap. Owner: Task 3 — `read_outcome` at 256 KiB and the next byte; Task 4 — `a_line_over_256_kib_ends_the_link`; Task 5 — `key_record` boundaries and the per-key rate; Task 6 — deck items after acks, coalesced; Task 8 — non-viewers get no `deck_keys`.

## Companion 5.0.7 facts (probed 2026-10-06 against `ghcr.io/bitfocus/companion/companion:v5.0.7`)

Probed on the dev box with a rootless Docker daemon (the account is not in the `docker` group), the container's ports mapped to 127.0.0.1, a Python Satellite client, and Playwright for the web UI. Containers and the image were removed afterwards.

- Image digest `sha256:8d98cc779cec5be2a77b7b36fb0da8d1a571d97b668e7ffa98e0f720393dbc35`; runs as user `companion` (uid 1000); `COMPANION_CONFIG_BASEDIR=/companion` (data in `/companion/v5.0/db.sqlite`, launch options in `/companion/config.yaml`, created by `config-tool.js` on first start); exposes 8000 (admin UI, HTTP), 16622 (Satellite TCP), 16623 (Satellite WebSocket); has its own `HEALTHCHECK` (`curl -fSsq http://localhost:8000/`, interval 30 s). `main.js --help` has no import option: an import goes through the UI.
- On connect: `BEGIN CompanionVersion="5.0.7+9763-stable-cec2f88e6f" ApiVersion="1.12.0" ` then `CAPS SUBSCRIPTIONS=0 NONSQUARE=1 BITMAP_FORMATS="rgb,png,webp" `. **Every line ends with a space before `\n`.** Quoted values are written `"…"` with no escaping (`companion/lib/Service/Satellite/SatelliteApi.ts` `sendMessage`).
- `ADD-DEVICE …` (the spec's line with `TEXT=0 BRIGHTNESS=0`) → `ADD-DEVICE OK DEVICEID="fohmixer-1" `, `BRIGHTNESS DEVICEID="fohmixer-1" VALUE=100 `, then one `KEY-STATE` per key (all 32, empty keys included). `KEYS_TOTAL=0` → `ADD-DEVICE ERROR … MESSAGE="Invalid KEYS_TOTAL" `; a second `ADD-DEVICE` of the same id on the same socket → `MESSAGE="Device already added"`; the id held by another socket → `MESSAGE="Device exists elsewhere"` (sent to the socket that owns the device). Two device ids with the same `SERIAL` are accepted side by side. `TEXT=false`, `COLORS=true`, `PRESSED=true` are accepted as booleans too.
- `KEY-STATE` with `BITMAP_FORMAT=webp COLORS=hex`: `KEY-STATE DEVICEID="fohmixer-10" KEY=0 LOCATION="1/0/0" PRESSED=1 TYPE="BUTTON" BITMAP="data:image/webp;base64,UklGRsIc…4y+M=" COLOR="#00aa00" TEXTCOLOR="#ffffff" `. **BITMAP is a `data:image/webp;base64,` URL.** A 144 px key is 720–7013 bytes per line (a full page of 32 keys 25–200 KB). A fresh install's page-nav keys come as `TYPE="PAGEUP"` (key 0), `"PAGENUM"` (8), `"PAGEDOWN"` (16) and have no `COLOR`.
- `KEY-PRESS` answers come in order before the key's new state (OK 3 ms after the press, the new image 12 ms after the OK): `KEY-PRESS OK DEVICEID="fohmixer-1" `. A press of an empty key: OK. `KEY=40` on 32 keys: `KEY-PRESS ERROR DEVICEID="fohmixer-1" MESSAGE="Invalid KEY" `. An unknown device: `MESSAGE="Device not found"`. An unknown command: `ERROR MESSAGE="Unknown command: FOO" ` (no DEVICEID). `PING abc` → `PONG abc `. `REMOVE-DEVICE` → `REMOVE-DEVICE OK DEVICEID="…" `.
- **Companion 5.0.7 closes a Satellite socket after 5.0 s without any byte from the client** (measured 5.01 s without `ADD-DEVICE`, 5.13 s with it); a `PING` every 2 s keeps it open (16 s checked).
- **A key held when its surface goes away stays held**: after the socket of the holding device closed, the button's release actions had not run (`hold_c` stayed `down`), and the next device's first `KEY-STATE` of that key said `PRESSED=1`; a release from the new device (another `DEVICEID`) ran the release actions. **A release of a key Companion does not hold is a no-op** (answered OK, no release action ran). A second down of a held key is ignored (the hold counts from the first).
- Duration actions: a key with a release group and a `1000` duration group ran the release group after a 200 ms and a 900 ms hold, the duration group after a 1500 ms hold.
- Web UI: `/import-export` holds `input[type=file][accept=".companionconfig,.yaml"]` (hidden inside a `label`); a chosen file is uploaded over the UI's tRPC WebSocket (`/trpc`, `importExport.prepareImport.start`), so the file may only be set once the socket is up (the sidebar's `v5.0.7` text appears from tRPC data); the first load of a fresh container shows the "Welcome to Companion" wizard and the "What's New" modal, which make the page behind them `aria-hidden` (role locators then find nothing: use CSS locators and close the visible `[aria-label="Close modal"]` buttons; the DOM keeps hidden closers of closed modals); the import dialog's buttons are "Import Preserving Unselected" (keeps Settings, so the Satellite service stays as it is) and "Full Reset & Import". The import accepts plain JSON under the `.companionconfig` name (and YAML; a gzip export too).
- HTTP API without login: `GET /int/export/full?format=json|yaml` and `GET /int/export/page/1?format=json` (Companion's own export, schema `version: 12`), `GET /api/custom-variable/<name>/value` (the value as text), `POST /api/custom-variable/<name>/value?value=…`.
- Companion 5 buttons are `type: button-layered` with `style.layers` (`canvas`, a `box` `color`, a `text` `text`), each property `{value, isExpression}`; a text without `isExpression` substitutes `$(custom:name)`; an expression colour such as `$(custom:light_a) == 'on' ? 43520 : 0` works; actions are entities `{id, definitionId, connectionId: "internal", options, type: "action", children}`; `custom_variable_set_value` takes `name`, `create`, `value` (an expression with `$(this:current)` toggles); duration groups are `steps."0".action_sets."1000"` with `options.runWhileHeld: []` (run on release when held at least that long). The deprecated "Button: set text/colour" actions were avoided.
- The synthetic export of Task 17 (built by `e2e/harness/companion_config.py`) imported into three fresh containers through the scripted import page (`seed.mjs`, Task 17) 3 of 3 times; Light A toggled `light_a` off→on→off on two taps, Scene 1 gave `short` (200 ms), `long` (1500 ms), `short` (900 ms), Hold C gave `down` while held.

## Spec gaps settled by the probe (for the main session)

- **G1 (§4 Disconnect, §0 "každé stlačené má svoje pustené"):** "the next session's surface starts released" is false for 5.0.7 (see the facts). The plan keeps the guarantee: keys held when the link went down are released right after the next `ADD-DEVICE OK`, logged `deck_release reason=reconnect` when the release goes out; safe because a release of a key Companion does not hold is a no-op.
- **G2 (§1 "Companion tracks no ping timeout"):** 5.0.7 closes a silent Satellite socket after 5 s. The 2 s ping is therefore required; nothing else changes.
- **G3 (§4 Graceful stop):** because of G1, the stop releases every held key first (`deck_release reason=stop`, a reason the spec already lists), then `REMOVE-DEVICE`; the router sends both to the task in that order.
- **G4 (§9 seeding):** a raw export carries machine data (the probe's own surfaces, module instance ids). `e2e/companion/test.companionconfig` is generated in Companion's export schema by `e2e/harness/companion_config.py` (a test pins the committed file to it) and was proven by importing it into fresh 5.0.7 containers; the test reads Companion's state through its HTTP API (custom variables), not through the hub.
- **G5 (§8 field lists):** `deck_press` also carries `peer` and `why`, `deck_ok` carries `client` (a page's `seq` repeats across pages, so the timeline matches an answer by client and seq).
- **G6 (§4 Inbound "type"):** no consumer reads a key's `TYPE`; the cache keeps `img`, `color`, `pressed` only.
- **G7 (§6 "resets the deck state on socket close"):** read with §2 "the same holds while the page's own socket is down": the reset sets `online` false and keeps the keys (dimmed) and the tab (red dot).
- **G8 (§5 without `[companion]`):** a `deck_press` to a hub without the table is answered `deck_ack ok:false error:"no Stream Deck"` (never happens from this page, which shows no tab then).

## Push checkpoints

- **Checkpoint A (after Task 9):** protocol, config, Companion task, deck state, outbox, event log, router wiring, the store's deck state. Local: `cargo fmt --all -- --check`, `ruff check .`, `python3 scripts/check_integrity.py`, `python3 scripts/check_version.py`, the denylist scan. Push `dev`, open the PR (Task 18 step 1), wait for the push and PR runs: `lint`, `test`, `windows`, `wasm`, `e2e` (the existing suite stays green), `integrity`, `python`, `secrets`, `supply-chain`, `version`, `mutants-list`, `mutation-warmup`, `mutation`.
- **Checkpoint B (after Task 12):** the press state machine, the recorder builders, the page and the tab. Same local checks; CI as A (the `wasm` and `e2e` jobs prove the page builds and the existing UI is untouched; `mutation` judges the pure UI code).
- **Checkpoint C (after Task 17):** forensics, installer, fake Companion and harness, the E2E deck spec, the real Companion job. Local adds `python3 -m unittest discover -s tools/forensics -p 'test_*.py'`, `python3 -m unittest discover -s e2e/harness -p 'test_*.py'`, `ruff format --check live-script sim tools e2e/harness scripts/cloudflare`, `cd e2e && npx playwright test --list && npx playwright test --config playwright.companion.config.ts --list`. CI adds `companion`.
- Each checkpoint: read BOTH runs of the head (push and PR); never cancel the push run of the PR's head; a red CI job is investigated from `gh run view <id> --log-failed`, fixed in one commit, pushed once.

---
### Task 1: Protocol: the deck messages and the status block

**Files:**
- Modify: `crates/fohmixer-proto/src/client.rs` (`ClientMsg` :52-106, `ServerMsg` :203-261, a new `DeckKey` before `ServerMsg`, `HubStatus` :483-498, a new `CompanionStatus` before it)
- Test: `crates/fohmixer-proto/src/client/tests.rs` (new test after `server_messages_round_trip`, `api_bodies_round_trip` :372-498)

**Interfaces:**
- Produces: `ClientMsg::DeckView { on: bool }`, `ClientMsg::DeckPress { key: u32, down: bool, seq: u64, t: f64, hold_ms: Option<f64>, why: Option<String> }`; `ServerMsg::Deck { online: bool, columns: u32, rows: u32, title: String }`, `ServerMsg::DeckKeys { items: Vec<DeckKey> }`, `ServerMsg::DeckAck { seq: u64, ok: bool, error: Option<String>, rtt_ms: Option<f64> }`; `pub struct DeckKey { key: u32, img: Option<String>, color: Option<String>, pressed: bool }`; `pub struct CompanionStatus { online: bool, last_error: Option<String>, connect_failures: u64, companion_version: Option<String>, api_version: Option<String>, keys: u32 }`; `HubStatus.companion: Option<CompanionStatus>`.
- Consumes: nothing new.

- [ ] **Step 1: Write the failing test** — append to `crates/fohmixer-proto/src/client/tests.rs`:

```rust
#[test]
fn deck_messages_round_trip() {
    round_trip_client(
        ClientMsg::DeckView { on: true },
        json!({"type": "deck_view", "on": true}),
    );
    // A down has no hold and no why: neither is on the wire.
    round_trip_client(
        ClientMsg::DeckPress {
            key: 3,
            down: true,
            seq: 7,
            t: 1_790_000_000_123.5,
            hold_ms: None,
            why: None,
        },
        json!({"type": "deck_press", "key": 3, "down": true, "seq": 7, "t": 1_790_000_000_123.5}),
    );
    round_trip_client(
        ClientMsg::DeckPress {
            key: 3,
            down: false,
            seq: 8,
            t: 1_790_000_000_323.5,
            hold_ms: Some(200.0),
            why: Some("cancel".into()),
        },
        json!({"type": "deck_press", "key": 3, "down": false, "seq": 8,
               "t": 1_790_000_000_323.5, "hold_ms": 200.0, "why": "cancel"}),
    );
    round_trip_server(
        ServerMsg::Deck {
            online: false,
            columns: 8,
            rows: 4,
            title: "Stream Deck".into(),
        },
        json!({"type": "deck", "online": false, "columns": 8, "rows": 4, "title": "Stream Deck"}),
    );
    round_trip_server(
        ServerMsg::DeckKeys {
            items: vec![
                DeckKey {
                    key: 0,
                    img: Some("data:image/webp;base64,UklGRg+/=".into()),
                    color: Some("#00aa00".into()),
                    pressed: true,
                },
                DeckKey {
                    key: 31,
                    img: None,
                    color: None,
                    pressed: false,
                },
            ],
        },
        json!({"type": "deck_keys", "items": [
            {"key": 0, "img": "data:image/webp;base64,UklGRg+/=", "color": "#00aa00", "pressed": true},
            {"key": 31, "pressed": false}]}),
    );
    round_trip_server(
        ServerMsg::DeckAck {
            seq: 7,
            ok: true,
            error: None,
            rtt_ms: Some(3.5),
        },
        json!({"type": "deck_ack", "seq": 7, "ok": true, "rtt_ms": 3.5}),
    );
    round_trip_server(
        ServerMsg::DeckAck {
            seq: 9,
            ok: false,
            error: Some("offline".into()),
            rtt_ms: None,
        },
        json!({"type": "deck_ack", "seq": 9, "ok": false, "error": "offline"}),
    );
}
```

In `api_bodies_round_trip`, add the field to the `HubStatus` literal (after `client_reports: vec![…],`):

```rust
        companion: Some(CompanionStatus {
            online: true,
            last_error: None,
            connect_failures: 0,
            companion_version: Some("5.0.7+9763-stable-cec2f88e6f".into()),
            api_version: Some("1.12.0".into()),
            keys: 32,
        }),
```

and, next to the `older` checks (after `older.as_object_mut().unwrap().remove("client_reports");`):

```rust
    older.as_object_mut().unwrap().remove("companion");
```

then, after `assert!(older.client_reports.is_empty());`:

```rust
    assert_eq!(older.companion, None, "an older hub's answer has no Stream Deck");
    assert_eq!(json["companion"]["keys"], 32);
    assert_eq!(json["companion"]["api_version"], "1.12.0");
```

- [ ] **Step 2: Add the protocol** — in `crates/fohmixer-proto/src/client.rs`:

Append to `ClientMsg` (after `Trace { events: Vec<Value> },`):

```rust
    /// The page opened (`on`) or closed the Stream Deck tab (#52): only a
    /// client viewing it gets the keys' images (`deck_keys`).
    DeckView { on: bool },
    /// A Stream Deck key's press (#52): `down` at the touch, up at the
    /// release. `seq` is the page's own counter of presses (not `set`'s);
    /// `t` the page's clock as for `set`; an up carries the hold the page
    /// measured and why it came (`up`, `cancel`, `lost`, `hidden`, `tab`).
    /// Answered by `deck_ack`; never queued, never resent.
    DeckPress {
        key: u32,
        down: bool,
        seq: u64,
        t: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hold_ms: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        why: Option<String>,
    },
```

Before `/// Hub → client.` add:

```rust
/// One Stream Deck key as Companion drew it (#52): its image (a `data:`
/// URL), its colour (`#rrggbb`) and whether Companion shows it pressed. A
/// key Companion has not drawn yet has neither image nor colour.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeckKey {
    pub key: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub img: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    pub pressed: bool,
}
```

Append to `ServerMsg` (after the `Error { … }` variant):

```rust
    /// The Stream Deck (#52): sent on attach and whenever Companion's link
    /// goes up or down, only by a hub with a `[companion]` table (a page
    /// without it shows no tab).
    Deck {
        online: bool,
        columns: u32,
        rows: u32,
        title: String,
    },
    /// Keys' new states (#52), coalesced per key, only to a client viewing
    /// the tab.
    DeckKeys { items: Vec<DeckKey> },
    /// The answer to a `deck_press` (#52): Companion's OK (`rtt_ms`: its
    /// round trip from the hub) or why it was not done (`offline`, Companion's
    /// own error); a press the hub took without forwarding it (the key already
    /// held, or an up from a page that does not hold it) is `ok` at once.
    DeckAck {
        seq: u64,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rtt_ms: Option<f64>,
    },
```

Before `/// \`GET /api/status\`.` add:

```rust
/// The Stream Deck's Companion link in `GET /api/status` (#52), from the
/// Companion task's snapshot: online, why the last attempt failed (until a
/// session starts), the failed attempts since the last session, Companion's
/// and its Satellite API's versions, and how many keys Companion drew this
/// session.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompanionStatus {
    pub online: bool,
    pub last_error: Option<String>,
    pub connect_failures: u64,
    pub companion_version: Option<String>,
    pub api_version: Option<String>,
    pub keys: u32,
}
```

and append to `HubStatus` (after `client_reports`):

```rust
    /// The Stream Deck's Companion link (#52); none without `[companion]`
    /// (and in an older hub's answer).
    #[serde(default)]
    pub companion: Option<CompanionStatus>,
```

- [ ] **Step 3: Format** — `cargo fmt --all` (non-compiling, allowed). The hub and UI do not compile against the new variants until Tasks 8 and 9 (exhaustive matches in `ws.rs` `handle_text` and `store/live.rs` `on_text`, the `HubStatus` literal in `lib.rs`); nothing is pushed before Checkpoint A.

- [ ] **Step 4: Verify** — at Checkpoint A, CI `test` runs `client::tests::deck_messages_round_trip` and `api_bodies_round_trip` (green), `lint` runs clippy for native and wasm32.

- [ ] **Step 5: Commit**

```bash
git add crates/fohmixer-proto/src/client.rs crates/fohmixer-proto/src/client/tests.rs
git commit -m "feat(proto): the Stream Deck messages and the status block (#52)

deck_view and deck_press from the page; deck, deck_keys and deck_ack from
the hub; the companion block of /api/status.

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---
### Task 2: Config: the `[companion]` table

**Files:**
- Modify: `crates/fohmixer-hub/src/config.rs` (doc block :1-37, constants after :66, a new `CompanionCfg` after `TunnelCfg` :178-181, `Config` :246-272, `Config::defaults` :276-289, `validate` :326-359, tests `bad_configs_are_refused` :488-520)
- Test: `crates/fohmixer-hub/src/config.rs` (`mod tests`)

**Interfaces:**
- Produces: `pub struct CompanionCfg { pub host: String, pub port: u16, pub columns: u32, pub rows: u32, pub bitmap_px: u32, pub title: String }` with `pub fn keys(&self) -> u32`; `Config.companion: Option<CompanionCfg>`; constants `DEFAULT_COMPANION_PORT: u16 = 16622`, `DEFAULT_DECK_COLUMNS: u32 = 8`, `DEFAULT_DECK_ROWS: u32 = 4`, `DEFAULT_DECK_BITMAP_PX: u32 = 144`, `DEFAULT_DECK_TITLE: &str = "Stream Deck"`.
- Consumes: nothing new.

- [ ] **Step 1: Write the failing tests** — in `config.rs` `mod tests`, add:

```rust
    #[test]
    fn the_companion_table_is_read_with_its_defaults() {
        let config = Config::parse("[companion]\nhost = \"10.0.0.7\"\n", &data()).unwrap();
        let deck = config.companion.unwrap();
        assert_eq!(
            deck,
            CompanionCfg {
                host: "10.0.0.7".into(),
                port: 16622,
                columns: 8,
                rows: 4,
                bitmap_px: 144,
                title: "Stream Deck".into(),
            }
        );
        assert_eq!(deck.keys(), 32);
        let full = Config::parse(
            "[companion]\nhost = \"companion.example.org\"\nport = 16700\ncolumns = 5\nrows = 3\n\
             bitmap_px = 72\ntitle = \"Deck\"\n",
            &data(),
        )
        .unwrap()
        .companion
        .unwrap();
        assert_eq!(
            (full.host.as_str(), full.port, full.columns, full.rows, full.bitmap_px, full.title.as_str()),
            ("companion.example.org", 16700, 5, 3, 72, "Deck")
        );
        assert_eq!(full.keys(), 15);
        assert_eq!(Config::parse("", &data()).unwrap().companion, None);
    }

    #[test]
    fn a_bad_companion_table_is_refused_at_its_bounds() {
        let table = |extra: &str| format!("[companion]\nhost = \"10.0.0.7\"\n{extra}");
        for (text, message) in [
            ("[companion]\nport = 1\n".to_string(), "missing field `host`"),
            ("[companion]\nhost = \"\"\n".to_string(), "[companion] host \"\""),
            ("[companion]\nhost = \"a b\"\n".to_string(), "[companion] host \"a b\""),
            (table("port = 0\n"), "[companion] port 0"),
            (table("columns = 0\n"), "[companion] columns 0: 1..=16"),
            (table("columns = 17\n"), "[companion] columns 17: 1..=16"),
            (table("rows = 0\n"), "[companion] rows 0: 1..=8"),
            (table("rows = 9\n"), "[companion] rows 9: 1..=8"),
            (table("bitmap_px = 31\n"), "[companion] bitmap_px 31: 32..=288"),
            (table("bitmap_px = 289\n"), "[companion] bitmap_px 289: 32..=288"),
            (table("title = \"\"\n"), "[companion] title \"\": 1..=24 characters"),
            (
                table("title = \"Stream Deck of the FOH 12\"\n"),
                "1..=24 characters",
            ),
            (table("colour = 1\n"), "unknown field"),
        ] {
            let error = format!("{:#}", Config::parse(&text, &data()).unwrap_err());
            assert!(error.contains(message), "{text}: {error}");
        }
        for ok in [
            "columns = 1\nrows = 1\nbitmap_px = 32\n",
            "columns = 16\nrows = 8\nbitmap_px = 288\n",
            // 24 characters, one of them not ASCII: characters, not bytes.
            "title = \"Stream Deck of the FOH Ž\"\n",
        ] {
            assert!(Config::parse(&table(ok), &data()).is_ok(), "{ok}");
        }
    }
```

(`"Stream Deck of the FOH 12"` is 25 characters; `"Stream Deck of the FOH Ž"` is 24 characters and 25 bytes.)

- [ ] **Step 2: Implement** — in `config.rs`:

Doc block, after the `[tunnel]` lines of the example (before the closing ```` //! ``` ````):

```rust
//!
//! # The Stream Deck tab (#52), optional: Bitfocus Companion's Satellite API
//! [companion]
//! host = "companion.example.org"
//! port = 16622           # the default
//! columns = 8            # 1..=16
//! rows = 4               # 1..=8
//! bitmap_px = 144        # 32..=288, the keys' image size
//! title = "Stream Deck"  # 1..=24 characters, the tab's title
```

Constants (after `DEFAULT_TUNNEL_READY_URL`):

```rust
/// Companion's Satellite API port when `[companion]` does not set one.
pub const DEFAULT_COMPANION_PORT: u16 = 16622;
/// The Stream Deck's grid when `[companion]` does not set it: Companion's
/// standard page (a Stream Deck XL).
pub const DEFAULT_DECK_COLUMNS: u32 = 8;
pub const DEFAULT_DECK_ROWS: u32 = 4;
/// The size Companion draws each key's image at (px, square).
pub const DEFAULT_DECK_BITMAP_PX: u32 = 144;
/// The tab's title.
pub const DEFAULT_DECK_TITLE: &str = "Stream Deck";
```

Default functions (after `default_tunnel_ready_url`):

```rust
fn default_companion_port() -> u16 {
    DEFAULT_COMPANION_PORT
}

fn default_deck_columns() -> u32 {
    DEFAULT_DECK_COLUMNS
}

fn default_deck_rows() -> u32 {
    DEFAULT_DECK_ROWS
}

fn default_deck_bitmap_px() -> u32 {
    DEFAULT_DECK_BITMAP_PX
}

fn default_deck_title() -> String {
    DEFAULT_DECK_TITLE.to_string()
}
```

After `TunnelCfg`:

```rust
/// `[companion]` (#52): the Stream Deck tab. The hub registers with
/// Bitfocus Companion's Satellite API at `host:port` as one Stream Deck of
/// `columns` × `rows` keys whose images Companion draws `bitmap_px` square;
/// the page's tab is titled `title`. The host is site data: the installer
/// writes it (`-CompanionHost`), never this repository.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompanionCfg {
    pub host: String,
    #[serde(default = "default_companion_port")]
    pub port: u16,
    #[serde(default = "default_deck_columns")]
    pub columns: u32,
    #[serde(default = "default_deck_rows")]
    pub rows: u32,
    #[serde(default = "default_deck_bitmap_px")]
    pub bitmap_px: u32,
    #[serde(default = "default_deck_title")]
    pub title: String,
}

impl CompanionCfg {
    /// The surface's keys: `columns` × `rows` (Companion's `KEYS_TOTAL`).
    pub fn keys(&self) -> u32 {
        self.columns * self.rows
    }
}
```

In `Config` (after `pub tunnel: Option<TunnelCfg>,`):

```rust
    /// The Stream Deck tab (#52); none: no tab.
    #[serde(default)]
    pub companion: Option<CompanionCfg>,
```

In `Config::defaults` (after `tunnel: None,`): `companion: None,`

In `validate`, replace the last line `self.validate_remote()` with:

```rust
        self.validate_companion()?;
        self.validate_remote()
```

and add the method after `validate`:

```rust
    /// `[companion]` (#52): a host without spaces or quotes, a port, the
    /// grid's and the image's bounds, a title of 1..=24 characters.
    fn validate_companion(&self) -> anyhow::Result<()> {
        let Some(deck) = &self.companion else {
            return Ok(());
        };
        if deck.host.is_empty() || deck.host.contains(|c: char| c.is_whitespace() || c == '"') {
            bail!("[companion] host {:?}: a name or address without spaces", deck.host);
        }
        if deck.port == 0 {
            bail!("[companion] port 0");
        }
        if !(1..=16).contains(&deck.columns) {
            bail!("[companion] columns {}: 1..=16", deck.columns);
        }
        if !(1..=8).contains(&deck.rows) {
            bail!("[companion] rows {}: 1..=8", deck.rows);
        }
        if !(32..=288).contains(&deck.bitmap_px) {
            bail!("[companion] bitmap_px {}: 32..=288", deck.bitmap_px);
        }
        if !(1..=24).contains(&deck.title.chars().count()) {
            bail!("[companion] title {:?}: 1..=24 characters", deck.title);
        }
        Ok(())
    }
```

- [ ] **Step 3: Format** — `cargo fmt --all`.

- [ ] **Step 4: Verify** — at Checkpoint A: CI `test` runs `config::tests::the_companion_table_is_read_with_its_defaults` and `a_bad_companion_table_is_refused_at_its_bounds`; the `windows` job's `config_cli` test still passes (the installer's config check).

- [ ] **Step 5: Commit**

```bash
git add crates/fohmixer-hub/src/config.rs
git commit -m "feat(hub): the [companion] table of the Stream Deck tab (#52)

host, port (16622), columns (8, 1..=16), rows (4, 1..=8), bitmap_px (144,
32..=288) and title (\"Stream Deck\", 1..=24 characters), refused at the
bounds by config check.

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---
### Task 3: The Satellite line protocol, pure

**Files:**
- Create: `crates/fohmixer-hub/src/companion.rs`
- Create: `crates/fohmixer-hub/src/companion/tests.rs`
- Modify: `crates/fohmixer-hub/src/lib.rs` :43-75 (`pub mod companion;`), `crates/fohmixer-hub/Cargo.toml` :18 (tokio gains `io-util`), `crates/fohmixer-hub/src/live/client.rs` :252 (`first_of_outage` becomes `pub(crate)`)

**Interfaces:**
- Consumes: `crate::config::CompanionCfg` (Task 2), `crate::live::subs::ClientId`.
- Produces (all `pub` in `crate::companion`): constants `SERIAL`, `PRODUCT`, `MAX_LINE: usize = 262_144`, `PING_EVERY: Duration`, `LINK_SILENCE: Duration`, `ADD_TIMEOUT: Duration`, `STOP_BOUND: Duration`, `MIN_API_MINOR: u32`, `OFFLINE: &str`; `struct Line { cmd: String, params: BTreeMap<String, Option<String>> }` with `get(&self, &str) -> Option<&str>`, `has(&self, &str) -> bool`; `fn parse_line(&str) -> Option<Line>`; `enum Read { Line(String), TooLong, Closed }`; `fn read_outcome(read: usize, buf: &[u8]) -> Read`; `fn api_ok(&str) -> bool`; `fn flag(&str) -> bool`; `fn reply(&Line) -> Result<(), String>`; `struct KeyUpdate { key: u32, img: Option<String>, color: Option<String>, pressed: Option<bool> }`; `enum Inbound { Begin { companion: String, api: String }, Caps(String), Added(Result<(), String>), Pressed(Result<(), String>), KeyState(KeyUpdate), KeysClear, Ping(String), Pong, Other(String) }` with `name(&self) -> String`; `fn classify(&str) -> Inbound`; `fn device_id(u64) -> String`; `fn add_device(&str, &CompanionCfg) -> String`; `fn key_press(&str, u32, bool) -> String`; `fn remove_device(&str) -> String`; `fn ping(u64) -> String`; `fn pong(&str) -> String`; `fn link_silent(Duration) -> bool`; `enum Phase { Begin, Adding, Up }`; `fn overdue(Phase, Duration, Duration) -> Option<&'static str>`; `fn first_seen(&mut BTreeMap<String, u64>, &str) -> bool`; `fn ms_between(Instant, Instant) -> f64`; `struct Press { key: u32, down: bool, from: Option<(ClientId, u64)> }`; `struct Answer { press: Press, ok: bool, error: Option<String>, rtt_ms: Option<f64> }` with `Answer::offline(Press) -> Answer`; `struct Fifo` with `sent(&mut self, Press, Instant)`, `answer(&mut self, Result<(), String>, Instant) -> Option<Answer>`, `offline(&mut self) -> Vec<Answer>`, `waiting(&self) -> usize`; `enum CompanionEvent { Up { companion: String, api: String, attempts: u64, down_ms: Option<f64> }, Down { error: String }, Failed { error: String, refused: bool, companion: Option<String>, api: Option<String>, attempts: u64 }, Key(KeyUpdate), Clear, Answered(Answer) }`.

- [ ] **Step 1: Write the failing tests** — `crates/fohmixer-hub/src/companion/tests.rs`:

```rust
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use super::*;
use crate::config::CompanionCfg;

fn deck(columns: u32, rows: u32, bitmap_px: u32) -> CompanionCfg {
    CompanionCfg {
        host: "10.0.0.7".into(),
        port: 16622,
        columns,
        rows,
        bitmap_px,
        title: "Stream Deck".into(),
    }
}

fn params(pairs: &[(&str, Option<&str>)]) -> BTreeMap<String, Option<String>> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), v.map(str::to_string)))
        .collect()
}

#[test]
fn a_line_is_its_command_and_its_parameters() {
    let line = parse_line("CMD A=1 B=\"x y\" C FLAG D=\"\" E=\"a=b+c/d==\" ").unwrap();
    assert_eq!(line.cmd, "CMD");
    assert_eq!(
        line.params,
        params(&[
            ("A", Some("1")),
            ("B", Some("x y")),
            ("C", None),
            ("FLAG", None),
            ("D", Some("")),
            ("E", Some("a=b+c/d==")),
        ])
    );
    assert_eq!(line.get("A"), Some("1"));
    assert_eq!(line.get("C"), None, "a bare word has no value");
    assert!(line.has("C") && line.has("A") && !line.has("Z"));
    // A quoted value at the line's end, with or without its closing quote.
    assert_eq!(parse_line("X V=\"a b\"").unwrap().get("V"), Some("a b"));
    assert_eq!(parse_line("X V=\"a b").unwrap().get("V"), Some("a b"));
    // A command alone, and blank lines.
    assert_eq!(parse_line("KEYS-CLEAR").unwrap().params, BTreeMap::new());
    assert_eq!(parse_line(""), None);
    assert_eq!(parse_line("   \r"), None);
    // A token that starts with `=` has an empty name and is still consumed.
    assert_eq!(
        parse_line("X =1 Y=2").unwrap().params,
        params(&[("", Some("1")), ("Y", Some("2"))])
    );
}

#[test]
fn classify_reads_companion_5_0_7_lines() {
    // The lines as the probe captured them (#52 plan), base64 shortened:
    // every line ends with a space.
    assert_eq!(
        classify(
            "BEGIN CompanionVersion=\"5.0.7+9763-stable-cec2f88e6f\" ApiVersion=\"1.12.0\" "
        ),
        Inbound::Begin {
            companion: "5.0.7+9763-stable-cec2f88e6f".into(),
            api: "1.12.0".into()
        }
    );
    assert_eq!(
        classify("CAPS SUBSCRIPTIONS=0 NONSQUARE=1 BITMAP_FORMATS=\"rgb,png,webp\" "),
        Inbound::Caps("SUBSCRIPTIONS=0 NONSQUARE=1 BITMAP_FORMATS=\"rgb,png,webp\"".into())
    );
    assert_eq!(
        classify("ADD-DEVICE OK DEVICEID=\"fohmixer-1\" "),
        Inbound::Added(Ok(()))
    );
    assert_eq!(
        classify("ADD-DEVICE ERROR DEVICEID=\"fohmixer-1\" MESSAGE=\"Device exists elsewhere\" "),
        Inbound::Added(Err("Device exists elsewhere".into()))
    );
    assert_eq!(
        classify("KEY-PRESS OK DEVICEID=\"fohmixer-1\" "),
        Inbound::Pressed(Ok(()))
    );
    assert_eq!(
        classify("KEY-PRESS ERROR DEVICEID=\"fohmixer-1\" MESSAGE=\"Invalid KEY\" "),
        Inbound::Pressed(Err("Invalid KEY".into()))
    );
    assert_eq!(
        classify("KEY-PRESS ERROR DEVICEID=\"nope\" "),
        Inbound::Pressed(Err("error".into())),
        "an ERROR without MESSAGE"
    );
    assert_eq!(
        classify(
            "KEY-STATE DEVICEID=\"fohmixer-10\" KEY=0 LOCATION=\"1/0/0\" PRESSED=1 TYPE=\"BUTTON\" \
             BITMAP=\"data:image/webp;base64,UklGRsIcAABXRUJQ+/4y+M=\" COLOR=\"#00aa00\" TEXTCOLOR=\"#ffffff\" "
        ),
        Inbound::KeyState(KeyUpdate {
            key: 0,
            img: Some("data:image/webp;base64,UklGRsIcAABXRUJQ+/4y+M=".into()),
            color: Some("#00aa00".into()),
            pressed: Some(true),
        })
    );
    // A page key of a fresh install: no COLOR.
    assert_eq!(
        classify(
            "KEY-STATE DEVICEID=\"fohmixer-1\" KEY=8 LOCATION=\"1/1/0\" PRESSED=0 TYPE=\"PAGENUM\" \
             BITMAP=\"data:image/webp;base64,AAAA\" "
        ),
        Inbound::KeyState(KeyUpdate {
            key: 8,
            img: Some("data:image/webp;base64,AAAA".into()),
            color: None,
            pressed: Some(false),
        })
    );
    // A missing field keeps the cache's old value: none here.
    assert_eq!(
        classify("KEY-STATE DEVICEID=\"x\" KEY=3 COLOR=\"#ff0000\""),
        Inbound::KeyState(KeyUpdate {
            key: 3,
            img: None,
            color: Some("#ff0000".into()),
            pressed: None,
        })
    );
    // A raw rgb bitmap is no image the page can show.
    assert_eq!(
        classify("KEY-STATE DEVICEID=\"x\" KEY=1 BITMAP=\"AAAAAA==\" PRESSED=true"),
        Inbound::KeyState(KeyUpdate {
            key: 1,
            img: None,
            color: None,
            pressed: Some(true),
        })
    );
    assert_eq!(
        classify("KEY-STATE DEVICEID=\"x\" KEY=x"),
        Inbound::Other("KEY-STATE".into())
    );
    assert_eq!(
        classify("KEYS-CLEAR DEVICEID=\"fohmixer-1\" "),
        Inbound::KeysClear
    );
    assert_eq!(classify("PING abc"), Inbound::Ping("abc".into()));
    assert_eq!(classify("PING abc "), Inbound::Ping("abc".into()));
    assert_eq!(classify("PONG 7 "), Inbound::Pong);
    assert_eq!(
        classify("BRIGHTNESS DEVICEID=\"fohmixer-1\" VALUE=100 "),
        Inbound::Other("BRIGHTNESS".into())
    );
    // Companion's answer to an unknown command answers no press.
    assert_eq!(
        classify("ERROR MESSAGE=\"Unknown command: FOO\" "),
        Inbound::Other("ERROR".into())
    );
    assert_eq!(classify(""), Inbound::Other(String::new()));
}

#[test]
fn every_inbound_kind_has_its_command_name() {
    for (inbound, name) in [
        (
            Inbound::Begin {
                companion: String::new(),
                api: String::new(),
            },
            "BEGIN",
        ),
        (Inbound::Caps(String::new()), "CAPS"),
        (Inbound::Added(Ok(())), "ADD-DEVICE"),
        (Inbound::Pressed(Ok(())), "KEY-PRESS"),
        (
            Inbound::KeyState(KeyUpdate {
                key: 0,
                img: None,
                color: None,
                pressed: None,
            }),
            "KEY-STATE",
        ),
        (Inbound::KeysClear, "KEYS-CLEAR"),
        (Inbound::Ping(String::new()), "PING"),
        (Inbound::Pong, "PONG"),
        (Inbound::Other("LOCKED-STATE".into()), "LOCKED-STATE"),
    ] {
        assert_eq!(inbound.name(), name);
    }
}

#[test]
fn a_flag_is_1_or_true() {
    assert!(flag("1") && flag("true") && flag("TRUE"));
    assert!(!flag("0") && !flag("false") && !flag("") && !flag("yes"));
}

#[test]
fn a_bounded_read_is_a_line_too_long_or_the_end() {
    assert_eq!(read_outcome(0, b""), Read::Closed);
    assert_eq!(read_outcome(5, b"PONG\n"), Read::Line("PONG".into()));
    assert_eq!(read_outcome(6, b"PONG\r\n"), Read::Line("PONG".into()));
    // A line of exactly MAX_LINE bytes and its newline: taken.
    let mut longest = vec![b'A'; MAX_LINE];
    longest.push(b'\n');
    assert_eq!(
        read_outcome(longest.len(), &longest),
        Read::Line("A".repeat(MAX_LINE))
    );
    // One byte more and no newline yet: too long.
    let over = vec![b'A'; MAX_LINE + 1];
    assert_eq!(read_outcome(over.len(), &over), Read::TooLong);
    // MAX_LINE bytes and no newline: the connection ended mid-line.
    let cut = vec![b'A'; MAX_LINE];
    assert_eq!(read_outcome(cut.len(), &cut), Read::Closed);
    assert_eq!(MAX_LINE, 262_144);
}

#[test]
fn the_api_must_be_1_12_or_a_later_1_x() {
    for ok in ["1.12.0", "1.12", "1.14.0", "1.12.3-beta"] {
        assert!(api_ok(ok), "{ok}");
    }
    for refused in ["1.11.9", "1.0.0", "2.0.0", "0.12.0", "", "1", "x.12", "1.x"] {
        assert!(!api_ok(refused), "{refused}");
    }
}

#[test]
fn the_hubs_lines() {
    assert_eq!(device_id(1), "fohmixer-1");
    assert_eq!(device_id(17), "fohmixer-17");
    assert_eq!(
        add_device("fohmixer-3", &deck(8, 4, 144)),
        "ADD-DEVICE DEVICEID=\"fohmixer-3\" SERIAL=\"fohmixer\" PRODUCT_NAME=\"fohmixer\" \
         KEYS_TOTAL=32 KEYS_PER_ROW=8 BITMAPS=144 BITMAP_FORMAT=webp COLORS=hex TEXT=0 BRIGHTNESS=0\n"
    );
    assert_eq!(
        add_device("fohmixer-4", &deck(5, 3, 72)),
        "ADD-DEVICE DEVICEID=\"fohmixer-4\" SERIAL=\"fohmixer\" PRODUCT_NAME=\"fohmixer\" \
         KEYS_TOTAL=15 KEYS_PER_ROW=5 BITMAPS=72 BITMAP_FORMAT=webp COLORS=hex TEXT=0 BRIGHTNESS=0\n"
    );
    assert_eq!(
        key_press("fohmixer-3", 7, true),
        "KEY-PRESS DEVICEID=\"fohmixer-3\" KEY=7 PRESSED=1\n"
    );
    assert_eq!(
        key_press("fohmixer-3", 0, false),
        "KEY-PRESS DEVICEID=\"fohmixer-3\" KEY=0 PRESSED=0\n"
    );
    assert_eq!(
        remove_device("fohmixer-3"),
        "REMOVE-DEVICE DEVICEID=\"fohmixer-3\"\n"
    );
    assert_eq!(ping(12), "PING 12\n");
    assert_eq!(pong("abc"), "PONG abc\n");
}

#[test]
fn a_session_is_overdue_at_its_deadlines() {
    let ms = Duration::from_millis;
    assert_eq!(overdue(Phase::Begin, ms(3000), ms(9000)), None);
    assert_eq!(
        overdue(Phase::Begin, ms(3001), ms(0)),
        Some("no BEGIN from Companion within 3 s")
    );
    assert_eq!(overdue(Phase::Adding, ms(3000), ms(9000)), None);
    assert_eq!(
        overdue(Phase::Adding, ms(3001), ms(0)),
        Some("no ADD-DEVICE answer within 3 s")
    );
    assert_eq!(overdue(Phase::Up, ms(60_000), ms(5000)), None);
    assert_eq!(
        overdue(Phase::Up, ms(0), ms(5001)),
        Some("nothing from Companion for 5 s")
    );
    assert!(!link_silent(ms(5000)));
    assert!(link_silent(ms(5001)));
    assert_eq!(
        (PING_EVERY, LINK_SILENCE, ADD_TIMEOUT, STOP_BOUND),
        (ms(2000), ms(5000), ms(3000), ms(500))
    );
}

#[test]
fn an_ignored_command_is_logged_once_per_session() {
    let mut counts = BTreeMap::new();
    assert!(first_seen(&mut counts, "BRIGHTNESS"));
    assert!(!first_seen(&mut counts, "BRIGHTNESS"));
    assert!(first_seen(&mut counts, "LOCKED-STATE"));
    assert!(!first_seen(&mut counts, "BRIGHTNESS"));
    assert_eq!(counts["BRIGHTNESS"], 3);
    assert_eq!(counts["LOCKED-STATE"], 1);
}

#[test]
fn milliseconds_between_two_instants() {
    let t = Instant::now();
    assert!((ms_between(t, t + Duration::from_millis(12)) - 12.0).abs() < 1e-9);
    assert!((ms_between(t, t + Duration::from_micros(2500)) - 2.5).abs() < 1e-9);
    assert_eq!(ms_between(t + Duration::from_millis(5), t), 0.0);
}

#[test]
fn answers_go_to_the_presses_in_the_order_they_were_sent() {
    let t = Instant::now();
    let down = Press {
        key: 3,
        down: true,
        from: Some((7, 1)),
    };
    let up = Press {
        key: 3,
        down: false,
        from: Some((7, 2)),
    };
    let release = Press {
        key: 9,
        down: false,
        from: None,
    };
    let mut fifo = Fifo::default();
    fifo.sent(down, t);
    fifo.sent(up, t + Duration::from_millis(100));
    fifo.sent(release, t + Duration::from_millis(150));
    assert_eq!(fifo.waiting(), 3);
    let first = fifo.answer(Ok(()), t + Duration::from_millis(4)).unwrap();
    assert_eq!((first.press, first.ok, first.error.clone()), (down, true, None));
    assert!((first.rtt_ms.unwrap() - 4.0).abs() < 1e-9);
    let second = fifo
        .answer(Err("Invalid KEY".into()), t + Duration::from_millis(110))
        .unwrap();
    assert_eq!(
        (second.press, second.ok, second.error.as_deref()),
        (up, false, Some("Invalid KEY"))
    );
    assert!((second.rtt_ms.unwrap() - 10.0).abs() < 1e-9);
    // The link goes: the rest is answered offline, without a round trip.
    assert_eq!(
        fifo.offline(),
        vec![Answer {
            press: release,
            ok: false,
            error: Some("offline".into()),
            rtt_ms: None
        }]
    );
    assert_eq!(fifo.waiting(), 0);
    // An answer to nothing the hub sent.
    assert_eq!(fifo.answer(Ok(()), t), None);
    assert_eq!(Answer::offline(down).error.as_deref(), Some(OFFLINE));
}
```

- [ ] **Step 2: Implement** — `crates/fohmixer-hub/src/companion.rs`:

```rust
//! The hub's side of Bitfocus Companion's Satellite API (#52; spec
//! `docs/superpowers/specs/2026-10-06-streamdeck-tab-design.md` §4): the hub
//! registers as one Stream Deck surface over TCP and forwards the pages'
//! presses. This file is the pure protocol, tested natively: the line parser
//! ([`parse_line`], [`classify`]), the bounded line read ([`read_outcome`]),
//! the version gate ([`api_ok`]), the lines the hub writes ([`add_device`],
//! [`key_press`], [`remove_device`], [`ping`], [`pong`]), the session's
//! deadlines ([`overdue`]) and the first-in-first-out matching of the
//! `KEY-PRESS` answers ([`Fifo`]). [`client`] is the task.
//!
//! Companion 5.0.7 as probed (#52 plan, "Companion 5.0.7 facts"): every
//! line it writes ends with a space before its `\n`; a quoted value is
//! `"…"` without escapes (a data URL holds `=`, `+` and `/`); `KEY-STATE`
//! carries `KEY`, `LOCATION`, `PRESSED`, `TYPE`, `BITMAP` (a
//! `data:image/webp;base64,` URL with `BITMAP_FORMAT=webp`), `COLOR` and
//! `TEXTCOLOR`; it answers `ADD-DEVICE`, `KEY-PRESS` and `REMOVE-DEVICE` with
//! `OK` / `ERROR … MESSAGE=` in order and an unknown command with a bare
//! `ERROR MESSAGE=` (no press answer); it closes a socket that sent nothing
//! for 5 s; a release of a key it does not hold is a no-op answered `OK`; a
//! key held when its surface goes away stays held until a release comes from
//! any surface.

pub mod client;

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

pub use client::{CompanionHandle, Events, Snapshot};

use crate::config::CompanionCfg;
use crate::live::subs::ClientId;

/// The surface's serial: Companion keys its settings for the surface (start
/// page, group) on it, so it never changes.
pub const SERIAL: &str = "fohmixer";
/// The name Companion shows for the surface.
pub const PRODUCT: &str = "fohmixer";
/// The longest line the hub reads (a 144 px webp key is a few KB; Companion
/// itself allows 2 MB): a longer one ends the link and bounds memory.
pub const MAX_LINE: usize = 256 * 1024;
/// How often the hub pings Companion (5.0.7 closes a socket silent for 5 s).
pub const PING_EVERY: Duration = Duration::from_secs(2);
/// No inbound line for longer than this: the link is lost.
pub const LINK_SILENCE: Duration = Duration::from_secs(5);
/// How long `BEGIN` and the `ADD-DEVICE` answer may each take.
pub const ADD_TIMEOUT: Duration = Duration::from_secs(3);
/// How long the graceful stop's `REMOVE-DEVICE` may take.
pub const STOP_BOUND: Duration = Duration::from_millis(500);
/// The oldest Satellite API minor version of major 1 this hub speaks (1.12:
/// `BITMAP_FORMAT`).
pub const MIN_API_MINOR: u32 = 12;
/// A press the link could not carry.
pub const OFFLINE: &str = "offline";

/// One line of the Satellite API: its command and its parameters (a bare
/// word such as `OK` has no value).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub cmd: String,
    pub params: BTreeMap<String, Option<String>>,
}

impl Line {
    /// The value of parameter `name`.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.params.get(name).and_then(|v| v.as_deref())
    }

    /// Whether `name` is there (a bare word: `OK`, `ERROR`).
    pub fn has(&self, name: &str) -> bool {
        self.params.contains_key(name)
    }
}

/// The parameter at the start of `rest` (no leading space): its name, its
/// value (none for a bare word), and the text after it. A quoted value runs
/// to the next `"` followed by a space, else to the line's end.
fn next_param(rest: &str) -> (String, Option<String>, &str) {
    let name_end = rest.find([' ', '=']).unwrap_or(rest.len());
    let (name, after) = rest.split_at(name_end);
    let Some(value) = after.strip_prefix('=') else {
        return (name.to_string(), None, after);
    };
    match value.strip_prefix('"') {
        Some(quoted) => match quoted.split_once("\" ") {
            Some((inner, after)) => (name.to_string(), Some(inner.to_string()), after),
            None => {
                let inner = quoted.strip_suffix('"').unwrap_or(quoted);
                (name.to_string(), Some(inner.to_string()), "")
            }
        },
        None => {
            let end = value.find(' ').unwrap_or(value.len());
            let (bare, after) = value.split_at(end);
            (name.to_string(), Some(bare.to_string()), after)
        }
    }
}

/// Parses a line (its `\r`, `\n` and Companion's trailing space ignored);
/// `None` for a blank one.
pub fn parse_line(text: &str) -> Option<Line> {
    let text = text.trim();
    let (cmd, mut rest) = text.split_once(' ').unwrap_or((text, ""));
    if cmd.is_empty() {
        return None;
    }
    let mut params = BTreeMap::new();
    // Each pass takes at least one byte of `rest`, so its length bounds the
    // loop (no hand-stepped index: .claude/rules/hub-rust.md).
    for _ in 0..=rest.len() {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        let (name, value, after) = next_param(rest);
        params.insert(name, value);
        rest = after;
    }
    Some(Line {
        cmd: cmd.to_string(),
        params,
    })
}

/// What one read of at most `MAX_LINE + 1` bytes up to a newline gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Read {
    /// A whole line, its line end removed.
    Line(String),
    /// More than [`MAX_LINE`] bytes without a newline.
    TooLong,
    /// The connection ended (mid-line too).
    Closed,
}

/// The outcome of a bounded `read_until` that read `read` bytes into `buf`.
pub fn read_outcome(read: usize, buf: &[u8]) -> Read {
    if read == 0 {
        return Read::Closed;
    }
    match buf.strip_suffix(b"\n") {
        Some(line) => {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            Read::Line(String::from_utf8_lossy(line).into_owned())
        }
        None if buf.len() > MAX_LINE => Read::TooLong,
        None => Read::Closed,
    }
}

/// Whether Companion's `ApiVersion` is one this hub speaks: major 1, minor
/// [`MIN_API_MINOR`] or later (`BITMAP_FORMAT`).
pub fn api_ok(version: &str) -> bool {
    let mut parts = version.split(['.', '-']).map(str::parse::<u32>);
    matches!(
        (parts.next(), parts.next()),
        (Some(Ok(1)), Some(Ok(minor))) if minor >= MIN_API_MINOR
    )
}

/// A Satellite boolean: `1` or `true` (any case).
pub fn flag(value: &str) -> bool {
    value == "1" || value.eq_ignore_ascii_case("true")
}

/// An answer's outcome: `OK`, else Companion's `MESSAGE` (`error` without
/// one).
pub fn reply(line: &Line) -> Result<(), String> {
    if line.has("OK") {
        Ok(())
    } else {
        Err(line.get("MESSAGE").unwrap_or("error").to_string())
    }
}

/// A key's new state as Companion sent it; a field it left out keeps the
/// cache's old value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyUpdate {
    pub key: u32,
    /// A `data:` URL (a raw rgb bitmap is none: the page cannot show it).
    pub img: Option<String>,
    pub color: Option<String>,
    pub pressed: Option<bool>,
}

fn key_update(line: &Line) -> Option<KeyUpdate> {
    let key = line.get("KEY")?.parse().ok()?;
    Some(KeyUpdate {
        key,
        img: line
            .get("BITMAP")
            .filter(|b| b.starts_with("data:"))
            .map(str::to_string),
        color: line.get("COLOR").map(str::to_string),
        pressed: line.get("PRESSED").map(flag),
    })
}

/// What a line from Companion means to a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inbound {
    Begin { companion: String, api: String },
    /// `CAPS` and its flags, as written (logged; the hub needs none).
    Caps(String),
    /// The `ADD-DEVICE` answer.
    Added(Result<(), String>),
    /// A `KEY-PRESS` answer.
    Pressed(Result<(), String>),
    KeyState(KeyUpdate),
    KeysClear,
    /// Companion's ping: its payload, answered `PONG <payload>`.
    Ping(String),
    Pong,
    /// Anything else, by its command (counted, logged once per session).
    Other(String),
}

impl Inbound {
    /// The command of the line it came from.
    pub fn name(&self) -> String {
        match self {
            Self::Begin { .. } => "BEGIN".into(),
            Self::Caps(_) => "CAPS".into(),
            Self::Added(_) => "ADD-DEVICE".into(),
            Self::Pressed(_) => "KEY-PRESS".into(),
            Self::KeyState(_) => "KEY-STATE".into(),
            Self::KeysClear => "KEYS-CLEAR".into(),
            Self::Ping(_) => "PING".into(),
            Self::Pong => "PONG".into(),
            Self::Other(cmd) => cmd.clone(),
        }
    }
}

/// Classifies one line from Companion.
pub fn classify(text: &str) -> Inbound {
    if let Some(payload) = text.strip_prefix("PING ") {
        return Inbound::Ping(payload.trim().to_string());
    }
    let Some(line) = parse_line(text) else {
        return Inbound::Other(String::new());
    };
    match line.cmd.as_str() {
        "BEGIN" => Inbound::Begin {
            companion: line.get("CompanionVersion").unwrap_or_default().to_string(),
            api: line.get("ApiVersion").unwrap_or_default().to_string(),
        },
        "CAPS" => Inbound::Caps(
            text.trim()
                .strip_prefix("CAPS")
                .unwrap_or_default()
                .trim()
                .to_string(),
        ),
        "ADD-DEVICE" => Inbound::Added(reply(&line)),
        "KEY-PRESS" => Inbound::Pressed(reply(&line)),
        "KEY-STATE" => key_update(&line).map_or_else(|| Inbound::Other(line.cmd.clone()), Inbound::KeyState),
        "KEYS-CLEAR" => Inbound::KeysClear,
        "PONG" => Inbound::Pong,
        _ => Inbound::Other(line.cmd),
    }
}

/// The device id of connection attempt `n` (a fresh one per connection:
/// Companion refuses an id another, possibly dead, socket still holds).
pub fn device_id(n: u64) -> String {
    format!("fohmixer-{n}")
}

/// The `ADD-DEVICE` line of the surface: `columns` × `rows` keys, webp
/// images of `bitmap_px`, colours as hex, no text, no brightness.
pub fn add_device(device: &str, deck: &CompanionCfg) -> String {
    format!(
        "ADD-DEVICE DEVICEID=\"{device}\" SERIAL=\"{SERIAL}\" PRODUCT_NAME=\"{PRODUCT}\" \
         KEYS_TOTAL={} KEYS_PER_ROW={} BITMAPS={} BITMAP_FORMAT=webp COLORS=hex TEXT=0 BRIGHTNESS=0\n",
        deck.keys(),
        deck.columns,
        deck.bitmap_px
    )
}

/// A press (`down`) or release of `key`.
pub fn key_press(device: &str, key: u32, down: bool) -> String {
    format!(
        "KEY-PRESS DEVICEID=\"{device}\" KEY={key} PRESSED={}\n",
        u8::from(down)
    )
}

/// The graceful stop's line.
pub fn remove_device(device: &str) -> String {
    format!("REMOVE-DEVICE DEVICEID=\"{device}\"\n")
}

/// The hub's ping number `n` (its own counter).
pub fn ping(n: u64) -> String {
    format!("PING {n}\n")
}

/// The answer to Companion's `PING <payload>`.
pub fn pong(payload: &str) -> String {
    format!("PONG {payload}\n")
}

/// Whether the link is lost: nothing came in for longer than
/// [`LINK_SILENCE`].
pub fn link_silent(since_heard: Duration) -> bool {
    since_heard > LINK_SILENCE
}

/// Where a session stands: waiting for `BEGIN`, for the `ADD-DEVICE`
/// answer, or registered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Begin,
    Adding,
    Up,
}

/// Why a session ends now, if it does: no `BEGIN` or no `ADD-DEVICE` answer
/// within [`ADD_TIMEOUT`] of its phase's start; once registered, nothing
/// from Companion for longer than [`LINK_SILENCE`].
pub fn overdue(phase: Phase, in_phase: Duration, since_heard: Duration) -> Option<&'static str> {
    match phase {
        Phase::Begin => (in_phase > ADD_TIMEOUT).then_some("no BEGIN from Companion within 3 s"),
        Phase::Adding => (in_phase > ADD_TIMEOUT).then_some("no ADD-DEVICE answer within 3 s"),
        Phase::Up => link_silent(since_heard).then_some("nothing from Companion for 5 s"),
    }
}

/// Counts a line of `cmd` the hub ignores; whether it is the session's first
/// of that command (the one logged).
pub fn first_seen(counts: &mut BTreeMap<String, u64>, cmd: &str) -> bool {
    let n = counts.entry(cmd.to_string()).or_insert(0);
    *n += 1;
    *n == 1
}

/// Milliseconds from `earlier` to `later` (0 when it is not later).
pub fn ms_between(earlier: Instant, later: Instant) -> f64 {
    later.saturating_duration_since(earlier).as_secs_f64() * 1000.0
}

/// A press for Companion: a page's (its client and `seq`) or one the hub
/// makes itself (`from` none: a release on a detach, a silence, a reconnect,
/// the stop).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Press {
    pub key: u32,
    pub down: bool,
    pub from: Option<(ClientId, u64)>,
}

/// Companion's answer to a press, or `offline` for one no link carried.
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    pub press: Press,
    pub ok: bool,
    pub error: Option<String>,
    /// From the hub's write to Companion's answer (none: not answered).
    pub rtt_ms: Option<f64>,
}

impl Answer {
    /// `press` was not carried: no session, or the link went first.
    pub fn offline(press: Press) -> Self {
        Self {
            press,
            ok: false,
            error: Some(OFFLINE.to_string()),
            rtt_ms: None,
        }
    }
}

/// The presses written to Companion, waiting for their answers (Companion
/// answers in order).
#[derive(Debug, Default)]
pub struct Fifo {
    waiting: VecDeque<(Press, Instant)>,
}

impl Fifo {
    /// `press` went out at `at`.
    pub fn sent(&mut self, press: Press, at: Instant) {
        self.waiting.push_back((press, at));
    }

    /// Companion answered the oldest press at `at`; none when nothing waits
    /// (an answer to nothing the hub sent).
    pub fn answer(&mut self, result: Result<(), String>, at: Instant) -> Option<Answer> {
        let (press, sent) = self.waiting.pop_front()?;
        let (ok, error) = match result {
            Ok(()) => (true, None),
            Err(message) => (false, Some(message)),
        };
        Some(Answer {
            press,
            ok,
            error,
            rtt_ms: Some(ms_between(sent, at)),
        })
    }

    /// The link went: every waiting press, oldest first, answered offline.
    pub fn offline(&mut self) -> Vec<Answer> {
        self.waiting
            .drain(..)
            .map(|(press, _)| Answer::offline(press))
            .collect()
    }

    pub fn waiting(&self) -> usize {
        self.waiting.len()
    }
}

/// What the Companion task tells the router, in the order it happened.
#[derive(Debug, Clone, PartialEq)]
pub enum CompanionEvent {
    /// `ADD-DEVICE OK`: the surface is registered. `attempts`: the attempts
    /// of the outage that ended (1 when the first one worked); `down_ms`: how
    /// long the link was down (none the first time).
    Up {
        companion: String,
        api: String,
        attempts: u64,
        down_ms: Option<f64>,
    },
    /// A registered session ended.
    Down { error: String },
    /// The first failed attempt of an outage (`refused`: Companion refused
    /// it, by its API version or `ADD-DEVICE ERROR`).
    Failed {
        error: String,
        refused: bool,
        companion: Option<String>,
        api: Option<String>,
        attempts: u64,
    },
    /// A key's new state.
    Key(KeyUpdate),
    /// `KEYS-CLEAR`.
    Clear,
    /// A press's answer (offline too).
    Answered(Answer),
}

#[cfg(test)]
mod tests;
```

`crates/fohmixer-hub/Cargo.toml` line 18:

```toml
tokio = { version = "1", features = ["rt-multi-thread", "macros", "signal", "net", "sync", "time", "io-util"] }
```

with the comment above it extended: `# Async runtime (signal: the binary's graceful stop; io-util: Companion's line reads, #52)`.

`crates/fohmixer-hub/src/live/client.rs` :252: `pub(crate) fn first_of_outage(failures: u64) -> bool {` (the Companion task shares the one decision).

`crates/fohmixer-hub/src/lib.rs`: add `pub mod companion;` between `pub mod clock;` and `pub mod config;`. (The `client` submodule comes in Task 4; until then add `companion/client.rs` as in Task 4 before Checkpoint A.)

- [ ] **Step 3: Format** — `cargo fmt --all`.

- [ ] **Step 4: Verify** — at Checkpoint A, CI `test` runs `companion::tests::*` (all green); `mutation` judges `parse_line`, `next_param`, `read_outcome`, `api_ok`, `flag`, `reply`, `classify`, `key_update`, `Inbound::name`, the builders, `link_silent`, `overdue`, `first_seen`, `ms_between`, `Fifo`.

- [ ] **Step 5: Commit**

```bash
git add crates/fohmixer-hub/src/companion.rs crates/fohmixer-hub/src/companion/tests.rs \
  crates/fohmixer-hub/Cargo.toml crates/fohmixer-hub/src/live/client.rs crates/fohmixer-hub/src/lib.rs
git commit -m "feat(hub): the Satellite API's lines, pure (#52)

The line parser for Companion 5.0.7's lines (a trailing space, quoted data
URLs), the 256 KiB bounded read, the API 1.12 gate, the hub's ADD-DEVICE,
KEY-PRESS, REMOVE-DEVICE, PING and PONG, the session's deadlines and the
first-in-first-out matching of press answers.

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---
### Task 4: The Companion task

**Files:**
- Create: `crates/fohmixer-hub/src/companion/client.rs`, `crates/fohmixer-hub/src/companion/client/tests.rs`
- Create: `crates/fohmixer-hub/tests/support/companion.rs` (the scripted fake Companion, shared with Task 8)
- Create: `crates/fohmixer-hub/tests/companion_client.rs`
- Modify: `crates/fohmixer-hub/tests/support/mod.rs` :25-30 (`pub mod companion;`), :99-125 (`events_in`)

**Interfaces:**
- Consumes: Task 3's protocol (`Phase`, `overdue`, `classify`, `Inbound`, `read_outcome`, `Read`, `MAX_LINE`, builders, `Fifo`, `Press`, `Answer`, `CompanionEvent`, `first_seen`, `ms_between`, `api_ok`, `PING_EVERY`, `STOP_BOUND`), `crate::live::Backoff`, `crate::live::client::{CONNECT_TIMEOUT, first_of_outage}`, `fohmixer_proto::client::CompanionStatus` (Task 1), `CompanionCfg` (Task 2).
- Produces: `pub type Events = Arc<dyn Fn(CompanionEvent) + Send + Sync>`; `pub struct Snapshot { online, last_error, connect_failures, companion_version, api_version, keys }`; `#[derive(Clone)] pub struct CompanionHandle` with `spawn(&CompanionCfg, Events) -> (Self, JoinHandle<()>)`, `press(&self, Press)`, `stop(&self)`, `snapshot(&self) -> Snapshot`, `status(&self) -> CompanionStatus`. Test support: `support::companion::{FakeCompanion, Script, Got, image, key_state, device_of}`, `support::events_in(&Path) -> Vec<Value>`.

- [ ] **Step 1: Write the fake Companion** — `crates/fohmixer-hub/tests/support/companion.rs`:

```rust
//! A scripted fake Bitfocus Companion for the Stream Deck's tests (#52): the
//! Satellite API on a tokio listener, written as Companion 5.0.7 writes it
//! (`BEGIN` and `CAPS` first, every line ending with a space before its
//! `\n`, `ADD-DEVICE OK` followed by `BRIGHTNESS` and 32 `KEY-STATE`s, a
//! press answered `OK` and then the key's new state). Connections are
//! numbered from 1; every line the hub sends is kept with its connection and
//! when it came. Host-free: these tests also run on Windows.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

/// How the fake answers.
#[derive(Debug, Clone)]
pub struct Script {
    /// Its `ApiVersion`.
    pub api: String,
    /// `ADD-DEVICE` refused with this message.
    pub refuse_add: Option<String>,
    /// `PING` and `KEY-PRESS` answered (false: silent after the handshake).
    pub answers: bool,
}

impl Script {
    /// Companion 5.0.7 as it answers.
    pub fn companion() -> Self {
        Self {
            api: "1.12.0".into(),
            refuse_add: None,
            answers: true,
        }
    }
}

/// A line the hub sent: its connection (from 1), when it came, its text.
#[derive(Debug, Clone)]
pub struct Got {
    pub conn: usize,
    pub at: Instant,
    pub line: String,
}

/// The fake: its port, what it got, how it answers.
#[derive(Clone)]
pub struct FakeCompanion {
    pub port: u16,
    got: Arc<Mutex<Vec<Got>>>,
    script: Arc<Mutex<Script>>,
    /// The latest connection's way out: a line, or `None` to close it.
    current: Arc<Mutex<Option<mpsc::UnboundedSender<Option<String>>>>>,
    /// New connections closed at once (Companion away).
    refusing: Arc<AtomicBool>,
}

impl FakeCompanion {
    /// A fake on a free port, answering by `script`.
    pub async fn start(script: Script) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let fake = Self {
            port: listener.local_addr().unwrap().port(),
            got: Arc::default(),
            script: Arc::new(Mutex::new(script)),
            current: Arc::default(),
            refusing: Arc::new(AtomicBool::new(false)),
        };
        let accepting = fake.clone();
        tokio::spawn(async move {
            let mut n = 0;
            while let Ok((stream, _)) = listener.accept().await {
                if accepting.refusing.load(Ordering::SeqCst) {
                    drop(stream);
                    continue;
                }
                n += 1;
                tokio::spawn(accepting.clone().serve(n, stream));
            }
        });
        fake
    }

    /// How the next connections answer.
    pub fn set_script(&self, script: Script) {
        *self.script.lock().unwrap() = script;
    }

    /// Companion away (`on`): the open connection closed, new ones closed at
    /// once; or back.
    pub fn refuse(&self, on: bool) {
        self.refusing.store(on, Ordering::SeqCst);
        if on {
            self.close();
        }
    }

    /// A line to the latest connection (Companion's trailing space added).
    pub fn send(&self, line: &str) {
        if let Some(tx) = &*self.current.lock().unwrap() {
            let _ = tx.send(Some(format!("{line} \n")));
        }
    }

    /// Closes the latest connection.
    pub fn close(&self) {
        if let Some(tx) = self.current.lock().unwrap().take() {
            let _ = tx.send(None);
        }
    }

    /// Every line the hub sent so far.
    pub fn got(&self) -> Vec<Got> {
        self.got.lock().unwrap().clone()
    }

    /// The lines of connection `conn`.
    pub fn lines_of(&self, conn: usize) -> Vec<String> {
        self.got()
            .into_iter()
            .filter(|g| g.conn == conn)
            .map(|g| g.line)
            .collect()
    }

    /// Waits up to `limit` until `check` holds on the lines; them.
    pub async fn until(&self, limit: Duration, what: &str, check: impl Fn(&[Got]) -> bool) -> Vec<Got> {
        let deadline = Instant::now() + limit;
        loop {
            let got = self.got();
            if check(&got) {
                return got;
            }
            assert!(Instant::now() < deadline, "the fake never got {what}: {got:?}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn serve(self, n: usize, stream: TcpStream) {
        let (read, mut write) = stream.into_split();
        let (tx, mut rx) = mpsc::unbounded_channel::<Option<String>>();
        *self.current.lock().unwrap() = Some(tx);
        let script = self.script.lock().unwrap().clone();
        let hello = format!(
            "BEGIN CompanionVersion=\"5.0.7+fake\" ApiVersion=\"{}\" \n\
             CAPS SUBSCRIPTIONS=0 NONSQUARE=1 BITMAP_FORMATS=\"rgb,png,webp\" \n",
            script.api
        );
        if write.write_all(hello.as_bytes()).await.is_err() {
            return;
        }
        let mut lines = BufReader::new(read).lines();
        loop {
            tokio::select! {
                line = lines.next_line() => {
                    let Ok(Some(line)) = line else { return };
                    self.got.lock().unwrap().push(Got { conn: n, at: Instant::now(), line: line.clone() });
                    for reply in answer(&script, &line) {
                        if write.write_all(reply.as_bytes()).await.is_err() {
                            return;
                        }
                    }
                }
                out = rx.recv() => match out {
                    Some(Some(text)) => {
                        if write.write_all(text.as_bytes()).await.is_err() {
                            return;
                        }
                    }
                    Some(None) | None => return,
                },
            }
        }
    }
}

/// The `DEVICEID` of a line.
pub fn device_of(line: &str) -> String {
    line.split("DEVICEID=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .unwrap_or_default()
        .to_string()
}

/// The image the fake draws for `key`: a data URL naming it and its state.
pub fn image(key: u32, pressed: bool) -> String {
    let what = format!("key {key} {}", if pressed { "down" } else { "up" });
    format!(
        "data:image/webp;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(what)
    )
}

/// A `KEY-STATE` line of `key` as Companion 5.0.7 writes it.
pub fn key_state(device: &str, key: u32, pressed: bool) -> String {
    format!(
        "KEY-STATE DEVICEID=\"{device}\" KEY={key} LOCATION=\"1/{}/{}\" PRESSED={} TYPE=\"BUTTON\" \
         BITMAP=\"{}\" COLOR=\"#000000\" TEXTCOLOR=\"#ffffff\" \n",
        key / 8,
        key % 8,
        u8::from(pressed),
        image(key, pressed)
    )
}

/// The fake's answer to one line of the hub.
fn answer(script: &Script, line: &str) -> Vec<String> {
    let device = device_of(line);
    match line.split(' ').next().unwrap_or_default() {
        "ADD-DEVICE" => match &script.refuse_add {
            Some(message) => vec![format!(
                "ADD-DEVICE ERROR DEVICEID=\"{device}\" MESSAGE=\"{message}\" \n"
            )],
            None => {
                let mut out = vec![
                    format!("ADD-DEVICE OK DEVICEID=\"{device}\" \n"),
                    format!("BRIGHTNESS DEVICEID=\"{device}\" VALUE=100 \n"),
                ];
                out.extend((0..32).map(|key| key_state(&device, key, false)));
                out
            }
        },
        "PING" if script.answers => {
            vec![format!("PONG {} \n", line.trim_start_matches("PING ").trim())]
        }
        "KEY-PRESS" if script.answers => {
            let key: u32 = line
                .split("KEY=")
                .nth(1)
                .and_then(|rest| rest.split(' ').next())
                .and_then(|k| k.parse().ok())
                .unwrap_or(0);
            let pressed = line.contains("PRESSED=1");
            vec![
                format!("KEY-PRESS OK DEVICEID=\"{device}\" \n"),
                key_state(&device, key, pressed),
            ]
        }
        "REMOVE-DEVICE" => vec![format!("REMOVE-DEVICE OK DEVICEID=\"{device}\" \n")],
        _ => Vec::new(),
    }
}
```

In `tests/support/mod.rs`: after `pub use host::Host;` add `pub mod companion;`. Replace the body of `TestHub::events` with `events_in(&self.dir)` and add, before `impl TestHub`:

```rust
/// Every event-log record in the data folder `dir` (#43), oldest day first
/// (a stopped hub's too).
pub fn events_in(dir: &Path) -> Vec<Value> {
    let logs = dir.join("logs");
    let mut days: Vec<PathBuf> = std::fs::read_dir(&logs)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("events-"))
                })
                .collect()
        })
        .unwrap_or_default();
    days.sort();
    days.iter()
        .flat_map(|day| {
            std::fs::read_to_string(day)
                .unwrap_or_default()
                .lines()
                .filter_map(|l| serde_json::from_str::<Value>(l).ok())
                .collect::<Vec<_>>()
        })
        .collect()
}
```

- [ ] **Step 2: Write the failing tests** — `crates/fohmixer-hub/tests/companion_client.rs`:

```rust
//! The Companion task (#52, `companion/client.rs`) against the scripted fake
//! Companion (`support/companion.rs`): the handshake to `ADD-DEVICE OK` and
//! Companion's key states, the pings, a press's round trip and its error, a
//! refused API and a refused `ADD-DEVICE`, 5 s of silence and the reconnect
//! with the next device id, a line over 256 KiB, and the stop's
//! `REMOVE-DEVICE`. Host-free: it also runs in the `windows` job.

mod support;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use fohmixer_hub::companion::{Answer, CompanionEvent, CompanionHandle, Events, Press};
use fohmixer_hub::config::CompanionCfg;
use support::companion::{FakeCompanion, Script, image};
use support::{runtime, serial};

/// The task's events with when they came.
#[derive(Clone, Default)]
struct Seen(Arc<Mutex<Vec<(Instant, CompanionEvent)>>>);

impl Seen {
    fn events(&self) -> Events {
        let seen = self.clone();
        Arc::new(move |event: CompanionEvent| {
            seen.0.lock().unwrap().push((Instant::now(), event));
        })
    }

    fn all(&self) -> Vec<(Instant, CompanionEvent)> {
        self.0.lock().unwrap().clone()
    }

    /// The first event `pick` takes (seen earlier or within `limit`), with
    /// when it came; the events up to it are consumed.
    async fn wait<T>(&self, limit: Duration, pick: impl Fn(&CompanionEvent) -> Option<T>) -> (Instant, T) {
        let deadline = Instant::now() + limit;
        loop {
            {
                let mut seen = self.0.lock().unwrap();
                if let Some(i) = seen.iter().position(|(_, e)| pick(e).is_some()) {
                    let (at, event) = seen[i].clone();
                    seen.drain(..=i);
                    return (at, pick(&event).unwrap());
                }
            }
            assert!(Instant::now() < deadline, "no such event within {limit:?}: {:?}", self.all());
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

fn deck(port: u16) -> CompanionCfg {
    CompanionCfg {
        host: "127.0.0.1".into(),
        port,
        columns: 8,
        rows: 4,
        bitmap_px: 72,
        title: "Stream Deck".into(),
    }
}

fn up(event: &CompanionEvent) -> Option<(String, String, u64, Option<f64>)> {
    match event {
        CompanionEvent::Up {
            companion,
            api,
            attempts,
            down_ms,
        } => Some((companion.clone(), api.clone(), *attempts, *down_ms)),
        _ => None,
    }
}

fn answered(event: &CompanionEvent) -> Option<Answer> {
    match event {
        CompanionEvent::Answered(answer) => Some(answer.clone()),
        _ => None,
    }
}

const ADD_DEVICE_1: &str = "ADD-DEVICE DEVICEID=\"fohmixer-1\" SERIAL=\"fohmixer\" PRODUCT_NAME=\"fohmixer\" \
    KEYS_TOTAL=32 KEYS_PER_ROW=8 BITMAPS=72 BITMAP_FORMAT=webp COLORS=hex TEXT=0 BRIGHTNESS=0";

#[test]
fn the_handshake_registers_one_surface_and_passes_companions_keys_on() {
    let _serial = serial();
    runtime().block_on(async {
        let fake = FakeCompanion::start(Script::companion()).await;
        let seen = Seen::default();
        let (handle, _task) = CompanionHandle::spawn(&deck(fake.port), seen.events());
        let (_, registered) = seen.wait(Duration::from_secs(5), up).await;
        assert_eq!(registered, ("5.0.7+fake".to_string(), "1.12.0".to_string(), 1, None));
        assert_eq!(fake.lines_of(1)[0], ADD_DEVICE_1);
        let (_, last) = seen
            .wait(Duration::from_secs(5), |e| match e {
                CompanionEvent::Key(k) if k.key == 31 => Some(k.clone()),
                _ => None,
            })
            .await;
        assert_eq!(last.img.as_deref(), Some(image(31, false).as_str()));
        assert_eq!((last.color.as_deref(), last.pressed), (Some("#000000"), Some(false)));
        let status = handle.status();
        assert!(status.online, "{status:?}");
        assert_eq!(
            (status.companion_version.as_deref(), status.api_version.as_deref(), status.keys),
            (Some("5.0.7+fake"), Some("1.12.0"), 32)
        );
        assert_eq!((status.connect_failures, status.last_error), (0, None));
        // KEYS-CLEAR goes on; BRIGHTNESS (sent after ADD-DEVICE OK) was
        // ignored: no event of it.
        fake.send("KEYS-CLEAR DEVICEID=\"fohmixer-1\"");
        seen.wait(Duration::from_secs(2), |e| matches!(e, CompanionEvent::Clear).then_some(())).await;
    });
}

#[test]
fn the_hub_pings_every_two_seconds_and_answers_companions_ping() {
    let _serial = serial();
    runtime().block_on(async {
        let fake = FakeCompanion::start(Script::companion()).await;
        let seen = Seen::default();
        let (_handle, _task) = CompanionHandle::spawn(&deck(fake.port), seen.events());
        seen.wait(Duration::from_secs(5), up).await;
        let got = fake
            .until(Duration::from_secs(6), "two pings", |g| {
                g.iter().any(|l| l.line == "PING 2")
            })
            .await;
        let at = |text: &str| got.iter().find(|l| l.line == text).unwrap().at;
        let gap = at("PING 2") - at("PING 1");
        assert!(
            gap >= Duration::from_millis(1700) && gap <= Duration::from_millis(2300),
            "{gap:?}"
        );
        fake.send("PING abc");
        fake.until(Duration::from_secs(2), "the pong", |g| g.iter().any(|l| l.line == "PONG abc"))
            .await;
    });
}

#[test]
fn a_forwarded_press_is_answered_with_companions_round_trip() {
    let _serial = serial();
    runtime().block_on(async {
        let fake = FakeCompanion::start(Script::companion()).await;
        let seen = Seen::default();
        let (handle, _task) = CompanionHandle::spawn(&deck(fake.port), seen.events());
        seen.wait(Duration::from_secs(5), up).await;
        let down = Press { key: 3, down: true, from: Some((7, 1)) };
        handle.press(down);
        let (_, answer) = seen.wait(Duration::from_secs(2), answered).await;
        assert_eq!((answer.press, answer.ok, answer.error.clone()), (down, true, None));
        let rtt = answer.rtt_ms.unwrap();
        assert!((0.0..1000.0).contains(&rtt), "{rtt}");
        assert!(fake.lines_of(1).contains(&"KEY-PRESS DEVICEID=\"fohmixer-1\" KEY=3 PRESSED=1".to_string()));
        // The fake's new state of the key follows its OK.
        seen.wait(Duration::from_secs(2), |e| match e {
            CompanionEvent::Key(k) if k.key == 3 && k.pressed == Some(true) => Some(()),
            _ => None,
        })
        .await;
        // A press Companion refuses: an answer the fake writes by hand,
        // after a bare ERROR that answers no press.
        fake.set_script(Script {
            api: "1.12.0".into(),
            refuse_add: None,
            answers: false,
        });
        fake.close();
        seen.wait(Duration::from_secs(5), |e| matches!(e, CompanionEvent::Down { .. }).then_some(())).await;
        seen.wait(Duration::from_secs(5), up).await;
        let refused = Press { key: 40, down: true, from: Some((7, 2)) };
        handle.press(refused);
        fake.until(Duration::from_secs(2), "the refused press", |g| {
            g.iter().any(|l| l.line.contains("KEY=40"))
        })
        .await;
        fake.send("ERROR MESSAGE=\"Unknown command: FOO\"");
        fake.send("KEY-PRESS ERROR DEVICEID=\"fohmixer-2\" MESSAGE=\"Invalid KEY\"");
        let (_, answer) = seen.wait(Duration::from_secs(2), answered).await;
        assert_eq!(
            (answer.press, answer.ok, answer.error.as_deref()),
            (refused, false, Some("Invalid KEY"))
        );
    });
}
```

```rust
#[test]
fn an_api_before_1_12_and_a_refused_add_device_are_failed_attempts() {
    let _serial = serial();
    runtime().block_on(async {
        let old = FakeCompanion::start(Script {
            api: "1.11.0".into(),
            refuse_add: None,
            answers: true,
        })
        .await;
        let seen = Seen::default();
        let (handle, task) = CompanionHandle::spawn(&deck(old.port), seen.events());
        let (_, failed) = seen
            .wait(Duration::from_secs(5), |e| match e {
                CompanionEvent::Failed { error, refused, api, attempts, .. } => {
                    Some((error.clone(), *refused, api.clone(), *attempts))
                }
                _ => None,
            })
            .await;
        assert!(failed.0.contains("1.11.0"), "{failed:?}");
        assert_eq!((failed.1, failed.2.as_deref(), failed.3), (true, Some("1.11.0"), 1));
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert!(old.got().iter().all(|g| !g.line.starts_with("ADD-DEVICE")), "never registered");
        let status = handle.status();
        assert!(!status.online);
        assert!(status.connect_failures >= 2, "{status:?}");
        assert!(status.last_error.unwrap().contains("1.11.0"));
        // Only the outage's first failure is an event.
        let failures = seen.all().iter().filter(|(_, e)| matches!(e, CompanionEvent::Failed { .. })).count();
        assert_eq!(failures, 0, "the first was consumed; no other came");
        handle.stop();
        tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap();

        let refusing = FakeCompanion::start(Script {
            api: "1.12.0".into(),
            refuse_add: Some("test refusal".into()),
            answers: true,
        })
        .await;
        let seen = Seen::default();
        let (handle, _task) = CompanionHandle::spawn(&deck(refusing.port), seen.events());
        let (_, error) = seen
            .wait(Duration::from_secs(5), |e| match e {
                CompanionEvent::Failed { error, refused: true, .. } => Some(error.clone()),
                _ => None,
            })
            .await;
        assert_eq!(error, "ADD-DEVICE refused: test refusal");
        assert!(!handle.status().online);
    });
}

#[test]
fn companion_silent_for_five_seconds_is_lost_and_the_next_device_id_registers() {
    let _serial = serial();
    runtime().block_on(async {
        // Companion goes silent after the handshake: no PONG, no answers.
        let fake = FakeCompanion::start(Script {
            api: "1.12.0".into(),
            refuse_add: None,
            answers: false,
        })
        .await;
        let seen = Seen::default();
        let (handle, _task) = CompanionHandle::spawn(&deck(fake.port), seen.events());
        seen.wait(Duration::from_secs(5), up).await;
        let (heard, _) = seen
            .wait(Duration::from_secs(2), |e| match e {
                CompanionEvent::Key(k) if k.key == 31 => Some(()),
                _ => None,
            })
            .await;
        let press = Press { key: 2, down: true, from: Some((7, 1)) };
        handle.press(press);
        // The press waiting for its answer goes offline with the link.
        let (lost_answer_at, offline) = seen.wait(Duration::from_secs(8), answered).await;
        assert_eq!(offline, Answer::offline(press));
        let (down_at, error) = seen
            .wait(Duration::from_secs(2), |e| match e {
                CompanionEvent::Down { error } => Some(error.clone()),
                _ => None,
            })
            .await;
        assert_eq!(error, "nothing from Companion for 5 s");
        assert!(lost_answer_at <= down_at);
        let silence = down_at - heard;
        assert!(
            silence >= Duration::from_secs(5) && silence < Duration::from_millis(6500),
            "{silence:?}"
        );
        // The pings went out meanwhile.
        assert!(fake.lines_of(1).iter().any(|l| l == "PING 2"));
        // The reconnect registers the next device id after the backoff.
        let (_, again) = seen.wait(Duration::from_secs(5), up).await;
        assert_eq!(again.2, 1);
        assert!(again.3.unwrap() >= 200.0, "down for the backoff at least: {again:?}");
        assert!(fake.lines_of(2)[0].starts_with("ADD-DEVICE DEVICEID=\"fohmixer-2\" SERIAL=\"fohmixer\""));
    });
}

#[test]
fn a_line_over_256_kib_ends_the_link() {
    let _serial = serial();
    runtime().block_on(async {
        let fake = FakeCompanion::start(Script::companion()).await;
        let seen = Seen::default();
        let (_handle, _task) = CompanionHandle::spawn(&deck(fake.port), seen.events());
        seen.wait(Duration::from_secs(5), up).await;
        fake.send(&format!(
            "KEY-STATE DEVICEID=\"fohmixer-1\" KEY=0 BITMAP=\"data:image/webp;base64,{}\"",
            "A".repeat(256 * 1024)
        ));
        let (_, error) = seen
            .wait(Duration::from_secs(5), |e| match e {
                CompanionEvent::Down { error } => Some(error.clone()),
                _ => None,
            })
            .await;
        assert_eq!(error, "a line over 256 KiB");
    });
}

#[test]
fn a_stop_writes_what_came_before_it_then_removes_the_device() {
    let _serial = serial();
    runtime().block_on(async {
        let fake = FakeCompanion::start(Script::companion()).await;
        let seen = Seen::default();
        let (handle, task) = CompanionHandle::spawn(&deck(fake.port), seen.events());
        seen.wait(Duration::from_secs(5), up).await;
        handle.press(Press { key: 4, down: false, from: None });
        handle.stop();
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("the task ends within the stop's bound")
            .unwrap();
        let lines = fake
            .until(Duration::from_secs(2), "REMOVE-DEVICE", |g| {
                g.iter().any(|l| l.line.starts_with("REMOVE-DEVICE"))
            })
            .await;
        // The last lines of the connection, a ping that may fall between
        // them left out: the release, then REMOVE-DEVICE.
        let lines: Vec<&str> = lines
            .iter()
            .map(|g| g.line.as_str())
            .filter(|l| !l.starts_with("PING "))
            .collect();
        assert_eq!(
            lines[lines.len() - 2..],
            [
                "KEY-PRESS DEVICEID=\"fohmixer-1\" KEY=4 PRESSED=0",
                "REMOVE-DEVICE DEVICEID=\"fohmixer-1\"",
            ]
        );
    });
}
```

The task's unit tests, `crates/fohmixer-hub/src/companion/client/tests.rs` (`client.rs` ends with `#[cfg(test)] mod tests;`):

```rust
use std::time::Duration;

use tokio::io::AsyncReadExt;

use super::*;

fn deck(port: u16) -> CompanionCfg {
    CompanionCfg {
        host: "127.0.0.1".into(),
        port,
        columns: 8,
        rows: 4,
        bitmap_px: 72,
        title: "Stream Deck".into(),
    }
}

/// An event sink and what it got.
fn sink() -> (Events, Arc<Mutex<Vec<CompanionEvent>>>) {
    let seen = Arc::new(Mutex::new(Vec::<CompanionEvent>::new()));
    let into = Arc::clone(&seen);
    let events: Events = Arc::new(move |e: CompanionEvent| into.lock().unwrap().push(e));
    (events, seen)
}

/// Waits up to 1 s for `event`.
async fn answered_at_once(seen: &Mutex<Vec<CompanionEvent>>, event: &CompanionEvent) {
    let started = Instant::now();
    while !seen.lock().unwrap().contains(event) {
        assert!(started.elapsed() < Duration::from_secs(1), "answered at once, never queued");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn a_press_before_add_device_ok_is_answered_offline_and_never_written() {
    // A Companion that accepts and says nothing: the session waits for BEGIN.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (events, seen) = sink();
    let (handle, task) = CompanionHandle::spawn(&deck(port), events);
    let (mut socket, _) = listener.accept().await.unwrap();
    let press = Press { key: 2, down: true, from: Some((4, 1)) };
    handle.press(press);
    answered_at_once(&seen, &CompanionEvent::Answered(Answer::offline(press))).await;
    // Nothing reached Companion.
    let mut buf = [0_u8; 64];
    let read = tokio::time::timeout(Duration::from_millis(500), socket.read(&mut buf)).await;
    assert!(read.is_err(), "no line went out: {read:?}");
    handle.stop();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("a stop ends the task")
        .unwrap();
}

#[tokio::test]
async fn a_press_without_a_session_is_answered_offline_at_once() {
    // A port nothing listens on: the task stays without a session.
    let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = free.local_addr().unwrap().port();
    drop(free);
    let (events, seen) = sink();
    let (handle, task) = CompanionHandle::spawn(&deck(port), events);
    let press = Press { key: 1, down: true, from: Some((3, 9)) };
    handle.press(press);
    answered_at_once(&seen, &CompanionEvent::Answered(Answer::offline(press))).await;
    // The first failure of the outage is an event; the status counts it.
    let deadline = Instant::now() + Duration::from_secs(3);
    while handle.snapshot().connect_failures == 0 {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(seen.lock().unwrap().iter().any(|e| matches!(e, CompanionEvent::Failed { refused: false, attempts: 1, .. })));
    assert!(!handle.status().online);
    handle.stop();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("a stop ends the task")
        .unwrap();
}
```

- [ ] **Step 3: Implement** — `crates/fohmixer-hub/src/companion/client.rs`:

```rust
//! The task that holds the hub's one Companion connection (#52, spec §4),
//! shaped like `live/client.rs`: it reconnects with [`Backoff`] and never
//! gives up; while there is no session a press is answered `offline` at once
//! (never queued); lines go out through a writer task of its own and come in
//! through a reader task of its own (each at most [`MAX_LINE`]), so the
//! session's `select!` never awaits a socket; it pings every 2 s and gives
//! the link up after 5 s without a line. A session is one connection that
//! reached `ADD-DEVICE OK`; each connection registers a fresh device id
//! (`fohmixer-<n>`) under the one serial `fohmixer`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use fohmixer_proto::client::CompanionStatus;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::{
    Answer, CompanionEvent, Fifo, Inbound, MAX_LINE, PING_EVERY, Phase, Press, Read, STOP_BOUND,
    add_device, api_ok, classify, device_id, first_seen, key_press, ms_between, overdue, ping, pong,
    read_outcome, remove_device,
};
use crate::config::CompanionCfg;
use crate::live::Backoff;
use crate::live::client::{CONNECT_TIMEOUT, first_of_outage};

/// How often the session checks its deadlines.
const CHECK_EVERY: Duration = Duration::from_millis(100);
/// Lines read ahead of the session.
const INBOUND_QUEUE: usize = 256;

/// Where the task's events go, called from the task in order.
pub type Events = Arc<dyn Fn(CompanionEvent) + Send + Sync>;

/// A request for the task.
enum Request {
    Press(Press),
    Stop,
}

/// The link as the hub last saw it (`/api/status`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub online: bool,
    pub last_error: Option<String>,
    pub connect_failures: u64,
    pub companion_version: Option<String>,
    pub api_version: Option<String>,
    /// The keys Companion drew this session.
    pub keys: u32,
}

fn lock(m: &Mutex<Snapshot>) -> std::sync::MutexGuard<'_, Snapshot> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The handle of the Companion task.
#[derive(Clone)]
pub struct CompanionHandle {
    tx: mpsc::UnboundedSender<Request>,
    snapshot: Arc<Mutex<Snapshot>>,
}

impl CompanionHandle {
    /// Starts the task of `deck`; its events go to `events`. It ends on
    /// [`CompanionHandle::stop`] or when every handle is gone.
    pub fn spawn(deck: &CompanionCfg, events: Events) -> (Self, JoinHandle<()>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let snapshot = Arc::new(Mutex::new(Snapshot::default()));
        let task = tokio::spawn(run(deck.clone(), rx, events, Arc::clone(&snapshot)));
        (Self { tx, snapshot }, task)
    }

    /// Writes a press at once; without a session it is answered `offline`
    /// at once (never queued).
    pub fn press(&self, press: Press) {
        let _ = self.tx.send(Request::Press(press));
    }

    /// Ends the task after every press sent before: `REMOVE-DEVICE`, bounded
    /// by [`STOP_BOUND`], then the close.
    pub fn stop(&self) {
        let _ = self.tx.send(Request::Stop);
    }

    /// The link now.
    pub fn snapshot(&self) -> Snapshot {
        lock(&self.snapshot).clone()
    }

    /// `/api/status`'s companion block.
    pub fn status(&self) -> CompanionStatus {
        let snap = self.snapshot();
        CompanionStatus {
            online: snap.online,
            last_error: snap.last_error,
            connect_failures: snap.connect_failures,
            companion_version: snap.companion_version,
            api_version: snap.api_version,
            keys: snap.keys,
        }
    }
}

/// Why a connection never became a session.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Failure {
    error: String,
    /// Companion refused it (its API version, `ADD-DEVICE ERROR`).
    refused: bool,
    companion: Option<String>,
    api: Option<String>,
}

impl Failure {
    fn plain(error: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            refused: false,
            companion: None,
            api: None,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum End {
    /// A session ended: reconnect.
    Lost(String),
    /// The connection never became a session.
    Failed(Failure),
    /// The hub stops.
    Stop,
}

/// Waits for `work` while answering every press `offline` (no session);
/// `None` when the hub stops (a stop, or every handle gone).
async fn refusing<T>(
    rx: &mut mpsc::UnboundedReceiver<Request>,
    events: &Events,
    work: impl std::future::Future<Output = T>,
) -> Option<T> {
    tokio::pin!(work);
    loop {
        tokio::select! {
            out = &mut work => return Some(out),
            request = rx.recv() => match request {
                Some(Request::Press(press)) => events(CompanionEvent::Answered(Answer::offline(press))),
                Some(Request::Stop) | None => return None,
            },
        }
    }
}

async fn run(
    deck: CompanionCfg,
    mut rx: mpsc::UnboundedReceiver<Request>,
    events: Events,
    snapshot: Arc<Mutex<Snapshot>>,
) {
    let mut backoff = Backoff::default();
    // One counter per kind of id: the device ids count attempts.
    let mut attempts: u64 = 0;
    let mut failures: u64 = 0;
    let mut down_since: Option<Instant> = None;
    loop {
        attempts += 1;
        let device = device_id(attempts);
        let connect = tokio::time::timeout(
            CONNECT_TIMEOUT,
            TcpStream::connect((deck.host.as_str(), deck.port)),
        );
        let Some(attempt) = refusing(&mut rx, &events, connect).await else {
            return;
        };
        let failure = match attempt {
            Ok(Ok(stream)) => {
                let session = Session {
                    deck: &deck,
                    device: &device,
                    events: &events,
                    snapshot: &snapshot,
                };
                match session.run(stream, &mut rx, failures + 1, down_since).await {
                    End::Stop => return,
                    End::Lost(why) => {
                        tracing::info!(device = %device, error = %why, "the Companion link was lost");
                        backoff.reset();
                        failures = 0;
                        down_since = Some(Instant::now());
                        events(CompanionEvent::Down { error: why });
                        None
                    }
                    End::Failed(failure) => Some(failure),
                }
            }
            Ok(Err(error)) => Some(Failure::plain(error.to_string())),
            Err(_) => Some(Failure::plain("connection attempt timed out")),
        };
        if let Some(failure) = failure {
            failures += 1;
            {
                let mut snap = lock(&snapshot);
                snap.connect_failures = failures;
                snap.last_error = Some(failure.error.clone());
            }
            if first_of_outage(failures) {
                tracing::warn!(host = %deck.host, port = deck.port, error = %failure.error, "no usable Companion: retrying every 2 s at most");
                events(CompanionEvent::Failed {
                    error: failure.error,
                    refused: failure.refused,
                    companion: failure.companion,
                    api: failure.api,
                    attempts: failures,
                });
            } else {
                tracing::debug!(host = %deck.host, port = deck.port, failures, error = %failure.error, "still no usable Companion");
            }
        }
        let wait = tokio::time::sleep(backoff.next_delay());
        if refusing(&mut rx, &events, wait).await.is_none() {
            return;
        }
    }
}

/// Reads Companion's lines, each at most [`MAX_LINE`] bytes, into `lines`
/// until the connection ends, a line is too long or the session is gone.
async fn read_lines(mut reader: BufReader<OwnedReadHalf>, lines: mpsc::Sender<Read>) {
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let read = (&mut reader)
            .take(MAX_LINE as u64 + 1)
            .read_until(b'\n', &mut buf)
            .await
            .unwrap_or(0);
        let outcome = read_outcome(read, &buf);
        let last = !matches!(outcome, Read::Line(_));
        if lines.send(outcome).await.is_err() || last {
            return;
        }
    }
}

/// Writes the session's lines in order until a write fails or the session
/// drops its sender; then closes the connection's write side.
async fn write_lines(mut half: OwnedWriteHalf, mut lines: mpsc::UnboundedReceiver<String>) {
    while let Some(line) = lines.recv().await {
        if half.write_all(line.as_bytes()).await.is_err() {
            return;
        }
    }
    let _ = half.shutdown().await;
}

/// One connection.
struct Session<'a> {
    deck: &'a CompanionCfg,
    device: &'a str,
    events: &'a Events,
    snapshot: &'a Mutex<Snapshot>,
}

/// A connection's state.
struct State {
    phase: Phase,
    /// When the phase began (its deadline).
    since: Instant,
    /// The last line from Companion.
    heard: Instant,
    pings: u64,
    fifo: Fifo,
    seen: BTreeSet<u32>,
    others: BTreeMap<String, u64>,
    companion: Option<String>,
    api: Option<String>,
}

impl State {
    fn new(now: Instant) -> Self {
        Self {
            phase: Phase::Begin,
            since: now,
            heard: now,
            pings: 0,
            fifo: Fifo::default(),
            seen: BTreeSet::new(),
            others: BTreeMap::new(),
            companion: None,
            api: None,
        }
    }

    /// The connection ends for `why`: a lost link once registered, else a
    /// failed attempt.
    fn end(&self, why: String) -> End {
        if self.phase == Phase::Up {
            End::Lost(why)
        } else {
            End::Failed(Failure {
                error: why,
                refused: false,
                companion: self.companion.clone(),
                api: self.api.clone(),
            })
        }
    }
}

impl Session<'_> {
    fn emit(&self, event: CompanionEvent) {
        (self.events)(event);
    }

    async fn run(
        &self,
        stream: TcpStream,
        rx: &mut mpsc::UnboundedReceiver<Request>,
        attempts: u64,
        down_since: Option<Instant>,
    ) -> End {
        let (read_half, write_half) = stream.into_split();
        let (out, lines_out) = mpsc::unbounded_channel::<String>();
        let mut writer = tokio::spawn(write_lines(write_half, lines_out));
        let (lines_in, mut inbound) = mpsc::channel::<Read>(INBOUND_QUEUE);
        let reader = tokio::spawn(read_lines(BufReader::new(read_half), lines_in));
        let mut state = State::new(Instant::now());
        let mut pinger =
            tokio::time::interval_at(tokio::time::Instant::now() + PING_EVERY, PING_EVERY);
        pinger.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut check = tokio::time::interval(CHECK_EVERY);
        check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let end = loop {
            tokio::select! {
                read = inbound.recv() => {
                    let now = Instant::now();
                    state.heard = now;
                    match read {
                        Some(Read::Line(text)) => {
                            if let Some(end) = self.on_line(&mut state, &out, &text, now, (attempts, down_since)) {
                                break end;
                            }
                        }
                        Some(Read::TooLong) => break state.end(format!("a line over {} KiB", MAX_LINE / 1024)),
                        Some(Read::Closed) | None => break state.end("Companion closed the connection".to_string()),
                    }
                }
                request = rx.recv() => match request {
                    Some(Request::Press(press)) => self.on_press(&mut state, &out, press),
                    Some(Request::Stop) | None => {
                        if state.phase == Phase::Up {
                            let _ = out.send(remove_device(self.device));
                        }
                        // The writer writes what waits, then closes.
                        drop(out);
                        if tokio::time::timeout(STOP_BOUND, &mut writer).await.is_err() {
                            tracing::warn!(device = self.device, "REMOVE-DEVICE did not go out within 500 ms");
                            writer.abort();
                        }
                        reader.abort();
                        tracing::info!(device = self.device, "the Stream Deck left Companion: the hub stops");
                        return End::Stop;
                    }
                },
                _ = &mut writer => break state.end("a write to Companion failed".to_string()),
                _ = pinger.tick() => {
                    state.pings += 1;
                    let _ = out.send(ping(state.pings));
                }
                _ = check.tick() => {
                    let now = Instant::now();
                    if let Some(why) = overdue(state.phase, now - state.since, now - state.heard) {
                        break state.end(why.to_string());
                    }
                }
            }
        };
        writer.abort();
        reader.abort();
        for answer in state.fifo.offline() {
            self.emit(CompanionEvent::Answered(answer));
        }
        if let End::Lost(why) = &end {
            tracing::info!(device = self.device, others = ?state.others, "the Companion session ended");
            // Every field spelled out: no struct-update base.
            *lock(self.snapshot) = Snapshot {
                online: false,
                last_error: Some(why.clone()),
                connect_failures: 0,
                companion_version: None,
                api_version: None,
                keys: 0,
            };
        }
        end
    }

    /// One line from Companion; the connection's end when it ends it. The
    /// match names every kind (no wildcard): a line that does not belong to
    /// the phase is counted and logged once per command per session.
    fn on_line(
        &self,
        state: &mut State,
        out: &mpsc::UnboundedSender<String>,
        text: &str,
        now: Instant,
        (attempts, down_since): (u64, Option<Instant>),
    ) -> Option<End> {
        let up = state.phase == Phase::Up;
        let ignored = match classify(text) {
            Inbound::Begin { companion, api } => {
                if state.phase != Phase::Begin {
                    Some("BEGIN".to_string())
                } else {
                    return self.on_begin(state, out, companion, api, now);
                }
            }
            Inbound::Caps(caps) => {
                tracing::info!(device = self.device, caps = %caps, "Companion's capabilities");
                None
            }
            Inbound::Added(result) => {
                if state.phase != Phase::Adding {
                    Some("ADD-DEVICE".to_string())
                } else {
                    return self.on_added(state, result, now, attempts, down_since);
                }
            }
            Inbound::KeyState(update) => {
                if up {
                    state.seen.insert(update.key);
                    lock(self.snapshot).keys = u32::try_from(state.seen.len()).unwrap_or(u32::MAX);
                    self.emit(CompanionEvent::Key(update));
                    None
                } else {
                    Some("KEY-STATE".to_string())
                }
            }
            Inbound::KeysClear => {
                if up {
                    self.emit(CompanionEvent::Clear);
                    None
                } else {
                    Some("KEYS-CLEAR".to_string())
                }
            }
            Inbound::Pressed(result) => {
                if up {
                    match state.fifo.answer(result, now) {
                        Some(answer) => {
                            self.emit(CompanionEvent::Answered(answer));
                            None
                        }
                        None => Some("KEY-PRESS".to_string()),
                    }
                } else {
                    Some("KEY-PRESS".to_string())
                }
            }
            Inbound::Ping(payload) => {
                let _ = out.send(pong(&payload));
                None
            }
            Inbound::Pong => None,
            Inbound::Other(cmd) => Some(cmd),
        };
        if let Some(cmd) = ignored
            && first_seen(&mut state.others, &cmd)
        {
            tracing::debug!(device = self.device, cmd = %cmd, "a Companion line the hub ignores");
        }
        None
    }

    /// `BEGIN`: the API gate, then `ADD-DEVICE`.
    fn on_begin(
        &self,
        state: &mut State,
        out: &mpsc::UnboundedSender<String>,
        companion: String,
        api: String,
        now: Instant,
    ) -> Option<End> {
        state.companion = Some(companion.clone());
        state.api = Some(api.clone());
        if !api_ok(&api) {
            return Some(End::Failed(Failure {
                error: format!("Companion's Satellite API {api} is not 1.12 or a later 1.x"),
                refused: true,
                companion: Some(companion),
                api: Some(api),
            }));
        }
        let _ = out.send(add_device(self.device, self.deck));
        state.phase = Phase::Adding;
        state.since = now;
        None
    }

    /// The `ADD-DEVICE` answer: registered (the `Up` event), or refused.
    fn on_added(
        &self,
        state: &mut State,
        result: Result<(), String>,
        now: Instant,
        attempts: u64,
        down_since: Option<Instant>,
    ) -> Option<End> {
        if let Err(message) = result {
            return Some(End::Failed(Failure {
                error: format!("ADD-DEVICE refused: {message}"),
                refused: true,
                companion: state.companion.clone(),
                api: state.api.clone(),
            }));
        }
        state.phase = Phase::Up;
        state.since = now;
        let companion = state.companion.clone().unwrap_or_default();
        let api = state.api.clone().unwrap_or_default();
        tracing::info!(device = self.device, companion = %companion, api = %api, "Stream Deck registered with Companion");
        // Every field spelled out: no struct-update base.
        *lock(self.snapshot) = Snapshot {
            online: true,
            last_error: None,
            connect_failures: 0,
            companion_version: Some(companion.clone()),
            api_version: Some(api.clone()),
            keys: 0,
        };
        self.emit(CompanionEvent::Up {
            companion,
            api,
            attempts,
            down_ms: down_since.map(|at| ms_between(at, now)),
        });
        None
    }

    /// A press: written at once in a session, else (no session yet, or the
    /// writer gone) answered `offline`, never written later.
    fn on_press(&self, state: &mut State, out: &mpsc::UnboundedSender<String>, press: Press) {
        let written = state.phase == Phase::Up
            && out
                .send(key_press(self.device, press.key, press.down))
                .is_ok();
        if written {
            state.fifo.sent(press, Instant::now());
        } else {
            self.emit(CompanionEvent::Answered(Answer::offline(press)));
        }
    }
}

#[cfg(test)]
mod tests;
```

- [ ] **Step 4: Format and check the CI wiring** — `cargo fmt --all`. In `.config/nextest.toml`, add `| binary_id(fohmixer-hub::companion_client)` to the `hub-hosts` group filter (timing checks: the 2 s pings, the 5 s silence) with a comment `# companion_client and deck (#52): no host, but timing checks`. In `.github/workflows/ci.yml` `windows` job, the step "Hub API tests on Windows" becomes `cargo test --locked -p fohmixer-hub --test layout --test auth --test config_cli --test companion_client --test deck` and its name "(the layout, auth, the installer's config check, the Stream Deck's Companion link; no Live host needed)". (The `deck` binary comes in Task 8.)

- [ ] **Step 5: Verify** — at Checkpoint A: CI `test` (and `windows`) run `companion_client` (7 tests: handshake, pings, round trip with the bare ERROR, refused API and ADD-DEVICE, 5 s silence with the offline answer and the reconnect with `fohmixer-2`, the 256 KiB line, the stop's order) and `companion::client::tests::*` (a press before `ADD-DEVICE OK` answered offline and never written; a press without a session answered offline at once); `mutation` judges `run`, `refusing`, `read_lines`, `write_lines`, `Session::*`, `State::*`, `CompanionHandle::*` (each covered by those tests).

- [ ] **Step 6: Commit**

```bash
git add crates/fohmixer-hub/src/companion/client.rs crates/fohmixer-hub/src/companion/client \
  crates/fohmixer-hub/tests/support/companion.rs \
  crates/fohmixer-hub/tests/support/mod.rs crates/fohmixer-hub/tests/companion_client.rs \
  .config/nextest.toml .github/workflows/ci.yml
git commit -m "feat(hub): the Companion task of the Stream Deck (#52)

One TCP session to Companion's Satellite API: BEGIN, the API 1.12 gate,
ADD-DEVICE with a fresh device id, a ping every 2 s, the link lost after
5 s of silence or a line over 256 KiB, presses written at once and
answered first in first out, offline at once without a session, and
REMOVE-DEVICE on the stop within 500 ms.

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---
### Task 5: The router's deck state, pure

**Files:**
- Create: `crates/fohmixer-hub/src/deck.rs`
- Create: `crates/fohmixer-hub/src/deck/holders.rs`
- Create: `crates/fohmixer-hub/src/deck/tests.rs`
- Modify: `crates/fohmixer-hub/src/lib.rs` (`pub mod deck;` after `pub mod config;`)

**Interfaces:**
- Consumes: `crate::companion::{Answer, KeyUpdate}` (Task 3), `crate::clock::one_way_delay`, `crate::live::subs::ClientId`, `fohmixer_proto::client::DeckKey` (Task 1).
- Produces: in `crate::deck::holders`: `enum Hold { Forward, Held, NotHeld }`, `struct Holders` with `down(&mut self, u32, ClientId) -> Hold`, `up(&mut self, u32, ClientId) -> Hold`, `drop_client(&mut self, ClientId) -> Vec<u32>`, `clear(&mut self) -> Vec<u32>`, `holders(&self, u32) -> usize`, `clients(&self) -> BTreeSet<ClientId>`. In `crate::deck`: constants `SILENT_MS = 2000.0`, `PRESS_WINDOW_MS = 10_000.0`, `KEY_LOG_EVERY_MS = 1000.0`, `SUMMARY_EVERY_MS = 60_000.0`, `TICK: Duration = 100 ms`, `NO_DECK = "no Stream Deck"`, `BLACK = "#000000"`; `fn silent(f64) -> bool`, `fn in_press_window(f64) -> bool`, `fn key_log_due(Option<f64>) -> bool`, `fn summary_due(f64) -> bool`, `fn key_record(bool, Option<f64>, bool, Option<f64>) -> bool`, `fn img_hash(&str) -> String`; `enum PressOutcome { Forwarded, Offline, Held, NotHeld, NoDeck }` with `reason(self) -> Option<&'static str>`, `ack(self) -> Option<(bool, Option<&'static str>)>`; `struct PressRecord<'a>`; `struct Deck` (Default) with `online`, `heard`, `view`, `viewers`, `press`, `holders_of`, `gap`, `forwarded`, `detach`, `silent_clients`, `link_down`, `link_up`, `stop`, `apply`, `clear`, `summary`; field builders `press_fields`, `ok_fields`, `release_fields`, `link_fields`, `key_fields`, `view_fields`.

- [ ] **Step 1: Write the failing tests** — `crates/fohmixer-hub/src/deck/tests.rs`:

```rust
use serde_json::json;

use super::*;
use crate::companion::{Answer, KeyUpdate, Press};

fn update(key: u32, img: Option<&str>, color: Option<&str>, pressed: Option<bool>) -> KeyUpdate {
    KeyUpdate {
        key,
        img: img.map(str::to_string),
        color: color.map(str::to_string),
        pressed,
    }
}

fn online() -> Deck {
    let mut deck = Deck::default();
    assert_eq!(deck.link_up(), Vec::<u32>::new());
    deck
}

#[test]
fn holders_forward_the_first_down_and_the_last_up() {
    let mut h = Holders::default();
    assert_eq!(h.down(4, 1), Hold::Forward);
    assert_eq!(h.down(4, 2), Hold::Held);
    assert_eq!(h.holders(4), 2);
    // The first finger lifts: Companion still holds the key.
    assert_eq!(h.up(4, 1), Hold::Held);
    assert_eq!(h.holders(4), 1);
    assert_eq!(h.up(4, 2), Hold::Forward);
    assert_eq!(h.holders(4), 0);
    // An up from a client that does not hold the key, or of a key nobody holds.
    assert_eq!(h.up(4, 2), Hold::NotHeld);
    assert_eq!(h.down(5, 1), Hold::Forward);
    assert_eq!(h.up(5, 3), Hold::NotHeld);
    assert_eq!(h.holders(5), 1, "a stranger's up leaves the holder");
    // A second down of the same client is no second press.
    assert_eq!(h.down(5, 1), Hold::Held);
    assert_eq!(h.up(5, 1), Hold::Forward);
}

#[test]
fn a_gone_client_releases_only_the_keys_it_held_alone() {
    let mut h = Holders::default();
    h.down(1, 7);
    h.down(1, 8);
    h.down(2, 7);
    h.down(5, 7);
    h.down(6, 8);
    assert_eq!(h.clients(), BTreeSet::from([7, 8]));
    assert_eq!(h.drop_client(7), vec![2, 5]);
    assert_eq!(h.holders(1), 1);
    assert_eq!(h.drop_client(7), Vec::<u32>::new());
    assert_eq!(h.clear(), vec![1, 6]);
    assert_eq!(h.clients(), BTreeSet::new());
}

#[test]
fn the_deck_thresholds_at_their_boundaries() {
    assert!(!silent(1999.0));
    assert!(silent(2000.0));
    assert!(in_press_window(10_000.0));
    assert!(!in_press_window(10_000.0_f64.next_up()));
    assert!(key_log_due(None));
    assert!(!key_log_due(Some(999.999)));
    assert!(key_log_due(Some(1000.0)));
    assert!(!summary_due(59_999.0));
    assert!(summary_due(60_000.0));
    assert_eq!(TICK, std::time::Duration::from_millis(100));
}

#[test]
fn a_key_change_is_recorded_on_a_pressed_change_after_a_press_or_once_a_second_while_viewed() {
    // The pressed flag changed: always.
    assert!(key_record(true, None, false, Some(0.0)));
    // Within 10 s of a press on the key: every change.
    assert!(key_record(false, Some(10_000.0), false, Some(0.0)));
    assert!(!key_record(false, Some(10_001.0), false, Some(0.0)));
    // While a client views the tab: at most once a second.
    assert!(key_record(false, None, true, Some(1000.0)));
    assert!(!key_record(false, None, true, Some(999.0)));
    assert!(key_record(false, None, true, None));
    // Nobody views it and no press: only the summary counts it.
    assert!(!key_record(false, None, false, None));
}

#[test]
fn an_image_is_logged_as_its_fnv_1a_hash() {
    assert_eq!(img_hash(""), "cbf29ce484222325");
    assert_eq!(img_hash("a"), "af63dc4c8601ec8c");
    assert_eq!(img_hash("foobar"), "85944171f73967e8");
    assert_eq!(img_hash("data:image/webp;base64,AAAA"), "89e4799c631f507f");
}

#[test]
fn a_press_is_forwarded_held_not_held_or_refused_offline() {
    let mut deck = Deck::default();
    assert!(!deck.online());
    assert_eq!(deck.press(1, 3, true), PressOutcome::Offline);
    assert_eq!(deck.holders_of(3), 0, "an offline down is not held");
    let mut deck = online();
    assert_eq!(deck.press(1, 3, true), PressOutcome::Forwarded);
    assert_eq!(deck.press(2, 3, true), PressOutcome::Held);
    assert_eq!(deck.press(1, 3, false), PressOutcome::Held);
    assert_eq!(deck.press(9, 3, false), PressOutcome::NotHeld);
    assert_eq!(deck.press(2, 3, false), PressOutcome::Forwarded);
    for (outcome, reason, ack) in [
        (PressOutcome::Forwarded, None, None),
        (PressOutcome::Held, Some("held"), Some((true, None))),
        (PressOutcome::NotHeld, Some("not held"), Some((true, None))),
        (PressOutcome::Offline, Some("offline"), Some((false, Some("offline")))),
        (PressOutcome::NoDeck, Some("no Stream Deck"), Some((false, Some("no Stream Deck")))),
    ] {
        assert_eq!((outcome.reason(), outcome.ack()), (reason, ack), "{outcome:?}");
    }
}

#[test]
fn gaps_and_holds_are_measured_per_client_and_key() {
    let mut deck = online();
    assert_eq!(deck.gap(1, 3, 1000.0), None);
    assert_eq!(deck.gap(1, 3, 1250.5), Some(250.5));
    assert_eq!(deck.gap(2, 3, 1300.0), None, "another client's own gap");
    assert_eq!(deck.gap(1, 4, 1400.0), None, "another key's own gap");
    assert_eq!(deck.forwarded(3, true, 5000.0), None);
    assert_eq!(deck.forwarded(3, false, 5120.0), Some(120.0));
    assert_eq!(deck.forwarded(3, false, 5200.0), None, "no down to measure from");
}

#[test]
fn a_detach_forgets_the_client_and_names_the_keys_to_release() {
    let mut deck = online();
    deck.heard(7, 0.0);
    deck.view(7, true);
    deck.press(7, 2, true);
    deck.press(7, 3, true);
    deck.press(8, 3, true);
    deck.gap(7, 2, 10.0);
    assert_eq!(deck.detach(7), vec![2]);
    assert_eq!(deck.viewers(), Vec::<ClientId>::new());
    assert_eq!(deck.holders_of(3), 1);
    assert_eq!(deck.gap(7, 2, 50.0), None, "its press history is gone");
}

#[test]
fn a_holding_client_is_released_after_two_silent_seconds() {
    let mut deck = online();
    deck.heard(7, 1000.0);
    deck.heard(8, 1000.0);
    deck.heard(9, 0.0);
    deck.press(7, 2, true);
    deck.press(8, 4, true);
    deck.heard(8, 2500.0);
    // 9 is silent but holds nothing.
    assert_eq!(deck.silent_clients(2999.0), vec![]);
    assert_eq!(deck.silent_clients(3000.0), vec![(7, vec![2])]);
    assert_eq!(deck.holders_of(2), 0);
    assert_eq!(deck.silent_clients(4500.0), vec![(8, vec![4])]);
    // A holder never heard (it cannot happen through ws.rs) counts as silent.
    deck.press(5, 6, true);
    assert_eq!(deck.silent_clients(4500.0), vec![(5, vec![6])]);
}

#[test]
fn a_lost_link_keeps_the_held_keys_for_a_release_once_it_is_back() {
    let mut deck = online();
    deck.press(1, 2, true);
    deck.press(2, 2, true);
    deck.press(1, 5, true);
    deck.forwarded(2, true, 10.0);
    deck.link_down();
    assert!(!deck.online());
    assert_eq!(deck.holders_of(2), 0);
    assert_eq!(deck.press(1, 2, false), PressOutcome::Offline);
    assert_eq!(deck.link_up(), vec![2, 5]);
    assert!(deck.online());
    assert_eq!(deck.link_up(), Vec::<u32>::new(), "released once");
    assert_eq!(deck.forwarded(2, false, 20.0), None, "the old hold is forgotten");
}

#[test]
fn the_stop_names_every_held_key() {
    let mut deck = online();
    deck.press(1, 9, true);
    deck.press(2, 9, true);
    deck.press(2, 1, true);
    assert_eq!(deck.stop(), vec![1, 9]);
    assert_eq!(deck.stop(), Vec::<u32>::new());
}

#[test]
fn key_states_merge_and_viewers_get_the_whole_cache() {
    let mut deck = online();
    let (key, _) = deck.apply(&update(3, Some("data:a"), Some("#000000"), Some(false)), 0.0);
    assert_eq!(
        key,
        DeckKey {
            key: 3,
            img: Some("data:a".into()),
            color: Some("#000000".into()),
            pressed: false
        }
    );
    // A missing field keeps its old value.
    let (key, _) = deck.apply(&update(3, None, Some("#ff0000"), None), 1.0);
    assert_eq!(
        (key.img.as_deref(), key.color.as_deref(), key.pressed),
        (Some("data:a"), Some("#ff0000"), false)
    );
    deck.apply(&update(0, Some("data:b"), None, Some(true)), 2.0);
    assert_eq!(deck.view(4, true).unwrap().iter().map(|k| k.key).collect::<Vec<_>>(), vec![0, 3]);
    assert_eq!(deck.viewers(), vec![4]);
    assert_eq!(deck.view(4, false), None);
    assert_eq!(deck.viewers(), Vec::<ClientId>::new());
    // KEYS-CLEAR: every cached key black and released.
    let cleared = deck.clear();
    assert_eq!(
        cleared,
        vec![
            DeckKey { key: 0, img: None, color: Some("#000000".into()), pressed: false },
            DeckKey { key: 3, img: None, color: Some("#000000".into()), pressed: false },
        ]
    );
}

#[test]
fn a_key_record_is_due_by_its_rules_and_counts_the_changes_it_stands_for() {
    let mut deck = online();
    // Nobody views, no press: only counted.
    assert_eq!(deck.apply(&update(1, Some("data:x"), None, Some(false)), 0.0).1, None);
    assert_eq!(deck.apply(&update(1, Some("data:y"), None, None), 10.0).1, None);
    // The pressed flag changes: recorded, with the changes since the last record.
    let record = deck.apply(&update(1, None, None, Some(true)), 20.0).1.unwrap();
    assert_eq!(
        record,
        json!({"key": 1, "pressed": true, "color": null, "img_hash": img_hash("data:y"),
               "img_bytes": 6, "changes": 3})
    );
    // After a press on the key every change for 10 s.
    deck.forwarded(1, true, 100.0);
    assert!(deck.apply(&update(1, Some("data:z"), None, None), 10_100.0).1.is_some());
    assert!(deck.apply(&update(1, Some("data:w"), None, None), 10_101.0).1.is_none());
    // While a client views the tab: once a second at most.
    deck.view(2, true);
    let first = deck.apply(&update(1, Some("data:v"), None, None), 11_102.0).1.unwrap();
    assert_eq!(first["changes"], 2);
    assert!(deck.apply(&update(1, Some("data:u"), None, None), 11_500.0).1.is_none());
    assert!(deck.apply(&update(1, Some("data:t"), None, None), 12_102.0).1.is_some());
}

#[test]
fn the_summary_comes_every_minute_while_the_link_is_up() {
    let mut deck = online();
    deck.apply(&update(1, Some("data:x"), None, None), 0.0);
    deck.apply(&update(1, Some("data:y"), None, None), 1.0);
    deck.apply(&update(4, Some("data:z"), None, None), 2.0);
    assert_eq!(deck.summary(59_999.0), None);
    assert_eq!(deck.summary(60_000.0), Some(json!({"changes": {"1": 2, "4": 1}})));
    assert_eq!(deck.summary(119_999.0), None);
    assert_eq!(deck.summary(120_000.0), Some(json!({"changes": {}})));
    deck.link_down();
    assert_eq!(deck.summary(500_000.0), None, "none while the link is down");
}

#[test]
fn the_record_fields() {
    let record = PressRecord {
        client: 7,
        peer: Some("10.0.0.5"),
        key: 3,
        down: false,
        seq: 12,
        t: 1_000.5,
        hub_ms: 1_250.5,
        offset_ms: Some(200.0),
        gap_ms: Some(180.0),
        hold_ms: Some(175.0),
        why: Some("up"),
        hub_hold_ms: Some(172.5),
        outcome: PressOutcome::Forwarded,
        holders: 0,
    };
    assert_eq!(
        press_fields(&record),
        json!({"client": 7, "peer": "10.0.0.5", "key": 3, "down": false, "seq": 12,
               "t": 1_000.5, "hub_ms": 1_250.5, "offset_ms": 200.0, "delay_ms": 50.0,
               "gap_ms": 180.0, "hold_ms": 175.0, "why": "up", "hub_hold_ms": 172.5,
               "forwarded": true, "reason": null, "holders": 0})
    );
    let held = PressRecord {
        client: 8,
        peer: None,
        key: 3,
        down: true,
        seq: 1,
        t: 0.0,
        hub_ms: 10.0,
        offset_ms: None,
        gap_ms: None,
        hold_ms: None,
        why: None,
        hub_hold_ms: None,
        outcome: PressOutcome::Held,
        holders: 2,
    };
    let fields = press_fields(&held);
    assert_eq!((fields["forwarded"].clone(), fields["reason"].clone()), (json!(false), json!("held")));
    assert_eq!(fields["delay_ms"], json!(null));
    let answer = Answer {
        press: Press { key: 3, down: true, from: Some((7, 12)) },
        ok: false,
        error: Some("Invalid KEY".into()),
        rtt_ms: Some(3.5),
    };
    assert_eq!(
        ok_fields(&answer),
        json!({"client": 7, "seq": 12, "key": 3, "down": true, "ok": false,
               "error": "Invalid KEY", "rtt_ms": 3.5})
    );
    let own = Answer::offline(Press { key: 5, down: false, from: None });
    assert_eq!(
        ok_fields(&own),
        json!({"client": null, "seq": null, "key": 5, "down": false, "ok": false,
               "error": "offline", "rtt_ms": null})
    );
    assert_eq!(
        release_fields(Some(7), 3, "silent", Some(2010.0)),
        json!({"client": 7, "key": 3, "reason": "silent", "hub_hold_ms": 2010.0})
    );
    assert_eq!(
        link_fields("up", Some("5.0.7"), Some("1.12.0"), None, Some(812.5), Some(3)),
        json!({"state": "up", "companion": "5.0.7", "api": "1.12.0", "error": null,
               "down_ms": 812.5, "attempts": 3})
    );
    assert_eq!(view_fields(4, true), json!({"client": 4, "on": true}));
    assert_eq!(
        key_fields(&DeckKey { key: 2, img: None, color: Some("#00aa00".into()), pressed: true }, 1),
        json!({"key": 2, "pressed": true, "color": "#00aa00", "img_hash": null, "img_bytes": null,
               "changes": 1})
    );
}
```

- [ ] **Step 2: Implement `holders.rs`** — `crates/fohmixer-hub/src/deck/holders.rs`:

```rust
//! Who holds which Stream Deck key (#52, spec §5), pure: a key's set of
//! clients. Companion sees one down when the first client puts a finger on a
//! key and one up when the last lifts it, so two fingers or two tablets on
//! one key never press it twice nor release it early.

use std::collections::{BTreeMap, BTreeSet};

use crate::live::subs::ClientId;

/// What a press does to Companion's key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hold {
    /// Forward it: the first down, or the last up.
    Forward,
    /// Companion already holds the key (a down), or still does (an up).
    Held,
    /// An up from a client that does not hold the key.
    NotHeld,
}

/// Each held key's clients.
#[derive(Debug, Default)]
pub struct Holders {
    keys: BTreeMap<u32, BTreeSet<ClientId>>,
}

impl Holders {
    /// `client` puts a finger on `key`.
    pub fn down(&mut self, key: u32, client: ClientId) -> Hold {
        let set = self.keys.entry(key).or_default();
        let first = set.is_empty();
        set.insert(client);
        if first { Hold::Forward } else { Hold::Held }
    }

    /// `client` lifts its finger from `key`.
    pub fn up(&mut self, key: u32, client: ClientId) -> Hold {
        let Some(set) = self.keys.get_mut(&key) else {
            return Hold::NotHeld;
        };
        if !set.remove(&client) {
            return Hold::NotHeld;
        }
        if set.is_empty() {
            self.keys.remove(&key);
            Hold::Forward
        } else {
            Hold::Held
        }
    }

    /// `client` is gone: it leaves every key; the keys that emptied, to
    /// release, in key order.
    pub fn drop_client(&mut self, client: ClientId) -> Vec<u32> {
        let held: Vec<u32> = self
            .keys
            .iter()
            .filter(|(_, set)| set.contains(&client))
            .map(|(key, _)| *key)
            .collect();
        held.into_iter()
            .filter(|key| self.up(*key, client) == Hold::Forward)
            .collect()
    }

    /// Every hold forgotten; the keys that were held, in key order.
    pub fn clear(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.keys).into_keys().collect()
    }

    /// How many clients hold `key`.
    pub fn holders(&self, key: u32) -> usize {
        self.keys.get(&key).map_or(0, BTreeSet::len)
    }

    /// Every client holding a key.
    pub fn clients(&self) -> BTreeSet<ClientId> {
        self.keys.values().flatten().copied().collect()
    }
}
```

- [ ] **Step 3: Implement `deck.rs`** — `crates/fohmixer-hub/src/deck.rs`:

```rust
//! The router's Stream Deck state (#52, spec §5, §8), pure: whether
//! Companion's link is up, Companion's keys as the pages get them, the
//! clients viewing the tab, who holds which key ([`holders`]), when each
//! client was last heard (a holding client silent for 2 s is released), the
//! keys held when the link went (released once it is back: Companion keeps
//! a held key held when its surface goes away), and when a key's change is
//! worth a `deck_key` record (its pressed flag; 10 s after a press on it;
//! once a second while viewed; a `deck_keys` summary every minute). The
//! router (`router/deck.rs`) carries the decisions out. Times are the
//! router's clock (ms).

pub mod holders;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Duration;

use fohmixer_proto::client::DeckKey;
use serde_json::{Value, json};

pub use holders::{Hold, Holders};

use crate::clock::one_way_delay;
use crate::companion::{Answer, KeyUpdate};
use crate::live::subs::ClientId;

/// A holding client silent this long (ms) is released.
pub const SILENT_MS: f64 = 2000.0;
/// Every change of a key is recorded this long (ms) after a press on it.
pub const PRESS_WINDOW_MS: f64 = 10_000.0;
/// While a client views the tab, a key's changes are recorded this often at
/// most (ms).
pub const KEY_LOG_EVERY_MS: f64 = 1000.0;
/// The `deck_keys` summary comes this often (ms) while the link is up.
pub const SUMMARY_EVERY_MS: f64 = 60_000.0;
/// The router's Stream Deck tick (the silence, the summary).
pub const TICK: Duration = Duration::from_millis(100);
/// The answer to a press on a hub without `[companion]`.
pub const NO_DECK: &str = "no Stream Deck";
/// A cleared key's colour.
pub const BLACK: &str = "#000000";

/// Whether a holding client heard nothing for `since_ms` is silent.
pub fn silent(since_ms: f64) -> bool {
    since_ms >= SILENT_MS
}

/// Whether a key pressed `since_ms` ago is still in its press window.
pub fn in_press_window(since_ms: f64) -> bool {
    since_ms <= PRESS_WINDOW_MS
}

/// Whether a viewed key's record is due, its last one `since_logged_ms` ago.
pub fn key_log_due(since_logged_ms: Option<f64>) -> bool {
    since_logged_ms.is_none_or(|ms| ms >= KEY_LOG_EVERY_MS)
}

/// Whether the summary is due, the last one `since_ms` ago.
pub fn summary_due(since_ms: f64) -> bool {
    since_ms >= SUMMARY_EVERY_MS
}

/// Whether a key's change is a `deck_key` record now: its pressed flag
/// changed; or a press on it came at most 10 s ago; or a client views the
/// tab and the key's last record is at least 1 s old.
pub fn key_record(
    pressed_changed: bool,
    since_press_ms: Option<f64>,
    viewing: bool,
    since_logged_ms: Option<f64>,
) -> bool {
    pressed_changed
        || since_press_ms.is_some_and(in_press_window)
        || (viewing && key_log_due(since_logged_ms))
}

/// FNV-1a (64 bit) of `text` as 16 hex digits: a key image's identity in the
/// event log (the image itself never is).
pub fn img_hash(text: &str) -> String {
    let hash = text.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    });
    format!("{hash:016x}")
}

/// What became of a page's press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PressOutcome {
    /// Written to Companion: Companion's answer acks it.
    Forwarded,
    /// Companion is offline: refused, never sent later.
    Offline,
    /// Companion already holds the key (a down) or still does (an up).
    Held,
    /// An up from a page that does not hold the key.
    NotHeld,
    /// The hub has no `[companion]`.
    NoDeck,
}

impl PressOutcome {
    /// The `reason` of its `deck_press` record (none: forwarded).
    pub fn reason(self) -> Option<&'static str> {
        match self {
            Self::Forwarded => None,
            Self::Offline => Some("offline"),
            Self::Held => Some("held"),
            Self::NotHeld => Some("not held"),
            Self::NoDeck => Some(NO_DECK),
        }
    }

    /// The page's ack of a press not forwarded (`ok`, the error), at once;
    /// none for a forwarded one.
    pub fn ack(self) -> Option<(bool, Option<&'static str>)> {
        match self {
            Self::Forwarded => None,
            Self::Held | Self::NotHeld => Some((true, None)),
            Self::Offline => Some((false, Some("offline"))),
            Self::NoDeck => Some((false, Some(NO_DECK))),
        }
    }
}

/// A key's log state: its last press, its last record, the changes since
/// that record and since the last summary.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct KeyLog {
    pressed_at: Option<f64>,
    logged_at: Option<f64>,
    unlogged: u32,
    summary: u32,
}

/// The Stream Deck as the router knows it.
#[derive(Debug, Default)]
pub struct Deck {
    online: bool,
    keys: BTreeMap<u32, DeckKey>,
    logs: BTreeMap<u32, KeyLog>,
    viewers: BTreeSet<ClientId>,
    holders: Holders,
    stale: BTreeSet<u32>,
    heard: HashMap<ClientId, f64>,
    down_at: BTreeMap<u32, f64>,
    last_press: HashMap<(ClientId, u32), f64>,
    summary_at: f64,
}

impl Deck {
    /// Whether Companion's link is up.
    pub fn online(&self) -> bool {
        self.online
    }

    /// Something came from `client` at `now`.
    pub fn heard(&mut self, client: ClientId, now: f64) {
        self.heard.insert(client, now);
    }

    /// `client` opened (`on`: the whole cache, in key order, for it) or
    /// closed the tab.
    pub fn view(&mut self, client: ClientId, on: bool) -> Option<Vec<DeckKey>> {
        if on {
            self.viewers.insert(client);
            Some(self.keys.values().cloned().collect())
        } else {
            self.viewers.remove(&client);
            None
        }
    }

    /// The clients viewing the tab.
    pub fn viewers(&self) -> Vec<ClientId> {
        self.viewers.iter().copied().collect()
    }

    /// A press of `client`: not forwarded (and not held) while offline.
    pub fn press(&mut self, client: ClientId, key: u32, down: bool) -> PressOutcome {
        if !self.online {
            return PressOutcome::Offline;
        }
        let hold = if down {
            self.holders.down(key, client)
        } else {
            self.holders.up(key, client)
        };
        match hold {
            Hold::Forward => PressOutcome::Forwarded,
            Hold::Held => PressOutcome::Held,
            Hold::NotHeld => PressOutcome::NotHeld,
        }
    }

    /// How many clients hold `key`.
    pub fn holders_of(&self, key: u32) -> usize {
        self.holders.holders(key)
    }

    /// The time since `client`'s previous press of `key` (hub ms); this one
    /// remembered.
    pub fn gap(&mut self, client: ClientId, key: u32, hub_ms: f64) -> Option<f64> {
        self.last_press
            .insert((client, key), hub_ms)
            .map(|before| hub_ms - before)
    }

    /// A press of `key` went to Companion at `now`: a down starts its hold,
    /// an up ends it (the hold's length, when its down went this session).
    pub fn forwarded(&mut self, key: u32, down: bool, now: f64) -> Option<f64> {
        self.logs.entry(key).or_default().pressed_at = Some(now);
        if down {
            self.down_at.insert(key, now);
            None
        } else {
            self.down_at.remove(&key).map(|at| now - at)
        }
    }

    /// `client` is gone: it is forgotten; the keys only it held, to release.
    pub fn detach(&mut self, client: ClientId) -> Vec<u32> {
        self.viewers.remove(&client);
        self.heard.remove(&client);
        self.last_press.retain(|(c, _), _| *c != client);
        self.holders.drop_client(client)
    }

    /// The holding clients silent at `now`, each with the keys only it held
    /// (to release); they hold nothing afterwards.
    pub fn silent_clients(&mut self, now: f64) -> Vec<(ClientId, Vec<u32>)> {
        let quiet: Vec<ClientId> = self
            .holders
            .clients()
            .into_iter()
            .filter(|c| self.heard.get(c).is_none_or(|at| silent(now - at)))
            .collect();
        quiet
            .into_iter()
            .map(|c| (c, self.holders.drop_client(c)))
            .collect()
    }

    /// The link went down: offline; the held keys kept to release once it is
    /// back.
    pub fn link_down(&mut self) {
        self.online = false;
        let held = self.holders.clear();
        self.stale.extend(held);
        self.down_at.clear();
    }

    /// The link is up: online; the keys held when it went down, to release
    /// now.
    pub fn link_up(&mut self) -> Vec<u32> {
        self.online = true;
        std::mem::take(&mut self.stale).into_iter().collect()
    }

    /// The hub stops: every held key, to release.
    pub fn stop(&mut self) -> Vec<u32> {
        self.holders.clear()
    }

    /// A key's new state at `now`: the key as the pages get it, and its
    /// `deck_key` record when one is due.
    pub fn apply(&mut self, update: &KeyUpdate, now: f64) -> (DeckKey, Option<Value>) {
        let cached = self.keys.entry(update.key).or_insert_with(|| DeckKey {
            key: update.key,
            img: None,
            color: None,
            pressed: false,
        });
        let was_pressed = cached.pressed;
        if let Some(img) = &update.img {
            cached.img = Some(img.clone());
        }
        if let Some(color) = &update.color {
            cached.color = Some(color.clone());
        }
        if let Some(pressed) = update.pressed {
            cached.pressed = pressed;
        }
        let key = cached.clone();
        let viewing = !self.viewers.is_empty();
        let log = self.logs.entry(update.key).or_default();
        log.unlogged += 1;
        log.summary += 1;
        let due = key_record(
            key.pressed != was_pressed,
            log.pressed_at.map(|at| now - at),
            viewing,
            log.logged_at.map(|at| now - at),
        );
        let record = due.then(|| {
            let fields = key_fields(&key, log.unlogged);
            log.logged_at = Some(now);
            log.unlogged = 0;
            fields
        });
        (key, record)
    }

    /// `KEYS-CLEAR`: every cached key black and released; them.
    pub fn clear(&mut self) -> Vec<DeckKey> {
        for key in self.keys.values_mut() {
            key.img = None;
            key.color = Some(BLACK.to_string());
            key.pressed = false;
        }
        self.keys.values().cloned().collect()
    }

    /// The `deck_keys` summary when due (every minute while the link is up):
    /// each key's changes since the last one.
    pub fn summary(&mut self, now: f64) -> Option<Value> {
        if !(self.online && summary_due(now - self.summary_at)) {
            return None;
        }
        self.summary_at = now;
        let changes: BTreeMap<String, u32> = self
            .logs
            .iter_mut()
            .filter(|(_, log)| log.summary > 0)
            .map(|(key, log)| (key.to_string(), std::mem::take(&mut log.summary)))
            .collect();
        Some(json!({"changes": changes}))
    }
}

/// A `deck_press` record's facts (spec §8).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PressRecord<'a> {
    pub client: ClientId,
    pub peer: Option<&'a str>,
    pub key: u32,
    pub down: bool,
    pub seq: u64,
    pub t: f64,
    pub hub_ms: f64,
    pub offset_ms: Option<f64>,
    pub gap_ms: Option<f64>,
    pub hold_ms: Option<f64>,
    pub why: Option<&'a str>,
    pub hub_hold_ms: Option<f64>,
    pub outcome: PressOutcome,
    pub holders: usize,
}

/// The `deck_press` fields: the page's press, its delay as a `set`'s, the
/// page's and the hub's hold, what became of it.
pub fn press_fields(p: &PressRecord<'_>) -> Value {
    json!({
        "client": p.client,
        "peer": p.peer,
        "key": p.key,
        "down": p.down,
        "seq": p.seq,
        "t": p.t,
        "hub_ms": p.hub_ms,
        "offset_ms": p.offset_ms,
        "delay_ms": one_way_delay(p.hub_ms, p.t, p.offset_ms),
        "gap_ms": p.gap_ms,
        "hold_ms": p.hold_ms,
        "why": p.why,
        "hub_hold_ms": p.hub_hold_ms,
        "forwarded": p.outcome == PressOutcome::Forwarded,
        "reason": p.outcome.reason(),
        "holders": p.holders,
    })
}

/// The `deck_ok` fields of Companion's answer (a hub-made release has no
/// client and no seq).
pub fn ok_fields(answer: &Answer) -> Value {
    let (client, seq) = answer
        .press
        .from
        .map_or((None, None), |(client, seq)| (Some(client), Some(seq)));
    json!({
        "client": client,
        "seq": seq,
        "key": answer.press.key,
        "down": answer.press.down,
        "ok": answer.ok,
        "error": answer.error,
        "rtt_ms": answer.rtt_ms,
    })
}

/// The `deck_release` fields of a release the hub made itself (`reason`:
/// detach, silent, reconnect, stop).
pub fn release_fields(client: Option<ClientId>, key: u32, reason: &str, hub_hold_ms: Option<f64>) -> Value {
    json!({"client": client, "key": key, "reason": reason, "hub_hold_ms": hub_hold_ms})
}

/// The `deck_link` fields: `state` up, down or refused.
pub fn link_fields(
    state: &str,
    companion: Option<&str>,
    api: Option<&str>,
    error: Option<&str>,
    down_ms: Option<f64>,
    attempts: Option<u64>,
) -> Value {
    json!({
        "state": state,
        "companion": companion,
        "api": api,
        "error": error,
        "down_ms": down_ms,
        "attempts": attempts,
    })
}

/// The `deck_key` fields: the image as its hash and size only.
pub fn key_fields(key: &DeckKey, changes: u32) -> Value {
    json!({
        "key": key.key,
        "pressed": key.pressed,
        "color": key.color,
        "img_hash": key.img.as_deref().map(img_hash),
        "img_bytes": key.img.as_ref().map(String::len),
        "changes": changes,
    })
}

/// The `deck_view` fields.
pub fn view_fields(client: ClientId, on: bool) -> Value {
    json!({"client": client, "on": on})
}

#[cfg(test)]
mod tests;
```

(`deck.silent_clients` is the spec's silence check; the test file calls it by that name.)

- [ ] **Step 4: Format** — `cargo fmt --all`.

- [ ] **Step 5: Verify** — at Checkpoint A: CI `test` runs `deck::tests::*`; `mutation` judges every function of `deck.rs` and `holders.rs` (the thresholds pinned at 1999/2000, 10 000/next float, 999.999/1000, 59 999/60 000; the hash's test vectors; `gap`'s subtraction by 250.5; `forwarded`'s by 120).

- [ ] **Step 6: Commit**

```bash
git add crates/fohmixer-hub/src/deck.rs crates/fohmixer-hub/src/deck crates/fohmixer-hub/src/lib.rs
git commit -m "feat(hub): the Stream Deck's state for the router, pure (#52)

The holder set (one down, one up per key, whatever the fingers and
tablets), the silent-holder release at 2 s, the keys held when the link
went kept for a release once it is back, the key cache, the deck_key
record rules and the event-log field builders.

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---
### Task 6: The outbox carries deck keys last

**Files:**
- Modify: `crates/fohmixer-hub/src/outbox.rs` (doc :1-21, `Inner` :33-42, `Inner::is_empty` :59-66, new methods after `ack` :124-134, `take` :178-204)
- Test: `crates/fohmixer-hub/src/outbox.rs` `mod tests`

**Interfaces:**
- Consumes: `fohmixer_proto::client::DeckKey` (Task 1).
- Produces: `Outbox::deck_key(&self, DeckKey)`, `Outbox::forget_deck(&self)`; `take()` ends with one `ServerMsg::DeckKeys` after the `Ack`.

- [ ] **Step 1: Write the failing tests** — in `outbox.rs` `mod tests`:

```rust
    fn deck_key(key: u32, img: &str) -> DeckKey {
        DeckKey {
            key,
            img: Some(img.to_string()),
            color: None,
            pressed: false,
        }
    }

    #[test]
    fn deck_keys_keep_the_latest_per_key_and_go_last() {
        let outbox = Outbox::new();
        outbox.deck_key(deck_key(5, "data:a"));
        outbox.deck_key(deck_key(1, "data:b"));
        outbox.deck_key(deck_key(5, "data:c"));
        outbox.ack(AckItem::applied("band|x|value", 3, None));
        outbox.value(ValueItem::value("a", json!(1), None));
        outbox.reply(result("1"));
        assert_eq!(
            outbox.take().unwrap(),
            vec![
                result("1"),
                ServerMsg::Values {
                    items: vec![ValueItem::value("a", json!(1), None)]
                },
                ServerMsg::Ack {
                    items: vec![AckItem::applied("band|x|value", 3, None)]
                },
                ServerMsg::DeckKeys {
                    items: vec![deck_key(1, "data:b"), deck_key(5, "data:c")]
                },
            ]
        );
        assert_eq!(outbox.take().unwrap(), vec![], "taken once");
    }

    #[test]
    fn a_waiting_deck_key_wakes_the_writer_and_a_closed_tab_drops_them() {
        let outbox = Outbox::new();
        outbox.deck_key(deck_key(2, "data:a"));
        assert!(!outbox.lock().is_empty(), "a deck key is something to write");
        outbox.forget_deck();
        assert!(outbox.lock().is_empty());
        assert_eq!(outbox.take().unwrap(), vec![]);
        outbox.close();
        outbox.deck_key(deck_key(2, "data:b"));
        assert_eq!(outbox.take(), None);
    }

    #[tokio::test]
    async fn the_writer_wakes_for_a_deck_key() {
        let outbox = Arc::new(Outbox::new());
        let writer = Arc::clone(&outbox);
        let batch = tokio::spawn(async move { writer.next_batch().await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        outbox.deck_key(deck_key(7, "data:a"));
        let batch = tokio::time::timeout(Duration::from_secs(2), batch)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(
            batch,
            vec![ServerMsg::DeckKeys {
                items: vec![deck_key(7, "data:a")]
            }]
        );
    }
```

(`mod tests` imports: add `use fohmixer_proto::client::DeckKey;` next to its other `use` lines; `AckItem` comes through `use super::*`.)

- [ ] **Step 2: Implement** — in `outbox.rs`:

Doc, after the acks paragraph:

```rust
//!
//! The Stream Deck's keys (#52) are coalesced too, the latest state per key,
//! and written last, after the acks: a viewer's key images never hold up a
//! fader's ack, and only a client viewing the tab gets them (the router
//! decides; a closed tab drops what waits, `forget_deck`).
```

`use fohmixer_proto::client::{AckItem, DeckKey, ServerMsg, ValueItem};`

`Inner` gains `deck: BTreeMap<u32, DeckKey>,` (after `acks`); `Inner::is_empty` gains `&& self.deck.is_empty()`.

After `pub fn ack`:

```rust
    /// A Stream Deck key's latest state (#52), written after everything else.
    pub fn deck_key(&self, key: DeckKey) {
        self.put(|inner| {
            inner.deck.insert(key.key, key);
        });
    }

    /// The client closed the tab: the keys waiting for it are dropped.
    pub fn forget_deck(&self) {
        self.lock().deck.clear();
    }
```

In `take`, after the `acks` block:

```rust
        if !inner.deck.is_empty() {
            out.push(ServerMsg::DeckKeys {
                items: std::mem::take(&mut inner.deck).into_values().collect(),
            });
        }
```

and its doc comment: `/// …, one \`values\` message, one \`ack\` message, then one \`deck_keys\` message; \`None\` once closed.`

- [ ] **Step 3: Format** — `cargo fmt --all`.

- [ ] **Step 4: Verify** — at Checkpoint A: CI `test` runs the three tests; `mutation` judges `deck_key`, `forget_deck`, `take`'s new block and `is_empty`'s new term.

- [ ] **Step 5: Commit**

```bash
git add crates/fohmixer-hub/src/outbox.rs
git commit -m "feat(hub): the outbox writes the Stream Deck's keys last (#52)

The latest state per key, after the acks; a closed tab drops what waits.

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---
### Task 7: The event log keeps the link and the hub's releases past its cap

**Files:**
- Modify: `crates/fohmixer-hub/src/events.rs` (doc :17-21, `is_warn` :166-181)
- Test: `crates/fohmixer-hub/src/events/tests.rs` `warn_class_records_are_socket_link_cap_dropped_and_errors` :94-106

**Interfaces:**
- Consumes: nothing new. Produces: `is_warn` true for `deck_link` and `deck_release`.

- [ ] **Step 1: Write the failing test** — in `events/tests.rs`, the first loop of `warn_class_records_are_socket_link_cap_dropped_and_errors` becomes `for ev in ["sock", "link", "cap", "dropped", "deck_link", "deck_release"] {`, the second `for ev in ["set", "batch", "ping", "ack", "applied", "trace", "deck_press", "deck_ok", "deck_key", "deck_keys", "deck_view"] {`, and after `assert!(!is_warn(&json!({"ev": "ack", "error": null})));` add:

```rust
    // Companion refusing a press is an error record (#52).
    assert!(is_warn(&json!({"ev": "deck_ok", "ok": false, "error": "Invalid KEY"})));
    assert!(!is_warn(&json!({"ev": "deck_ok", "ok": true, "error": null})));
```

- [ ] **Step 2: Implement** — in `events.rs`, `is_warn`'s match becomes `|| matches!(ev, "sock" | "link" | "cap" | "dropped" | "deck_link" | "deck_release")`; its doc gains "the Stream Deck's link changes and the releases the hub made itself (#52)"; the module doc's record list gains: `` the Stream Deck's (#52): `deck_link`, `deck_press`, `deck_ok`, `deck_release`, `deck_key`, `deck_keys`, `deck_view` (an image only as its hash and size). ``

- [ ] **Step 3: Format** — `cargo fmt --all`.

- [ ] **Step 4: Verify** — at Checkpoint A, CI `test` runs the test.

- [ ] **Step 5: Commit**

```bash
git add crates/fohmixer-hub/src/events.rs crates/fohmixer-hub/src/events/tests.rs
git commit -m "feat(hub): the Stream Deck's link and the hub's releases are warn-class (#52)

They survive the event log's daily cap, like a socket or Live's link.

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---
### Task 8: The router, the socket and the hub carry the Stream Deck

**Files:**
- Create: `crates/fohmixer-hub/src/router/deck.rs`
- Create: `crates/fohmixer-hub/tests/deck.rs`
- Modify: `crates/fohmixer-hub/src/router.rs` (doc :1-14, `#[path]` mods :33-34, `RouterMsg` :40-107, `Router` :143-159 and `Router::new` :162-190, `run` :195-207, `handle` :209-328)
- Modify: `crates/fohmixer-hub/src/ws.rs` (doc :1-23, `handle_text` :293-356)
- Modify: `crates/fohmixer-hub/src/lib.rs` (doc :1-25, `HubInner` :113-134, `stop` :150-162, `status` :187-234, `Hub::start` :242-321, `serve_until` :533-534, tests)
- Modify: `crates/fohmixer-hub/tests/graceful_stop.rs` (`start_with` :91, `spawn_hub` :118-144, a new test)
- Modify: `.config/nextest.toml` (the `hub-hosts` filter: `| binary_id(fohmixer-hub::deck)`)

**Interfaces:**
- Consumes: Tasks 1–7: `CompanionHandle`, `CompanionEvent`, `Press`, `Events` (`crate::companion`), `Deck`, `PressOutcome`, `PressRecord`, `press_fields`, `ok_fields`, `release_fields`, `link_fields`, `key_fields`, `view_fields`, `TICK` (`crate::deck`), `Outbox::{deck_key, forget_deck}`, `CompanionCfg`, `ClientMsg::{DeckView, DeckPress}`, `ServerMsg::{Deck, DeckAck}`.
- Produces: `RouterMsg::{DeckView { client, on }, DeckPress { client, key, down, seq, t, hold_ms, why, hub_ms, offset_ms }, Heard { client }, Deck { event }, Tick}`; `Router::with_deck(self, CompanionCfg, CompanionHandle) -> Self`; in `router/deck.rs`: `PressMsg`, `DeckIo`, `Router::{deck_attach, deck_heard, deck_view, deck_press, deck_event, deck_detach, deck_tick, deck_stop}`; `lib.rs`: `COMPANION_STOP_WAIT`, `HubInner::companion_stopped(&self)`, `HubStatus.companion` filled.

- [ ] **Step 1: Write the failing tests** — `crates/fohmixer-hub/tests/deck.rs`:

```rust
//! The Stream Deck through the hub (#52, spec §5): the `deck` message on
//! attach and on Companion's link changes, the keys only to viewers,
//! KEYS-CLEAR, a press's round trip and its records, two fingers on one
//! key, the hub's own releases (a closed page socket, a holding page silent
//! for 2 s, a key held when Companion was lost, the stop), a press waiting
//! when the link goes, presses while Companion is away, and a hub without
//! `[companion]`. Against the scripted fake Companion
//! (`support/companion.rs`); host-free: it also runs in the `windows` job.

mod support;

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use fohmixer_hub::config::{CompanionCfg, Config};
use fohmixer_hub::deck::img_hash;
use fohmixer_proto::client::{ClientMsg, DeckKey, ServerMsg};
use serde_json::{Value, json};
use support::companion::{FakeCompanion, Got, Script, image, key_state};
use support::{Client, TestHub, events_in, runtime, serial};

const WAIT: Duration = Duration::from_secs(5);

fn deck_config(dir: &std::path::Path, port: u16) -> Config {
    let mut config = Config::defaults(dir);
    config.instances.clear();
    config.companion = Some(CompanionCfg {
        host: "127.0.0.1".into(),
        port,
        columns: 8,
        rows: 4,
        bitmap_px: 72,
        title: "Stream Deck".into(),
    });
    config
}

/// Waits for the `deck` message saying `online`.
async fn deck_online(client: &mut Client, online: bool) {
    client
        .wait(Duration::from_secs(8), |m| match m {
            ServerMsg::Deck {
                online: now,
                columns: 8,
                rows: 4,
                title,
            } if *now == online && title == "Stream Deck" => Some(()),
            _ => None,
        })
        .await;
}

/// A press of `key` (an up with the page's 150 ms hold, why `up`).
async fn press(client: &mut Client, key: u32, down: bool, seq: u64) {
    client
        .send(&ClientMsg::DeckPress {
            key,
            down,
            seq,
            t: 1_000.0 + seq as f64,
            hold_ms: (!down).then_some(150.0),
            why: (!down).then(|| "up".to_string()),
        })
        .await;
}

/// The `deck_ack` of `seq`: ok, error, round trip.
async fn ack(client: &mut Client, seq: u64) -> (bool, Option<String>, Option<f64>) {
    client
        .wait(WAIT, move |m| match m {
            ServerMsg::DeckAck {
                seq: s,
                ok,
                error,
                rtt_ms,
            } if *s == seq => Some((*ok, error.clone(), *rtt_ms)),
            _ => None,
        })
        .await
}

/// The keys of the `deck_keys` messages until `enough` holds.
async fn keys_until(
    client: &mut Client,
    enough: impl Fn(&BTreeMap<u32, DeckKey>) -> bool,
) -> BTreeMap<u32, DeckKey> {
    let deadline = Instant::now() + WAIT;
    let mut keys = BTreeMap::new();
    while !enough(&keys) {
        let left = deadline.saturating_duration_since(Instant::now());
        let items = client
            .wait(left, |m| match m {
                ServerMsg::DeckKeys { items } => Some(items.clone()),
                _ => None,
            })
            .await;
        for item in items {
            keys.insert(item.key, item);
        }
    }
    keys
}

/// The press lines of `key` the fake got: (connection, down).
fn presses_in(got: &[Got], key: u32) -> Vec<(usize, bool)> {
    let field = format!(" KEY={key} PRESSED=");
    got.iter()
        .filter(|g| g.line.starts_with("KEY-PRESS "))
        .filter_map(|g| g.line.split_once(&field).map(|(_, v)| (g.conn, v == "1")))
        .collect()
}

fn of<'a>(records: &'a [Value], ev: &str) -> Vec<&'a Value> {
    records.iter().filter(|r| r["ev"] == ev).collect()
}

/// A line the fake writes as Companion: the key's state, its terminator off.
fn state_line(key: u32, pressed: bool) -> String {
    key_state("fohmixer-1", key, pressed).trim_end().to_string()
}

#[test]
fn the_deck_comes_on_attach_and_only_viewers_get_the_keys() {
    let _serial = serial();
    runtime().block_on(async {
        let fake = FakeCompanion::start(Script::companion()).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start_config(deck_config(dir.path(), fake.port)).await;
        let mut viewer = hub.client().await;
        let mut other = hub.client().await;
        deck_online(&mut viewer, true).await;
        deck_online(&mut other, true).await;
        viewer.send(&ClientMsg::DeckView { on: true }).await;
        let keys = keys_until(&mut viewer, |k| k.len() == 32).await;
        assert_eq!(
            keys[&31],
            DeckKey {
                key: 31,
                img: Some(image(31, false)),
                color: Some("#000000".into()),
                pressed: false
            }
        );
        fake.send(&state_line(5, true));
        let keys = keys_until(&mut viewer, |k| k.get(&5).is_some_and(|k| k.pressed)).await;
        assert_eq!(keys[&5].img.as_deref(), Some(image(5, true).as_str()));
        assert!(
            other
                .gets(Duration::from_millis(400), |m| matches!(m, ServerMsg::DeckKeys { .. }).then_some(()))
                .await
                .is_none(),
            "a client not viewing the tab gets no key"
        );
        fake.send("KEYS-CLEAR DEVICEID=\"fohmixer-1\"");
        let keys = keys_until(&mut viewer, |k| k.get(&5).is_some_and(|k| k.img.is_none())).await;
        assert_eq!(
            keys[&5],
            DeckKey {
                key: 5,
                img: None,
                color: Some("#000000".into()),
                pressed: false
            }
        );
        viewer.send(&ClientMsg::DeckView { on: false }).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        viewer.clear();
        fake.send(&state_line(6, true));
        assert!(
            viewer
                .gets(Duration::from_millis(400), |m| matches!(m, ServerMsg::DeckKeys { .. }).then_some(()))
                .await
                .is_none(),
            "a closed tab gets no key"
        );
        let status = hub.status().await.companion.expect("a hub with [companion]");
        assert!(status.online);
        assert_eq!((status.api_version.as_deref(), status.keys), (Some("1.12.0"), 32));
        let records = hub
            .events_until(WAIT, |r| r.iter().any(|e| e["ev"] == "deck_view" && e["on"] == false))
            .await;
        let link = of(&records, "deck_link");
        assert_eq!(
            (link[0]["state"].clone(), link[0]["companion"].clone(), link[0]["api"].clone(), link[0]["attempts"].clone(), link[0]["down_ms"].clone()),
            (json!("up"), json!("5.0.7+fake"), json!("1.12.0"), json!(1), Value::Null)
        );
        let pressed = of(&records, "deck_key")
            .into_iter()
            .find(|r| r["key"] == 5 && r["pressed"] == true)
            .expect("key 5's pressed change is recorded");
        assert_eq!(pressed["img_hash"], json!(img_hash(&image(5, true))));
        assert_eq!(pressed["img_bytes"], json!(image(5, true).len()));
        assert_eq!(of(&records, "deck_view").len(), 2);
        hub.stop().await;
    });
}

#[test]
fn a_press_goes_to_companion_and_its_ack_carries_the_round_trip() {
    let _serial = serial();
    runtime().block_on(async {
        let fake = FakeCompanion::start(Script::companion()).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start_config(deck_config(dir.path(), fake.port)).await;
        let mut client = hub.client().await;
        deck_online(&mut client, true).await;
        press(&mut client, 3, true, 1).await;
        let (ok, error, rtt) = ack(&mut client, 1).await;
        assert!(ok && error.is_none(), "{error:?}");
        assert!(rtt.is_some_and(|ms| (0.0..1000.0).contains(&ms)), "{rtt:?}");
        press(&mut client, 3, false, 2).await;
        assert!(ack(&mut client, 2).await.0);
        assert_eq!(presses_in(&fake.got(), 3), vec![(1, true), (1, false)]);
        let records = hub
            .events_until(WAIT, |r| r.iter().filter(|e| e["ev"] == "deck_ok").count() == 2)
            .await;
        let presses = of(&records, "deck_press");
        assert_eq!(presses.len(), 2);
        assert_eq!(
            (presses[0]["key"].clone(), presses[0]["down"].clone(), presses[0]["seq"].clone(), presses[0]["forwarded"].clone(), presses[0]["reason"].clone(), presses[0]["holders"].clone(), presses[0]["hold_ms"].clone()),
            (json!(3), json!(true), json!(1), json!(true), Value::Null, json!(1), Value::Null)
        );
        assert_eq!(
            (presses[1]["down"].clone(), presses[1]["holders"].clone(), presses[1]["hold_ms"].clone(), presses[1]["why"].clone(), presses[1]["t"].clone()),
            (json!(false), json!(0), json!(150.0), json!("up"), json!(1002.0))
        );
        assert!(presses[1]["hub_hold_ms"].as_f64().is_some_and(|ms| ms >= 0.0));
        let client_id = presses[0]["client"].clone();
        let oks = of(&records, "deck_ok");
        assert_eq!(
            oks.iter().map(|r| (r["client"].clone(), r["seq"].clone(), r["ok"].clone())).collect::<Vec<_>>(),
            vec![(client_id.clone(), json!(1), json!(true)), (client_id, json!(2), json!(true))]
        );
        assert!(oks.iter().all(|r| r["rtt_ms"].as_f64().is_some()));
        hub.stop().await;
    });
}

#[test]
fn two_fingers_on_one_key_give_companion_one_down_and_one_up() {
    let _serial = serial();
    runtime().block_on(async {
        let fake = FakeCompanion::start(Script::companion()).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start_config(deck_config(dir.path(), fake.port)).await;
        let mut a = hub.client().await;
        let mut b = hub.client().await;
        let mut c = hub.client().await;
        deck_online(&mut a, true).await;
        press(&mut a, 4, true, 1).await;
        assert!(ack(&mut a, 1).await.2.is_some(), "forwarded: Companion's round trip");
        press(&mut b, 4, true, 1).await;
        assert_eq!(ack(&mut b, 1).await, (true, None, None));
        press(&mut a, 4, false, 2).await;
        assert_eq!(ack(&mut a, 2).await, (true, None, None));
        press(&mut c, 4, false, 1).await;
        assert_eq!(ack(&mut c, 1).await, (true, None, None));
        press(&mut b, 4, false, 2).await;
        assert!(ack(&mut b, 2).await.2.is_some());
        assert_eq!(presses_in(&fake.got(), 4), vec![(1, true), (1, false)]);
        let records = hub
            .events_until(WAIT, |r| r.iter().filter(|e| e["ev"] == "deck_press").count() == 5)
            .await;
        let seen: Vec<(Value, Value)> = of(&records, "deck_press")
            .iter()
            .map(|r| (r["reason"].clone(), r["holders"].clone()))
            .collect();
        assert_eq!(
            seen,
            vec![
                (Value::Null, json!(1)),
                (json!("held"), json!(2)),
                (json!("held"), json!(1)),
                (json!("not held"), json!(1)),
                (Value::Null, json!(0)),
            ]
        );
        hub.stop().await;
    });
}

#[test]
fn a_closed_page_socket_releases_its_keys() {
    let _serial = serial();
    runtime().block_on(async {
        let fake = FakeCompanion::start(Script::companion()).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start_config(deck_config(dir.path(), fake.port)).await;
        let mut a = hub.client().await;
        deck_online(&mut a, true).await;
        press(&mut a, 6, true, 1).await;
        assert!(ack(&mut a, 1).await.0);
        a.close().await;
        fake.until(Duration::from_secs(2), "key 6's release", |g| {
            presses_in(g, 6) == vec![(1, true), (1, false)]
        })
        .await;
        let records = hub
            .events_until(WAIT, |r| r.iter().any(|e| e["ev"] == "deck_release"))
            .await;
        let release = of(&records, "deck_release")[0];
        assert_eq!((release["key"].clone(), release["reason"].clone()), (json!(6), json!("detach")));
        assert_eq!(release["client"], of(&records, "deck_press")[0]["client"]);
        assert!(release["hub_hold_ms"].as_f64().is_some());
        hub.stop().await;
    });
}

#[test]
fn a_silent_holding_page_is_released_after_two_seconds_not_before() {
    let _serial = serial();
    runtime().block_on(async {
        let fake = FakeCompanion::start(Script::companion()).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start_config(deck_config(dir.path(), fake.port)).await;
        let mut quiet = hub.client().await;
        let mut pinging = hub.client().await;
        deck_online(&mut quiet, true).await;
        press(&mut pinging, 8, true, 1).await;
        assert!(ack(&mut pinging, 1).await.0);
        let sent = Instant::now();
        press(&mut quiet, 7, true, 1).await;
        assert!(ack(&mut quiet, 1).await.0);
        // The pinging page keeps its key; the quiet one is released.
        for n in 0..15_u32 {
            pinging
                .send(&ClientMsg::Ping {
                    n,
                    t: 5_000.0 + f64::from(n) * 200.0,
                    rtt: None,
                    rtt_n: None,
                })
                .await;
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let got = fake.got();
        assert_eq!(presses_in(&got, 7), vec![(1, true), (1, false)]);
        let released = got
            .iter()
            .find(|g| g.line.ends_with(" KEY=7 PRESSED=0"))
            .unwrap()
            .at;
        let silence = released - sent;
        assert!(
            silence >= Duration::from_secs(2) && silence <= Duration::from_secs(3),
            "{silence:?}"
        );
        assert_eq!(presses_in(&got, 8), vec![(1, true)], "a pinging page keeps its key");
        // The quiet page's late up: taken, not forwarded.
        press(&mut quiet, 7, false, 2).await;
        assert_eq!(ack(&mut quiet, 2).await, (true, None, None));
        press(&mut pinging, 8, false, 2).await;
        assert!(ack(&mut pinging, 2).await.2.is_some());
        let records = hub
            .events_until(WAIT, |r| r.iter().filter(|e| e["ev"] == "deck_press").count() == 4)
            .await;
        let releases = of(&records, "deck_release");
        assert_eq!(releases.len(), 1);
        assert_eq!((releases[0]["key"].clone(), releases[0]["reason"].clone()), (json!(7), json!("silent")));
        assert_eq!(of(&records, "deck_press")[2]["reason"], "not held");
        hub.stop().await;
    });
}

#[test]
fn a_key_held_when_companion_was_lost_is_released_after_the_reconnect() {
    let _serial = serial();
    runtime().block_on(async {
        let fake = FakeCompanion::start(Script::companion()).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start_config(deck_config(dir.path(), fake.port)).await;
        let mut a = hub.client().await;
        deck_online(&mut a, true).await;
        press(&mut a, 2, true, 1).await;
        assert!(ack(&mut a, 1).await.0);
        fake.close();
        deck_online(&mut a, false).await;
        deck_online(&mut a, true).await;
        // Companion 5.0.7 keeps a key held across its surface's removal: the
        // new surface releases it.
        fake.until(Duration::from_secs(2), "the release on the new surface", |g| {
            presses_in(g, 2) == vec![(1, true), (2, false)]
        })
        .await;
        assert!(fake.lines_of(2)[0].starts_with("ADD-DEVICE DEVICEID=\"fohmixer-2\""));
        // The page's own up later: not held any more, not forwarded.
        press(&mut a, 2, false, 2).await;
        assert_eq!(ack(&mut a, 2).await, (true, None, None));
        assert_eq!(presses_in(&fake.got(), 2).len(), 2);
        let records = hub
            .events_until(WAIT, |r| r.iter().any(|e| e["ev"] == "deck_release"))
            .await;
        let links: Vec<Value> = of(&records, "deck_link").iter().map(|r| r["state"].clone()).collect();
        assert_eq!(links, vec![json!("up"), json!("down"), json!("up")]);
        let down = of(&records, "deck_link")[1];
        assert_eq!(down["error"], "Companion closed the connection");
        let up = of(&records, "deck_link")[2];
        assert!(up["down_ms"].as_f64().is_some_and(|ms| ms >= 200.0), "{up}");
        let release = of(&records, "deck_release")[0];
        assert_eq!(
            (release["client"].clone(), release["key"].clone(), release["reason"].clone()),
            (Value::Null, json!(2), json!("reconnect"))
        );
        hub.stop().await;
    });
}

#[test]
fn a_press_waiting_for_companion_is_answered_offline_when_the_link_goes() {
    let _serial = serial();
    runtime().block_on(async {
        let fake = FakeCompanion::start(Script {
            api: "1.12.0".into(),
            refuse_add: None,
            answers: false,
        })
        .await;
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start_config(deck_config(dir.path(), fake.port)).await;
        let mut a = hub.client().await;
        deck_online(&mut a, true).await;
        press(&mut a, 3, true, 1).await;
        assert!(
            a.gets(Duration::from_millis(300), |m| matches!(m, ServerMsg::DeckAck { .. }).then_some(()))
                .await
                .is_none(),
            "no answer from Companion yet"
        );
        fake.close();
        assert_eq!(ack(&mut a, 1).await, (false, Some("offline".to_string()), None));
        hub.stop().await;
    });
}

#[test]
fn presses_while_companion_is_away_are_refused_at_once_and_never_sent_later() {
    let _serial = serial();
    runtime().block_on(async {
        let fake = FakeCompanion::start(Script::companion()).await;
        fake.refuse(true);
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start_config(deck_config(dir.path(), fake.port)).await;
        let mut a = hub.client().await;
        deck_online(&mut a, false).await;
        let started = Instant::now();
        press(&mut a, 1, true, 1).await;
        assert_eq!(ack(&mut a, 1).await, (false, Some("offline".to_string()), None));
        assert!(started.elapsed() < Duration::from_millis(500), "at once");
        let status = hub
            .status_until(WAIT, |s| s.companion.as_ref().is_some_and(|c| c.connect_failures >= 1))
            .await
            .companion
            .unwrap();
        assert!(!status.online && status.last_error.is_some(), "{status:?}");
        fake.refuse(false);
        deck_online(&mut a, true).await;
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert!(presses_in(&fake.got(), 1).is_empty(), "a refused press is never sent later");
        let records = hub
            .events_until(WAIT, |r| r.iter().filter(|e| e["ev"] == "deck_link").count() >= 2)
            .await;
        let refused = of(&records, "deck_press")[0];
        assert_eq!((refused["forwarded"].clone(), refused["reason"].clone()), (json!(false), json!("offline")));
        let first = of(&records, "deck_link")[0];
        assert_eq!((first["state"].clone(), first["attempts"].clone()), (json!("down"), json!(1)));
        hub.stop().await;
    });
}

#[test]
fn a_hub_without_companion_sends_no_deck_and_refuses_presses() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start(vec![], dir.path()).await;
        let mut client = hub.client().await;
        assert!(
            client
                .gets(Duration::from_millis(500), |m| matches!(m, ServerMsg::Deck { .. }).then_some(()))
                .await
                .is_none()
        );
        press(&mut client, 1, true, 1).await;
        assert_eq!(ack(&mut client, 1).await, (false, Some("no Stream Deck".to_string()), None));
        assert_eq!(hub.status().await.companion, None);
        hub.stop().await;
    });
}

#[test]
fn the_stop_releases_held_keys_then_removes_the_device() {
    let _serial = serial();
    runtime().block_on(async {
        let fake = FakeCompanion::start(Script::companion()).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start_config(deck_config(dir.path(), fake.port)).await;
        let mut a = hub.client().await;
        deck_online(&mut a, true).await;
        press(&mut a, 9, true, 1).await;
        assert!(ack(&mut a, 1).await.0);
        hub.stop().await;
        let got = fake
            .until(Duration::from_secs(2), "REMOVE-DEVICE", |g| {
                g.iter().any(|l| l.line.starts_with("REMOVE-DEVICE"))
            })
            .await;
        // A ping that may fall between them left out: the release, then
        // REMOVE-DEVICE.
        let lines: Vec<&str> = got
            .iter()
            .map(|g| g.line.as_str())
            .filter(|l| !l.starts_with("PING "))
            .collect();
        assert_eq!(
            lines[lines.len() - 2..],
            [
                "KEY-PRESS DEVICEID=\"fohmixer-1\" KEY=9 PRESSED=0",
                "REMOVE-DEVICE DEVICEID=\"fohmixer-1\"",
            ]
        );
        // The stopped hub's log has the release.
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let records = events_in(dir.path());
            if let Some(release) = records.iter().find(|r| r["ev"] == "deck_release") {
                assert_eq!((release["key"].clone(), release["reason"].clone()), (json!(9), json!("stop")));
                break;
            }
            assert!(Instant::now() < deadline, "no deck_release: {records:?}");
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    });
}
```

In `crates/fohmixer-hub/src/lib.rs` `mod tests`, add (the stop waits for the task; with the mutant that skips the wait the task is still there):

```rust
    #[tokio::test]
    async fn the_stop_waits_for_the_companion_task_to_remove_its_device() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::defaults(dir.path());
        config.instances.clear();
        config.companion = Some(config::CompanionCfg {
            host: "127.0.0.1".into(),
            port,
            columns: 8,
            rows: 4,
            bitmap_px: 72,
            title: "Stream Deck".into(),
        });
        let hub = Hub::start(config).unwrap();
        let (socket, _) = listener.accept().await.unwrap();
        let (read, mut write) = socket.into_split();
        write
            .write_all(b"BEGIN CompanionVersion=\"5.0.7+fake\" ApiVersion=\"1.12.0\" \n")
            .await
            .unwrap();
        let mut lines = BufReader::new(read).lines();
        let add = lines.next_line().await.unwrap().unwrap();
        assert!(add.starts_with("ADD-DEVICE DEVICEID=\"fohmixer-1\""), "{add}");
        write.write_all(b"ADD-DEVICE OK DEVICEID=\"fohmixer-1\" \n").await.unwrap();
        let handle = hub.companion.clone().expect("a hub with [companion] has the task");
        for _ in 0..300 {
            if handle.snapshot().online {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(handle.snapshot().online);
        hub.stop();
        hub.companion_stopped().await;
        assert!(
            hub.companion_task.lock().unwrap_or_else(PoisonError::into_inner).is_none(),
            "the stop awaited the task"
        );
        let mut last = String::new();
        while let Ok(Some(line)) = lines.next_line().await {
            last = line;
        }
        assert_eq!(last, "REMOVE-DEVICE DEVICEID=\"fohmixer-1\"");
    }
```

In `crates/fohmixer-hub/tests/graceful_stop.rs`: replace `fn start_with(envs)`'s first line with `let mut server = spawn_hub("instances = []\n", envs);`, rename the helper to `start_toml(toml: &str, envs: &[(&str, &str)]) -> Server` (body unchanged but calling `spawn_hub(toml, envs)`), make `start_with(envs)` call `start_toml("instances = []\n", envs)`, and give `spawn_hub` a `toml: &str` first parameter written instead of the literal (`std::fs::write(dir.path().join("fohmixer-hub.toml"), toml).unwrap();`; other callers pass `"instances = []\n"`). Add `use std::io::BufRead;` and the test:

```rust
/// The binary's stop removes the Stream Deck from Companion (#52): the
/// stop waits (bounded) for the Companion task's `REMOVE-DEVICE` before the
/// process ends.
#[test]
fn sigterm_removes_the_stream_deck_from_companion() {
    let _turn = serial();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    // A fake Companion: BEGIN, ADD-DEVICE OK, PONG; every line it got, until
    // the hub closes the connection.
    let fake = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
        stream
            .write_all(b"BEGIN CompanionVersion=\"5.0.7+fake\" ApiVersion=\"1.12.0\" \n")
            .unwrap();
        let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
        let mut got = Vec::new();
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => return got,
                Ok(_) => {}
            }
            let line = line.trim_end().to_string();
            if line.starts_with("ADD-DEVICE") {
                stream.write_all(b"ADD-DEVICE OK DEVICEID=\"fohmixer-1\" \n").unwrap();
            }
            if let Some(n) = line.strip_prefix("PING ") {
                stream.write_all(format!("PONG {n} \n").as_bytes()).unwrap();
            }
            got.push(line);
        }
    });
    let toml = format!("instances = []\n[companion]\nhost = \"127.0.0.1\"\nport = {port}\n");
    let mut server = start_toml(&toml, &[]);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !server.log().contains("Stream Deck registered with Companion") {
        assert!(Instant::now() < deadline, "never registered: {}", server.log());
        std::thread::sleep(Duration::from_millis(50));
    }
    let since = request_stop(&server, "TERM");
    let status = exit_within(&mut server, since, Duration::from_secs(10));
    assert!(status.success(), "{status}: {}", server.log());
    let got = fake.join().unwrap();
    assert_eq!(
        got.last().map(String::as_str),
        Some("REMOVE-DEVICE DEVICEID=\"fohmixer-1\""),
        "{got:?}"
    );
}
```

- [ ] **Step 2: Implement `router/deck.rs`** — `crates/fohmixer-hub/src/router/deck.rs`:

```rust
//! The router's half of the Stream Deck (#52, spec §5): it sends each
//! client the `deck` message (on attach and on Companion's link changes),
//! the keys only to the clients viewing the tab (through their outboxes,
//! after everything else), forwards the pages' presses through the holder
//! set to the Companion task, acks each press to its page (at once, or with
//! Companion's answer), and makes the releases no page can: a closed socket
//! (`detach`), a holding page silent for 2 s (`silent`), the keys held when
//! Companion was lost (`reconnect`, released after the next `ADD-DEVICE OK`:
//! Companion keeps a held key held when its surface goes away) and the stop
//! (`stop`, before the task's `REMOVE-DEVICE`). Every hop is an event-log
//! record (`deck_link`, `deck_press`, `deck_ok`, `deck_release`, `deck_key`,
//! `deck_keys`, `deck_view`). The decisions are `crate::deck`'s.

use fohmixer_proto::client::{DeckKey, ServerMsg};

use super::Router;
use crate::companion::{CompanionEvent, CompanionHandle, Press};
use crate::config::CompanionCfg;
use crate::deck::{
    Deck, PressOutcome, PressRecord, link_fields, ok_fields, press_fields, release_fields,
    view_fields,
};
use crate::live::subs::ClientId;
use crate::outbox::Outbox;

/// A page's key press as the router takes it.
pub(super) struct PressMsg {
    pub client: ClientId,
    pub key: u32,
    pub down: bool,
    pub seq: u64,
    pub t: f64,
    pub hold_ms: Option<f64>,
    pub why: Option<String>,
    /// When it reached the hub (hub UTC ms).
    pub hub_ms: f64,
    /// The page clock's offset as of the socket's last ping.
    pub offset_ms: Option<f64>,
}

/// The Stream Deck's part of the router.
pub(super) struct DeckIo {
    cfg: CompanionCfg,
    companion: CompanionHandle,
    state: Deck,
}

impl Router {
    /// The router of a hub with `[companion]` (#52).
    #[must_use]
    pub fn with_deck(mut self, cfg: CompanionCfg, companion: CompanionHandle) -> Self {
        self.deck = Some(DeckIo {
            cfg,
            companion,
            state: Deck::default(),
        });
        self
    }

    /// The `deck` message of the Stream Deck now.
    fn deck_msg(io: &DeckIo) -> ServerMsg {
        ServerMsg::Deck {
            online: io.state.online(),
            columns: io.cfg.columns,
            rows: io.cfg.rows,
            title: io.cfg.title.clone(),
        }
    }

    /// A new client: the Stream Deck's state (none without `[companion]`:
    /// the page shows no tab), and it counts as heard.
    pub(super) fn deck_attach(&mut self, client: ClientId, outbox: &Outbox) {
        let now = self.now_ms();
        if let Some(io) = self.deck.as_mut() {
            io.state.heard(client, now);
            outbox.reply(Self::deck_msg(io));
        }
    }

    /// Something came from `client`.
    pub(super) fn deck_heard(&mut self, client: ClientId) {
        let now = self.now_ms();
        if let Some(io) = self.deck.as_mut() {
            io.state.heard(client, now);
        }
    }

    /// `client` opened the tab (the whole cache goes to it) or closed it.
    pub(super) fn deck_view(&mut self, client: ClientId, on: bool) {
        let Some(io) = self.deck.as_mut() else {
            return;
        };
        let keys = io.state.view(client, on);
        if let Some(outbox) = self.clients.get(&client) {
            match keys {
                Some(keys) => {
                    for key in keys {
                        outbox.deck_key(key);
                    }
                }
                None => outbox.forget_deck(),
            }
        }
        self.io.events.record("deck_view", view_fields(client, on));
    }

    /// A page's press: through the holder set to Companion, recorded, acked
    /// at once unless forwarded (then Companion's answer acks it).
    pub(super) fn deck_press(&mut self, p: PressMsg) {
        let now = self.now_ms();
        let (outcome, gap_ms, hub_hold_ms, holders) = match self.deck.as_mut() {
            None => (PressOutcome::NoDeck, None, None, 0),
            Some(io) => {
                let gap_ms = io.state.gap(p.client, p.key, p.hub_ms);
                let outcome = io.state.press(p.client, p.key, p.down);
                let mut hub_hold_ms = None;
                if outcome == PressOutcome::Forwarded {
                    hub_hold_ms = io.state.forwarded(p.key, p.down, now);
                    io.companion.press(Press {
                        key: p.key,
                        down: p.down,
                        from: Some((p.client, p.seq)),
                    });
                }
                (outcome, gap_ms, hub_hold_ms, io.state.holders_of(p.key))
            }
        };
        let record = PressRecord {
            client: p.client,
            peer: self.peers.get(&p.client).map(String::as_str),
            key: p.key,
            down: p.down,
            seq: p.seq,
            t: p.t,
            hub_ms: p.hub_ms,
            offset_ms: p.offset_ms,
            gap_ms,
            hold_ms: p.hold_ms,
            why: p.why.as_deref(),
            hub_hold_ms,
            outcome,
            holders,
        };
        self.io.events.record("deck_press", press_fields(&record));
        if let Some((ok, error)) = outcome.ack() {
            self.deck_ack(p.client, p.seq, ok, error.map(str::to_string), None);
        }
    }

    /// A `deck_ack` to its page (gone pages get nothing).
    fn deck_ack(&self, client: ClientId, seq: u64, ok: bool, error: Option<String>, rtt_ms: Option<f64>) {
        if let Some(outbox) = self.clients.get(&client) {
            outbox.reply(ServerMsg::DeckAck {
                seq,
                ok,
                error,
                rtt_ms,
            });
        }
    }

    /// A release the hub makes itself (`reason`: detach, silent, reconnect,
    /// stop), for `client` (none: the hub's own).
    fn deck_release(&mut self, client: Option<ClientId>, key: u32, reason: &str) {
        let now = self.now_ms();
        let Some(io) = self.deck.as_mut() else {
            return;
        };
        let hub_hold_ms = io.state.forwarded(key, false, now);
        io.companion.press(Press {
            key,
            down: false,
            from: None,
        });
        self.io
            .events
            .record("deck_release", release_fields(client, key, reason, hub_hold_ms));
        tracing::info!(client = ?client, key, reason, "the hub released a Stream Deck key");
    }

    /// A client is gone: the keys only it held are released.
    pub(super) fn deck_detach(&mut self, client: ClientId) {
        let Some(io) = self.deck.as_mut() else {
            return;
        };
        for key in io.state.detach(client) {
            self.deck_release(Some(client), key, "detach");
        }
    }

    /// The Stream Deck's tick: holding pages silent for 2 s released; the
    /// minute's summary.
    pub(super) fn deck_tick(&mut self) {
        let now = self.now_ms();
        let Some(io) = self.deck.as_mut() else {
            return;
        };
        let silent = io.state.silent_clients(now);
        let summary = io.state.summary(now);
        for (client, keys) in silent {
            for key in keys {
                self.deck_release(Some(client), key, "silent");
            }
        }
        if let Some(fields) = summary {
            self.io.events.record("deck_keys", fields);
        }
    }

    /// The Companion task's event.
    pub(super) fn deck_event(&mut self, event: CompanionEvent) {
        let now = self.now_ms();
        let Some(io) = self.deck.as_mut() else {
            return;
        };
        match event {
            CompanionEvent::Up {
                companion,
                api,
                attempts,
                down_ms,
            } => {
                let stale = io.state.link_up();
                self.io.events.record(
                    "deck_link",
                    link_fields("up", Some(&companion), Some(&api), None, down_ms, Some(attempts)),
                );
                self.deck_broadcast();
                for key in stale {
                    self.deck_release(None, key, "reconnect");
                }
            }
            CompanionEvent::Down { error } => {
                io.state.link_down();
                self.io
                    .events
                    .record("deck_link", link_fields("down", None, None, Some(&error), None, None));
                self.deck_broadcast();
            }
            CompanionEvent::Failed {
                error,
                refused,
                companion,
                api,
                attempts,
            } => {
                let state = if refused { "refused" } else { "down" };
                self.io.events.record(
                    "deck_link",
                    link_fields(state, companion.as_deref(), api.as_deref(), Some(&error), None, Some(attempts)),
                );
            }
            CompanionEvent::Key(update) => {
                let (key, record) = io.state.apply(&update, now);
                if let Some(fields) = record {
                    self.io.events.record("deck_key", fields);
                }
                self.deck_fan_out(&[key]);
            }
            CompanionEvent::Clear => {
                let keys = io.state.clear();
                self.deck_fan_out(&keys);
            }
            CompanionEvent::Answered(answer) => {
                self.io.events.record("deck_ok", ok_fields(&answer));
                if let Some((client, seq)) = answer.press.from {
                    self.deck_ack(client, seq, answer.ok, answer.error, answer.rtt_ms);
                }
            }
        }
    }

    /// Keys to the clients viewing the tab.
    fn deck_fan_out(&self, keys: &[DeckKey]) {
        let Some(io) = &self.deck else {
            return;
        };
        for client in io.state.viewers() {
            if let Some(outbox) = self.clients.get(&client) {
                for key in keys {
                    outbox.deck_key(key.clone());
                }
            }
        }
    }

    /// The `deck` message to every client (the link went up or down).
    fn deck_broadcast(&self) {
        let Some(io) = &self.deck else {
            return;
        };
        let msg = Self::deck_msg(io);
        for outbox in self.clients.values() {
            outbox.reply(msg.clone());
        }
    }

    /// The hub stops: every held key released, then the task told to stop
    /// (its `REMOVE-DEVICE` goes after the releases: one channel, in order).
    pub(super) fn deck_stop(&mut self) {
        let Some(io) = self.deck.as_mut() else {
            return;
        };
        for key in io.state.stop() {
            self.deck_release(None, key, "stop");
        }
        if let Some(io) = &self.deck {
            io.companion.stop();
        }
    }
}
```

(The borrow of `io` ends before each `self.…` call that needs all of `self`: `io` is not used after `link_up`, `link_down`, `apply`, `clear`, `detach` or `stop` in its arm or loop head; the loops iterate an owned `Vec`.)

- [ ] **Step 3: Wire the router** — in `crates/fohmixer-hub/src/router.rs`:

Doc, append: `//! The Stream Deck (#52, `deck.rs`): the router owns its state too; the Companion task's events come as messages, and `router/deck.rs` carries the decisions out.`

After `#[path = "router/writes.rs"] mod writes;` add:

```rust
#[path = "router/deck.rs"]
mod deck;
```

`RouterMsg` gains (before `Stop`):

```rust
    /// A page opened or closed the Stream Deck tab (#52).
    DeckView { client: ClientId, on: bool },
    /// A page's Stream Deck press (#52), with its arrival (hub UTC ms) and
    /// the page clock's offset as of its socket's last ping.
    DeckPress {
        client: ClientId,
        key: u32,
        down: bool,
        seq: u64,
        t: f64,
        hold_ms: Option<f64>,
        why: Option<String>,
        hub_ms: f64,
        offset_ms: Option<f64>,
    },
    /// Something came from a client (#52: a holding page silent for 2 s is
    /// released).
    Heard { client: ClientId },
    /// An event of the Companion task (#52).
    Deck { event: crate::companion::CompanionEvent },
    /// The Stream Deck's clock (#52, every `deck::TICK` with `[companion]`).
    Tick,
```

`Router` gains `deck: Option<deck::DeckIo>,` (after `started`); `Router::new` sets `deck: None,`.

In `run`, after the `while` loop and before closing the outboxes: `self.deck_stop();`.

In `handle`:
- `RouterMsg::Attach { … }`: before `self.clients.insert(client, outbox);` add `self.deck_attach(client, &outbox);`.
- `RouterMsg::Detach { client }`: first line `self.deck_detach(client);`.
- new arms before `RouterMsg::Stop => return false,`:

```rust
            RouterMsg::DeckView { client, on } => self.deck_view(client, on),
            RouterMsg::DeckPress {
                client,
                key,
                down,
                seq,
                t,
                hold_ms,
                why,
                hub_ms,
                offset_ms,
            } => self.deck_press(deck::PressMsg {
                client,
                key,
                down,
                seq,
                t,
                hold_ms,
                why,
                hub_ms,
                offset_ms,
            }),
            RouterMsg::Heard { client } => self.deck_heard(client),
            RouterMsg::Deck { event } => self.deck_event(event),
            RouterMsg::Tick => self.deck_tick(),
```

- [ ] **Step 4: Route the page's messages** — in `crates/fohmixer-hub/src/ws.rs` `handle_text`, first lines after `let outbox = &outbox;`:

```rust
    // A holding page's silence releases its Stream Deck keys (#52): every
    // message it sends counts.
    if hub.config.companion.is_some() {
        hub.route(RouterMsg::Heard { client });
    }
```

and the arms (after the `Trace` arm):

```rust
        Ok(ClientMsg::DeckView { on }) => hub.route(RouterMsg::DeckView { client, on }),
        Ok(ClientMsg::DeckPress {
            key,
            down,
            seq,
            t,
            hold_ms,
            why,
        }) => hub.route(RouterMsg::DeckPress {
            client,
            key,
            down,
            seq,
            t,
            hold_ms,
            why,
            hub_ms: crate::live::wall_ms().unwrap_or(0.0),
            offset_ms: conn.offset,
        }),
```

Doc: after "the writes (`set`, #43) go to the router;" add "the Stream Deck's `deck_view` and `deck_press` (#52) too, and with `[companion]` every message tells the router the page was heard;".

- [ ] **Step 5: Start, report and stop the task** — in `crates/fohmixer-hub/src/lib.rs`:

Doc, append: `//!\n//! The Stream Deck tab (#52, \`[companion]\`): one more Stream Deck on Bitfocus Companion's Satellite API (\`companion.rs\`), its state in the router (\`deck.rs\`), its link in \`/api/status\`.`

`pub mod deck;` (Task 5) and `pub mod companion;` (Task 3) are in place. Add `use companion::CompanionHandle;` and the constant:

```rust
/// How long the stop waits for the Companion task's `REMOVE-DEVICE` (#52; the
/// task bounds its own write at 500 ms) before it ends the task.
pub const COMPANION_STOP_WAIT: Duration = Duration::from_secs(1);
```

`HubInner` gains (after `events`):

```rust
    /// The Stream Deck's Companion task (#52, `[companion]`).
    pub(crate) companion: Option<CompanionHandle>,
    /// Its task, awaited after the stop so its `REMOVE-DEVICE` goes out.
    companion_task: Mutex<Option<JoinHandle<()>>>,
```

After `add_task`:

```rust
    /// Waits up to [`COMPANION_STOP_WAIT`] for the Companion task to end
    /// after a stop (the router tells it to, after its releases), then ends
    /// it.
    pub async fn companion_stopped(&self) {
        let task = self
            .companion_task
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(mut task) = task
            && tokio::time::timeout(COMPANION_STOP_WAIT, &mut task).await.is_err()
        {
            tracing::warn!("the Companion task did not end within 1 s of the stop: ending it");
            task.abort();
        }
    }
```

In `status`, the `HubStatus` literal gains `companion: self.companion.as_ref().map(CompanionHandle::status),`.

In `Hub::start`, after the instances' loop:

```rust
        // The Stream Deck (#52): its Companion task and the router's tick.
        let companion = config.companion.as_ref().map(|deck| {
            let tx = router_tx.clone();
            let events: companion::Events = Arc::new(move |event: companion::CompanionEvent| {
                let _ = tx.send(RouterMsg::Deck { event });
            });
            CompanionHandle::spawn(deck, events)
        });
        if config.companion.is_some() {
            tasks.push(tokio::spawn(deck_ticks(router_tx.clone(), deck::TICK)));
        }
```

the `hub started` line gains `companion = config.companion.is_some(),`; the router is built with the deck:

```rust
        let mut router = router::Router::new(
            live.clone(),
            hub_state.stage_aut,
            config.data_dir.clone(),
            router::RouterIo {
                tx: router_tx.clone(),
                events: events.clone(),
            },
        );
        if let (Some(deck), Some((handle, _))) = (&config.companion, &companion) {
            router = router.with_deck(deck.clone(), handle.clone());
        }
        tokio::spawn(router.run(router_rx));
```

and the `HubInner` literal:

```rust
        let (companion, companion_task) = match companion {
            Some((handle, task)) => (Some(handle), Some(task)),
            None => (None, None),
        };
```

(placed before `Ok(Self(Arc::new(HubInner {`), with the fields `companion,` and `companion_task: Mutex::new(companion_task),`.

After `poll_layout` add:

```rust
/// The Stream Deck's clock (#52): a tick to the router every `period`.
async fn deck_ticks(router: mpsc::UnboundedSender<RouterMsg>, period: Duration) {
    let mut tick = tokio::time::interval(period);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        if router.send(RouterMsg::Tick).is_err() {
            return;
        }
    }
}
```

In `serve_until`, after `tracing::info!("HTTP server stopped");` add `hub.companion_stopped().await;`.

- [ ] **Step 6: Format and nextest** — `cargo fmt --all`; `.config/nextest.toml` `hub-hosts` filter gains `| binary_id(fohmixer-hub::deck)` (the comment from Task 4 covers it).

- [ ] **Step 7: Playbook** — in `.claude/rules/hub-rust.md` "Design facts to keep", add:

```markdown
- The Stream Deck (#52, `companion.rs`, `companion/client.rs`, `deck.rs`, `deck/holders.rs`, `router/deck.rs`): one Companion task like a Live client (a writer task AND a reader task: the session's `select!` awaits no socket; lines read with `take(MAX_LINE + 1).read_until`, 256 KiB). Companion 5.0.7 facts the code rests on (the plan `docs/superpowers/plans/2026-10-06-streamdeck-tab.md` has the probe): every line ends with a space; a bare `ERROR MESSAGE=` answers no press (only `KEY-PRESS OK|ERROR` feed the `Fifo`); it closes a socket silent for 5 s (the 2 s ping is required); it keeps a key held when its surface goes away, so the router releases the keys held at a link loss after the next `ADD-DEVICE OK` (`deck_release reason=reconnect`) and at the stop before `REMOVE-DEVICE`; a release of a key it does not hold is a no-op. A fresh `DEVICEID` per connection (`fohmixer-<attempt>`), one `SERIAL` (`fohmixer`). The router owns the holders (one down/up per key whatever the fingers), the viewers (keys only to them, `Outbox::deck_key` written last, `forget_deck` on a closed tab), the 2 s silent-holder release (`RouterMsg::Heard` from every page message, `RouterMsg::Tick` every 100 ms) and the `deck_key` rules (pressed change, 10 s after a press, 1/s per key while viewed, a `deck_keys` summary a minute; images only as FNV-1a hash and size). The stop: the router's `deck_stop` (releases, then `CompanionHandle::stop`), and `serve_until` awaits `HubInner::companion_stopped` (bounded 1 s).
```

and in "Test harness":

```markdown
- The Stream Deck's tests (#52) use the scripted fake Companion `tests/support/companion.rs` (a tokio listener; `Script` sets the API version, an `ADD-DEVICE` refusal, silence; `refuse(true)` closes new connections at once; lines kept with their connection and time): `tests/companion_client.rs` drives `CompanionHandle` directly, `tests/deck.rs` the whole hub. Both are host-free (also in the `windows` job) and in the `hub-hosts` nextest group (timing checks: 2 s pings, the 2 s silence, 5 s of Companion's silence). `graceful_stop.rs` `sigterm_removes_the_stream_deck_from_companion` proves the binary's stop waits for `REMOVE-DEVICE`.
```

- [ ] **Step 8: Verify** — at Checkpoint A: CI `test` runs `tests/deck.rs` (9 tests), `companion_client`, `lib::tests::the_stop_waits_for_the_companion_task_to_remove_its_device`, `graceful_stop::sigterm_removes_the_stream_deck_from_companion`; `windows` runs `deck` and `companion_client`; `lint` (clippy `-D warnings`, native) passes; the existing `e2e` job is green (no `[companion]` there yet); `mutation` judges `router/deck.rs`, the new `RouterMsg` arms, `ws.rs`'s routing, `lib.rs`'s `companion_stopped`, `deck_ticks` and the start's wiring (each covered above).

- [ ] **Step 9: Commit**

```bash
git add crates/fohmixer-hub/src/router.rs crates/fohmixer-hub/src/router/deck.rs crates/fohmixer-hub/src/ws.rs \
  crates/fohmixer-hub/src/lib.rs crates/fohmixer-hub/tests/deck.rs crates/fohmixer-hub/tests/graceful_stop.rs \
  .config/nextest.toml .claude/rules/hub-rust.md
git commit -m "feat(hub): the router carries the Stream Deck (#52)

The deck message on attach and on Companion's link changes, the keys only
to the clients viewing the tab, presses through the holder set to
Companion and acked to their page, and the releases only the hub can
make: a closed page socket, a holding page silent for 2 s, the keys held
when Companion was lost (Companion 5.0.7 keeps them held), and the stop,
before REMOVE-DEVICE. The companion block of /api/status.

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---
### Task 9: The page's store holds the Stream Deck

**Files:**
- Create: `crates/fohmixer-ui/src/store/deck.rs`
- Create: `crates/fohmixer-ui/src/store/live/deck.rs`
- Modify: `crates/fohmixer-ui/src/store.rs` :26-28 (`pub mod deck;`), doc :19-24
- Modify: `crates/fohmixer-ui/src/store/live.rs` (`mod` lines :30-31, `Inner` :65-94, `LiveStore` :97-119 and `new` :123-136, `on_close` :385-392, `on_text` :423-501, `on_hello` :536-538)
- Modify: `.cargo/mutants.toml` (a new `exclude_re` entry for `store/live/deck.rs`)

**Interfaces:**
- Consumes: `ServerMsg::{Deck, DeckKeys, DeckAck}`, `ClientMsg::{DeckView, DeckPress}`, `DeckKey` (Task 1); the store's `FailFn`, `Socket::open`, `Conn::ready`, `LiveStore::send`.
- Produces: in `crate::store::deck`: `pub struct DeckInfo { pub online: bool, pub columns: u32, pub rows: u32, pub title: String }`, `pub fn key_side(width: f64, height: f64, columns: u32, rows: u32, gap: f64) -> f64`, `pub struct DeckWaiting<F>` with `next_seq(&mut self) -> u64`, `sent(&mut self, seq: u64, down: bool, on_fail: F)`, `ack(&mut self, seq: u64, ok: bool) -> Option<F>`, `clear(&mut self)`, `waiting(&self) -> usize`. On `LiveStore`: `pub deck: RwSignal<Option<DeckInfo>>`, `pub deck_keys: RwSignal<BTreeMap<u32, DeckKey>>`, `pub fn deck_view(self, on: bool)`, `pub fn can_send(self) -> bool`, `pub fn deck_press(self, key: u32, down: bool, hold_ms: Option<f64>, why: Option<&str>, on_fail: FailFn) -> Result<u64, FailFn>`, and the glue `on_deck`, `on_deck_keys`, `on_deck_ack`, `deck_hello`, `deck_closed`.

- [ ] **Step 1: Write the failing tests** — `crates/fohmixer-ui/src/store/deck.rs` ends with:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_as_large_as_the_area_allows_and_square() {
        // The FOH iPad's area (1194 × 834 minus the bar, the rail and the
        // padding): the width decides.
        assert_eq!(key_side(1084.0, 758.0, 8, 4, 8.0), 128.0);
        // The height decides.
        assert_eq!(key_side(1000.0, 200.0, 4, 2, 10.0), 95.0);
        // Three columns: the gaps count twice (with `/` for `*`, 31).
        assert_eq!(key_side(100.0, 1000.0, 3, 1, 10.0), 26.0);
        // Three rows likewise.
        assert_eq!(key_side(1000.0, 100.0, 1, 3, 10.0), 26.0);
        // One key fills the area.
        assert_eq!(key_side(100.0, 120.0, 1, 1, 8.0), 100.0);
        // Too small an area, or no grid yet: no key, never a negative size.
        assert_eq!(key_side(10.0, 10.0, 8, 4, 8.0), 0.0);
        assert_eq!(key_side(100.0, 100.0, 0, 0, 8.0), 100.0);
    }

    #[test]
    fn presses_have_their_own_numbers() {
        let mut waiting = DeckWaiting::<&str>::default();
        assert_eq!(
            (waiting.next_seq(), waiting.next_seq(), waiting.next_seq()),
            (1, 2, 3)
        );
    }

    #[test]
    fn a_failed_down_flashes_and_nothing_else_does() {
        let mut waiting = DeckWaiting::<&str>::default();
        waiting.sent(1, true, "flash 1");
        waiting.sent(2, false, "never kept");
        waiting.sent(3, true, "flash 3");
        assert_eq!(waiting.waiting(), 2, "only downs wait");
        assert_eq!(waiting.ack(1, true), None, "an ok down");
        assert_eq!(waiting.ack(1, false), None, "answered once");
        assert_eq!(waiting.ack(2, false), None, "an up never flashes");
        assert_eq!(waiting.ack(3, false), Some("flash 3"));
        assert_eq!(waiting.ack(9, false), None, "a seq it never sent");
        waiting.sent(4, true, "flash 4");
        waiting.clear();
        assert_eq!(waiting.waiting(), 0);
        assert_eq!(waiting.ack(4, false), None, "a closed socket forgot it");
    }
}
```

- [ ] **Step 2: Implement the pure part** — `crates/fohmixer-ui/src/store/deck.rs` (above the tests):

```rust
//! The Stream Deck on the page (#52, spec §6), pure: what the hub said of
//! it (`deck`: Companion online, the grid, the tab's title), the size of a
//! square key in the measured area, and the page's presses waiting for their
//! `deck_ack` (a failed down flashes its key; nothing is retried, nothing is
//! sent again after a reconnect). `store/live/deck.rs` carries it out.

use std::collections::BTreeMap;

/// The Stream Deck as the hub's latest `deck` message described it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeckInfo {
    /// Companion's link is up (false also while the page's own socket is
    /// down: the keys dim, the tab shows its red dot).
    pub online: bool,
    pub columns: u32,
    pub rows: u32,
    pub title: String,
}

/// The side (px, whole) of the largest square key that fits `columns` ×
/// `rows` keys with `gap` between them into `width` × `height`.
pub fn key_side(width: f64, height: f64, columns: u32, rows: u32, gap: f64) -> f64 {
    let per = |room: f64, n: u32| (room - gap * f64::from(n.saturating_sub(1))) / f64::from(n.max(1));
    per(width, columns).min(per(height, rows)).floor().max(0.0)
}

/// The page's presses waiting for their `deck_ack` (only downs: an up's
/// failure shows nothing), by the page's own press numbers (one counter per
/// kind of id: never `set`'s).
#[derive(Debug)]
pub struct DeckWaiting<F> {
    seq: u64,
    waiting: BTreeMap<u64, F>,
}

impl<F> Default for DeckWaiting<F> {
    fn default() -> Self {
        Self {
            seq: 0,
            waiting: BTreeMap::new(),
        }
    }
}

impl<F> DeckWaiting<F> {
    /// The next press's number.
    pub fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    /// Press `seq` went onto the socket: a down keeps `on_fail` until its ack.
    pub fn sent(&mut self, seq: u64, down: bool, on_fail: F) {
        if down {
            self.waiting.insert(seq, on_fail);
        }
    }

    /// The ack of `seq`: the failure handler of a down that failed.
    pub fn ack(&mut self, seq: u64, ok: bool) -> Option<F> {
        let on_fail = self.waiting.remove(&seq)?;
        (!ok).then_some(on_fail)
    }

    /// The socket closed: no ack will come.
    pub fn clear(&mut self) {
        self.waiting.clear();
    }

    pub fn waiting(&self) -> usize {
        self.waiting.len()
    }
}
```

(Checks of `key_side`: `(1084 − 7·8)/8 = 128.5`, `(758 − 3·8)/4 = 183.5` → 128; `(1000 − 30)/4 = 242.5`, `(200 − 10)/2 = 95` → 95; `(100 − 20)/3 = 26.67` → 26, with `gap / 2` it would be `(100 − 5)/3 = 31.67` → 31; `(10 − 56)/8` < 0 → 0; `columns = rows = 0`: `(100 − 0)/1` → 100.)

- [ ] **Step 3: Implement the glue** — `crates/fohmixer-ui/src/store/live/deck.rs`:

```rust
//! The Stream Deck's glue in the store (#52): the hub's `deck`, `deck_keys`
//! and `deck_ack` into the signals and the waiting presses
//! (`store/deck.rs`, tested), the page's `deck_view` (sent again after every
//! hello while the tab is open: the hub forgets a viewer whose socket
//! closed) and `deck_press` (sent at once or not at all: never queued, never
//! sent again after a reconnect, #52 §6). A socket close leaves the keys as
//! they were, dimmed (`online` false), and forgets the waiting presses (the
//! hub's `Detach` releases what this page held).

use fohmixer_proto::client::{ClientMsg, DeckKey};
use leptos::prelude::*;

use super::{FailFn, LiveStore, Socket};
use crate::dom;
use crate::store::deck::DeckInfo;

impl LiveStore {
    /// The hub's `deck`.
    pub(super) fn on_deck(self, info: DeckInfo) {
        let _ = self.deck.try_set(Some(info));
    }

    /// Keys' new states (this page views the tab).
    pub(super) fn on_deck_keys(self, items: Vec<DeckKey>) {
        let _ = self.deck_keys.try_update(|keys| {
            for item in items {
                keys.insert(item.key, item);
            }
        });
    }

    /// A press's answer: a failed down flashes its key; nothing is retried.
    pub(super) fn on_deck_ack(self, seq: u64, ok: bool, error: Option<String>) {
        if let Some(Some(flash)) = self.inner.try_update_value(|i| i.deck.ack(seq, ok)) {
            let why = error.unwrap_or_default();
            dom::log(&format!("Stream Deck press {seq} failed: {why}"));
            flash(why);
        }
    }

    /// The tab opened (`on`) or closed: only a page viewing it gets the keys.
    pub fn deck_view(self, on: bool) {
        let _ = self.inner.try_update_value(|i| i.deck_viewing = on);
        self.send(&ClientMsg::DeckView { on });
    }

    /// After a hello: the open tab tells the hub again.
    pub(super) fn deck_hello(self) {
        if self.inner.try_with_value(|i| i.deck_viewing) == Some(true) {
            self.send(&ClientMsg::DeckView { on: true });
        }
    }

    /// The socket closed: Companion's state unknown until the next hello.
    pub(super) fn deck_closed(self) {
        let _ = self.inner.try_update_value(|i| i.deck.clear());
        let _ = self.deck.try_update(|deck| {
            if let Some(deck) = deck {
                deck.online = false;
            }
        });
    }

    /// Whether a press can go now: the socket open and past its hello.
    pub fn can_send(self) -> bool {
        self.inner
            .try_with_value(|i| i.conn.ready() && i.socket.as_ref().is_some_and(Socket::open))
            .unwrap_or(false)
    }

    /// A Stream Deck press (`down`; an up's measured hold and why): onto the
    /// socket at once with the page's own number, or not at all (`Err` hands
    /// `on_fail` back to flash the key).
    pub fn deck_press(
        self,
        key: u32,
        down: bool,
        hold_ms: Option<f64>,
        why: Option<&str>,
        on_fail: FailFn,
    ) -> Result<u64, FailFn> {
        let Some(seq) = self.inner.try_update_value(|i| i.deck.next_seq()) else {
            return Err(on_fail);
        };
        let msg = ClientMsg::DeckPress {
            key,
            down,
            seq,
            t: dom::epoch_now(),
            hold_ms,
            why: why.map(str::to_string),
        };
        if !self.send(&msg) {
            return Err(on_fail);
        }
        let _ = self.inner.try_update_value(|i| i.deck.sent(seq, down, on_fail));
        Ok(seq)
    }
}
```

In `crates/fohmixer-ui/src/store/live.rs`:
- `mod link; mod writes;` gains `mod deck;`; `use crate::store::deck::{DeckInfo, DeckWaiting};` and `use fohmixer_proto::client::DeckKey;` (add to the existing `fohmixer_proto::client` import).
- `Inner` gains `/// The Stream Deck's presses waiting for their ack (#52).\n    deck: DeckWaiting<FailFn>,\n    /// The Stream Deck tab is open (#52): told again after each hello.\n    deck_viewing: bool,`; `Inner::new` gains `deck: DeckWaiting::default(),\n            deck_viewing: false,`.
- `LiveStore` gains `/// The Stream Deck (#52): none from a hub without one (no tab).\n    pub deck: RwSignal<Option<DeckInfo>>,\n    /// Its keys as Companion drew them (only while this page views the tab).\n    pub deck_keys: RwSignal<BTreeMap<u32, DeckKey>>,`; `LiveStore::new` gains `deck: RwSignal::new(None),\n            deck_keys: RwSignal::new(BTreeMap::new()),`.
- `on_close`, in the reconnecting branch after the `instances` reset: `self.deck_closed();`.
- `on_text`, before `ServerMsg::Link { .. } => {}`:

```rust
            ServerMsg::Deck {
                online,
                columns,
                rows,
                title,
            } => self.on_deck(DeckInfo {
                online,
                columns,
                rows,
                title,
            }),
            ServerMsg::DeckKeys { items } => self.on_deck_keys(items),
            ServerMsg::DeckAck { seq, ok, error, .. } => self.on_deck_ack(seq, ok, error),
```

- `on_hello`, after the `for spec in &hello.specs { self.send_sub(spec); }` loop: `self.deck_hello();`.

In `crates/fohmixer-ui/src/store.rs`: `pub mod deck;` after `mod conn;`; the doc's pure-parts sentence gains "in `deck` the Stream Deck's (#52: the key size, the presses waiting for their ack)".

In `.cargo/mutants.toml`, after the `store/live/writes.rs` entry:

```toml
  # store/live/deck.rs (#52): the Stream Deck's glue, one function at a time:
  # on_deck, on_deck_keys and on_deck_ack (the hub's messages into the
  # signals and the tested DeckWaiting), deck_view and deck_hello (the tab's
  # deck_view, again after a hello), deck_closed (a closed socket: offline,
  # DeckWaiting::clear), can_send (the socket's readiness) and deck_press
  # (DeckWaiting::next_seq and sent around the socket). deck.spec.ts drives
  # them (the grid with its keys, the presses at the fake Companion, the
  # offline dot and the red flash for Companion down and for the page's own
  # socket down, a refused press's flash, the tab again after a reconnect).
  "fohmixer-ui/src/store/live/deck\\.rs.*(replace|in) LiveStore::(on_deck|on_deck_keys|on_deck_ack|deck_view|deck_hello|deck_closed|can_send|deck_press)( |$)",
```

- [ ] **Step 4: Format and local checks** — `cargo fmt --all`; `python3 scripts/check_disposal_safety.py` (no `.set(`/`.update(` on a `set_*` receiver); `python3 scripts/check_integrity.py`.

- [ ] **Step 5: Commit**

```bash
git add crates/fohmixer-ui/src/store.rs crates/fohmixer-ui/src/store/deck.rs crates/fohmixer-ui/src/store/live.rs \
  crates/fohmixer-ui/src/store/live/deck.rs .cargo/mutants.toml
git commit -m "feat(ui): the store holds the Stream Deck (#52)

The hub's deck, deck_keys and deck_ack into signals; the tab's deck_view
again after every hello; presses sent at once or not at all, their own
numbers, a failed down's flash; a closed socket dims the keys and keeps
them.

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

- [ ] **Step 6: Checkpoint A — push and open the PR**

```bash
cargo fmt --all -- --check
ruff check .
python3 scripts/check_integrity.py
python3 scripts/check_disposal_safety.py
python3 scripts/check_version.py
python3 -m unittest discover -s scripts -p 'test_*.py'
python3 scripts/denylist_scan.py --denylist ~/.local/share/fohmixer-private/denylist.txt \
  --identities scripts/allowed-identities.txt --boundary scripts/denylist-boundary.txt \
  --accepted scripts/denylist-accepted.txt --tree HEAD --commits HEAD
git fetch origin && git merge origin/dev
git push origin dev
```

Then open the PR at once (Task 18 Step 1) so `mutants-list`, `mutation-warmup`, `mutation` and `version` run, and wait for BOTH runs of the head (one foreground bounded poll per run; `gh run view <id> --log-failed` on a red job, one fix commit, one push). Expected green: `integrity`, `python`, `lint`, `test`, `windows`, `wasm`, `e2e`, `supply-chain`, `secrets`, `version`, `mutants-list`, `mutation-warmup`, every `mutation` shard (a survivor in the diff gets a test before Checkpoint B).

---
### Task 10: The key's press state machine

**Files:**
- Create: `crates/fohmixer-ui/src/behave/deck.rs`
- Modify: `crates/fohmixer-ui/src/behave/mod.rs` :8-21 (`pub mod deck;`)

**Interfaces:**
- Consumes: nothing.
- Produces: `pub enum Why { Up, Cancel, Lost, Hidden, Tab }` with `name(self) -> &'static str`, `of_event(&str) -> Option<Why>`; `pub enum Action { Send { key: u32, down: bool, hold_ms: Option<f64>, why: Option<Why> }, Flash { key: u32 }, Nothing }`; `#[derive(Default)] pub struct Presses` with `down(&mut self, key: u32, pointer: i32, t: f64, connected: bool) -> Action`, `unsent(&mut self, key: u32)`, `up(&mut self, key: u32, pointer: i32, t: f64, why: Why) -> Action`, `leave_all(&mut self, t: f64, why: Why) -> Vec<Action>`, `clear(&mut self)`, `is_held(&self, key: u32) -> bool`, `held_keys(&self) -> BTreeSet<u32>`.

- [ ] **Step 1: Write the failing tests** — at the end of `crates/fohmixer-ui/src/behave/deck.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn down_sent(key: u32) -> Action {
        Action::Send {
            key,
            down: true,
            hold_ms: None,
            why: None,
        }
    }

    fn up_sent(key: u32, hold_ms: f64, why: Why) -> Action {
        Action::Send {
            key,
            down: false,
            hold_ms: Some(hold_ms),
            why: Some(why),
        }
    }

    #[test]
    fn a_key_is_down_at_the_touch_and_up_at_the_release_with_its_hold() {
        let mut p = Presses::default();
        assert_eq!(p.down(3, 21, 1000.0, true), down_sent(3));
        assert!(p.is_held(3));
        assert_eq!(p.up(3, 21, 1150.5, Why::Up), up_sent(3, 150.5, Why::Up));
        assert!(!p.is_held(3));
        // The lost capture after the lift ends nothing new.
        assert_eq!(p.up(3, 21, 1151.0, Why::Lost), Action::Nothing);
    }

    #[test]
    fn two_fingers_on_one_key_send_one_down_and_one_up() {
        let mut p = Presses::default();
        assert_eq!(p.down(4, 1, 0.0, true), down_sent(4));
        assert_eq!(p.down(4, 2, 50.0, true), Action::Nothing);
        assert_eq!(p.up(4, 1, 100.0, Why::Up), Action::Nothing, "a finger still holds it");
        assert!(p.is_held(4));
        assert_eq!(p.up(4, 2, 300.0, Why::Cancel), up_sent(4, 300.0, Why::Cancel));
        // A pointer that never touched the key.
        assert_eq!(p.down(5, 1, 0.0, true), down_sent(5));
        assert_eq!(p.up(5, 9, 10.0, Why::Up), Action::Nothing);
        assert!(p.is_held(5));
        // A key nobody holds.
        assert_eq!(p.up(6, 1, 10.0, Why::Up), Action::Nothing);
    }

    #[test]
    fn a_down_that_cannot_go_flashes_and_its_up_is_never_sent() {
        let mut p = Presses::default();
        assert_eq!(p.down(7, 1, 0.0, false), Action::Flash { key: 7 });
        assert!(p.is_held(7), "the outline shows the finger");
        assert_eq!(p.up(7, 1, 200.0, Why::Up), Action::Nothing);
        // A down the socket refused after all.
        assert_eq!(p.down(8, 1, 0.0, true), down_sent(8));
        p.unsent(8);
        assert_eq!(p.up(8, 1, 200.0, Why::Up), Action::Nothing);
        p.unsent(9);
        assert!(!p.is_held(9), "unsent of a key nobody holds holds nothing");
    }

    #[test]
    fn leaving_sends_an_up_for_every_sent_key_in_key_order() {
        let mut p = Presses::default();
        p.down(9, 1, 100.0, true);
        p.down(2, 2, 200.0, true);
        p.down(2, 3, 250.0, true);
        p.down(5, 4, 300.0, false);
        assert_eq!(p.held_keys(), BTreeSet::from([2, 5, 9]));
        assert_eq!(
            p.leave_all(1000.0, Why::Tab),
            vec![up_sent(2, 800.0, Why::Tab), up_sent(9, 900.0, Why::Tab)]
        );
        assert_eq!(p.held_keys(), BTreeSet::new());
        assert_eq!(p.up(2, 2, 1100.0, Why::Up), Action::Nothing, "left: nothing more");
        assert_eq!(p.leave_all(2000.0, Why::Hidden), vec![]);
    }

    #[test]
    fn a_closed_socket_forgets_every_hold_without_an_up() {
        let mut p = Presses::default();
        p.down(1, 1, 0.0, true);
        p.clear();
        assert!(!p.is_held(1));
        assert_eq!(p.up(1, 1, 50.0, Why::Up), Action::Nothing, "the hub releases it");
    }

    #[test]
    fn the_why_of_an_up() {
        assert_eq!(Why::of_event("pointerup"), Some(Why::Up));
        assert_eq!(Why::of_event("pointercancel"), Some(Why::Cancel));
        assert_eq!(Why::of_event("lostpointercapture"), Some(Why::Lost));
        assert_eq!(Why::of_event("pointermove"), None);
        assert_eq!(
            [Why::Up, Why::Cancel, Why::Lost, Why::Hidden, Why::Tab].map(Why::name),
            ["up", "cancel", "lost", "hidden", "tab"]
        );
    }
}
```

- [ ] **Step 2: Implement** — `crates/fohmixer-ui/src/behave/deck.rs` (above the tests):

```rust
//! A Stream Deck key's presses on the page (#52, spec §6), pure: each held
//! key's pointers, when its first finger came and whether its down went to
//! the hub. A key is down at its first finger and up at its last lift, so
//! Companion's long-press and duration actions see the real hold; a down
//! that cannot go now flashes the key and is never sent later, and its up is
//! not sent either. Leaving the tab or the page going hidden lifts every
//! held key; a closed socket forgets them (the hub releases them). The
//! component (`pages/deck.rs`) carries the actions out.

use std::collections::{BTreeMap, BTreeSet};

/// Why an up came.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// `pointerup`.
    Up,
    /// `pointercancel` (the system took the touch).
    Cancel,
    /// `lostpointercapture` without a `pointerup` before it.
    Lost,
    /// The page went hidden (`visibilitychange`).
    Hidden,
    /// The tab was left (the page's cleanup).
    Tab,
}

impl Why {
    /// Its name on the wire and in the flight recorder.
    pub fn name(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Cancel => "cancel",
            Self::Lost => "lost",
            Self::Hidden => "hidden",
            Self::Tab => "tab",
        }
    }

    /// The why of a pointer event that ends a touch.
    pub fn of_event(event_type: &str) -> Option<Self> {
        match event_type {
            "pointerup" => Some(Self::Up),
            "pointercancel" => Some(Self::Cancel),
            "lostpointercapture" => Some(Self::Lost),
            _ => None,
        }
    }
}

/// What the page does about a pointer event.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    /// Send the press: a down, or an up with the page's hold and its why.
    Send {
        key: u32,
        down: bool,
        hold_ms: Option<f64>,
        why: Option<Why>,
    },
    /// Flash the key red: its down cannot go now (never later either).
    Flash { key: u32 },
    Nothing,
}

/// One held key.
#[derive(Debug, Clone, PartialEq)]
struct Hold {
    pointers: BTreeSet<i32>,
    /// The first finger's page time.
    at: f64,
    /// Its down went to the hub.
    sent: bool,
}

/// The page's held keys.
#[derive(Debug, Default)]
pub struct Presses {
    held: BTreeMap<u32, Hold>,
}

impl Presses {
    /// A finger (`pointer`) on `key` at page time `t`; `connected`: a press
    /// can go now (the socket ready, Companion online).
    pub fn down(&mut self, key: u32, pointer: i32, t: f64, connected: bool) -> Action {
        if let Some(hold) = self.held.get_mut(&key) {
            hold.pointers.insert(pointer);
            return Action::Nothing;
        }
        self.held.insert(
            key,
            Hold {
                pointers: BTreeSet::from([pointer]),
                at: t,
                sent: connected,
            },
        );
        if connected {
            Action::Send {
                key,
                down: true,
                hold_ms: None,
                why: None,
            }
        } else {
            Action::Flash { key }
        }
    }

    /// The down of `key` did not go after all (the socket refused it): its
    /// up is not sent either.
    pub fn unsent(&mut self, key: u32) {
        if let Some(hold) = self.held.get_mut(&key) {
            hold.sent = false;
        }
    }

    /// A finger (`pointer`) leaves `key` at page time `t`: the up of a key
    /// whose down went, when it was the last finger.
    pub fn up(&mut self, key: u32, pointer: i32, t: f64, why: Why) -> Action {
        let last = match self.held.get_mut(&key) {
            Some(hold) => hold.pointers.remove(&pointer) && hold.pointers.is_empty(),
            None => false,
        };
        if !last {
            return Action::Nothing;
        }
        match self.held.remove(&key) {
            Some(hold) if hold.sent => Action::Send {
                key,
                down: false,
                hold_ms: Some(t - hold.at),
                why: Some(why),
            },
            _ => Action::Nothing,
        }
    }

    /// The page leaves every key (`why`: hidden, tab): an up for each key
    /// whose down went, in key order.
    pub fn leave_all(&mut self, t: f64, why: Why) -> Vec<Action> {
        std::mem::take(&mut self.held)
            .into_iter()
            .filter(|(_, hold)| hold.sent)
            .map(|(key, hold)| Action::Send {
                key,
                down: false,
                hold_ms: Some(t - hold.at),
                why: Some(why),
            })
            .collect()
    }

    /// The socket closed: every hold forgotten, no up sent.
    pub fn clear(&mut self) {
        self.held.clear();
    }

    /// Whether a finger holds `key` (the local outline).
    pub fn is_held(&self, key: u32) -> bool {
        self.held.contains_key(&key)
    }

    /// The keys fingers hold.
    pub fn held_keys(&self) -> BTreeSet<u32> {
        self.held.keys().copied().collect()
    }
}
```

`crates/fohmixer-ui/src/behave/mod.rs`: `pub mod deck;` after `pub mod db_text;`; its doc gains "and the Stream Deck's presses (#52, `deck`)".

- [ ] **Step 3: Format** — `cargo fmt --all`.

- [ ] **Step 4: Verify** — at Checkpoint B: CI `test` (`cargo test -p fohmixer-ui --lib`) runs `behave::deck::tests::*`; `mutation` judges every function (the hold's `t - at` pinned by 150.5, 300, 800, 900).

- [ ] **Step 5: Commit**

```bash
git add crates/fohmixer-ui/src/behave/deck.rs crates/fohmixer-ui/src/behave/mod.rs
git commit -m "feat(ui): the Stream Deck key's press state machine (#52)

Down at the first finger, up at the last lift with the page's hold and its
why; a down that cannot go flashes and its up never goes; leaving the tab
or a hidden page lifts every key; a closed socket forgets them.

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---
### Task 11: The flight recorder's deck events

**Files:**
- Modify: `crates/fohmixer-ui/src/diag/trace.rs` (doc :6-23, `drop_rank` :131-138, new builders after `intent` :156-158)
- Test: `crates/fohmixer-ui/src/diag/trace/tests.rs` (new test), `crates/fohmixer-ui/src/diag/trace/backlog_tests.rs` :268-283

**Interfaces:**
- Produces: `pub fn deck(t: f64, key: u32, down: bool, hold_ms: Option<f64>, why: Option<&str>, sent: bool, seq: Option<u64>) -> Value`; `pub fn deck_view(t: f64, on: bool) -> Value`; `drop_rank("deck") == None`.

- [ ] **Step 1: Write the failing tests** — in `diag/trace/tests.rs`:

```rust
#[test]
fn a_stream_deck_press_and_the_tab_are_page_events() {
    assert_eq!(
        deck(1_000.5, 3, true, None, None, true, Some(7)),
        json!({"ev": "deck", "t": 1_000.5, "k": 3, "d": 1, "sent": true, "q": 7})
    );
    assert_eq!(
        deck(1_150.5, 3, false, Some(149.6), Some("cancel"), true, Some(8)),
        json!({"ev": "deck", "t": 1_150.5, "k": 3, "d": 0, "h": 150.0, "why": "cancel",
               "sent": true, "q": 8})
    );
    // A down that could not go: no seq.
    assert_eq!(
        deck(2_000.0, 6, true, None, None, false, None),
        json!({"ev": "deck", "t": 2_000.0, "k": 6, "d": 1, "sent": false})
    );
    assert_eq!(deck_view(5.0, true), json!({"ev": "deck_view", "t": 5.0, "on": true}));
    assert_eq!(deck_view(6.0, false), json!({"ev": "deck_view", "t": 6.0, "on": false}));
}
```

In `backlog_tests.rs`, add `"deck",` to the essential loop's list (after `"intent",`) and change `for ev in ["rtt", "frame", "x", ""]` to `for ev in ["rtt", "frame", "deck_view", "x", ""]`.

- [ ] **Step 2: Implement** — in `diag/trace.rs`, after `intent`:

```rust
/// A Stream Deck key's press on the page (#52): `k` the key, `d` 1 down / 0
/// up, `h` the hold the page measured (an up; whole ms), `why` the up's cause
/// (`up`, `cancel`, `lost`, `hidden`, `tab`), `sent` whether the socket took
/// it, `q` its seq (when it went). Essential: what the forensics read of
/// every press and every red flash.
pub fn deck(
    t: f64,
    key: u32,
    down: bool,
    hold_ms: Option<f64>,
    why: Option<&str>,
    sent: bool,
    seq: Option<u64>,
) -> Value {
    let mut event = json!({"ev": "deck", "t": t, "k": key, "d": u8::from(down), "sent": sent});
    if let Some(hold) = hold_ms {
        event["h"] = json!(hold.round());
    }
    if let Some(why) = why {
        event["why"] = json!(why);
    }
    if let Some(seq) = seq {
        event["q"] = json!(seq);
    }
    event
}

/// The Stream Deck tab opened (`on`) or closed (#52).
pub fn deck_view(t: f64, on: bool) -> Value {
    json!({"ev": "deck_view", "t": t, "on": on})
}
```

`drop_rank`'s essential arm becomes `"touch" | "dropout" | "reset" | "sock" | "visibility" | "overflow" | "intent" | "sys" | "zoom" | "deck" => None,`; its doc and the module doc's event list gain "a Stream Deck press (`deck`, #52, essential) and the tab's opening and closing (`deck_view`)".

- [ ] **Step 3: Format** — `cargo fmt --all`.

- [ ] **Step 4: Verify** — at Checkpoint B: CI `test` runs both tests; `mutation` judges `deck` (the `round`, each optional field), `deck_view`, the new `drop_rank` arm.

- [ ] **Step 5: Commit**

```bash
git add crates/fohmixer-ui/src/diag/trace.rs crates/fohmixer-ui/src/diag/trace/tests.rs \
  crates/fohmixer-ui/src/diag/trace/backlog_tests.rs
git commit -m "feat(ui): the flight recorder's Stream Deck events (#52)

deck (essential): key, down or up, the page's hold, why, sent, seq;
deck_view: the tab opened or closed.

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---
### Task 12: The Stream Deck tab and its page

**Files:**
- Create: `crates/fohmixer-ui/src/pages/deck.rs`
- Create: `crates/fohmixer-ui/deck.css`
- Modify: `crates/fohmixer-ui/src/pages/mod.rs` (`pub mod deck;`), `crates/fohmixer-ui/src/pages/surface.rs` (doc :1-8, `Nav` :32-62, `Surface` :76-83, `Shell` :141-195, `TabBar` :197-233, a new `DeckTab`), `crates/fohmixer-ui/index.html` :7-12, `crates/fohmixer-ui/Trunk.toml` :42, `crates/fohmixer-ui/style.css` :9-14 (the stylesheet list), `.cargo/mutants.toml` (the `pages/surface.rs` entry, a new `pages/deck.rs` entry), `.claude/rules/ui-rust.md`

**Interfaces:**
- Consumes: `Presses`, `Action`, `Why` (Task 10); `trace::deck`, `trace::deck_view`, `diag::record` (Task 11); `LiveStore::{deck, deck_keys, deck_view, deck_press, can_send, connected}`, `key_side` (Task 9); `components::{ControlView, fail_flash, owns_touches}`, `dom::{epoch_now, hidden, current_element}`.
- Produces: `pages::deck::DeckView` (props `global: Vec<Control>`, `viewport: RwSignal<(f64, f64)>`), `KEY_GAP = 8.0`, `AREA_PAD = 10.0`; `Nav::show_deck`, `Nav.deck: RwSignal<bool>`; DOM: `[data-testid="deck-tab"]` (`data-selected`, `data-offline`), `[data-testid="deck-dot"]`, `[data-testid="deck"]`, `[data-testid="deck-grid"]`, `[data-testid="deck-key"]` (`data-key`, `data-pressed`, `data-held`, `data-failed`; classes `pressed`, `held`, `failed`, `offline`) with an `<img class="deck-img">`.

- [ ] **Step 1: The page** — `crates/fohmixer-ui/src/pages/deck.rs`:

```rust
//! The Stream Deck tab (#52, spec §2, §6): Companion's keys as one grid of
//! square keys as large as the area allows, the layout's global controls on
//! the rail (as on the Conf page). A key is down at the touch and up at the
//! release (`behave::deck::Presses` decides; this file carries it out): a
//! down that cannot go now flashes red and is never sent later; a finger on
//! a key shows a local outline at once (Companion's pressed look needs a
//! round trip). The page tells the hub it views the tab (only a viewer gets
//! the keys' images) and lifts every held key when the tab is left (`tab`)
//! or the page goes hidden (`hidden`); a closed socket forgets the holds
//! (the hub releases them). Every key owns its touches (`use:owns_touches`):
//! no loupe, callout or drag on a hold.

use std::collections::BTreeSet;

use fohmixer_proto::layout::Control;
use leptos::html;
use leptos::prelude::*;

use crate::behave::deck::{Action, Presses, Why};
use crate::components::{ControlView, fail_flash, owns_touches};
use crate::diag::{self, trace};
use crate::dom;
use crate::store::LiveStore;
use crate::store::deck::key_side;

/// The gap between two keys (px): `.deck-grid`'s `gap` in deck.css.
pub const KEY_GAP: f64 = 8.0;
/// The grid area's padding on each side (px): `.deck-area` in deck.css.
pub const AREA_PAD: f64 = 10.0;

/// Carries a press decision out: onto the socket and into the flight
/// recorder; a down that could not go flashes `failed` (when given) and its
/// up is not sent either.
fn carry(store: LiveStore, presses: StoredValue<Presses>, action: Action, failed: Option<RwSignal<bool>>) {
    let t = dom::epoch_now();
    match action {
        Action::Nothing => {}
        Action::Flash { key } => {
            if let Some(failed) = failed {
                fail_flash(failed)("not connected".to_string());
            }
            diag::record(&trace::deck(t, key, true, None, None, false, None));
        }
        Action::Send {
            key,
            down,
            hold_ms,
            why,
        } => {
            let why = why.map(Why::name);
            let on_fail: Box<dyn FnOnce(String)> = match failed {
                Some(failed) => fail_flash(failed),
                None => Box::new(|_: String| {}),
            };
            match store.deck_press(key, down, hold_ms, why, on_fail) {
                Ok(seq) => diag::record(&trace::deck(t, key, down, hold_ms, why, true, Some(seq))),
                Err(flash) => {
                    if down {
                        flash("not connected".to_string());
                        let _ = presses.try_update_value(|p| p.unsent(key));
                    }
                    diag::record(&trace::deck(t, key, down, hold_ms, why, false, None));
                }
            }
        }
    }
}

/// The held keys' outline follows the state machine.
fn show_held(presses: StoredValue<Presses>, held: RwSignal<BTreeSet<u32>>) {
    if let Some(keys) = presses.try_with_value(Presses::held_keys) {
        let _ = held.try_set(keys);
    }
}

/// Every held key up (`why`: the tab left, the page hidden).
fn leave_keys(store: LiveStore, presses: StoredValue<Presses>, held: RwSignal<BTreeSet<u32>>, why: Why) {
    let actions = presses
        .try_update_value(|p| p.leave_all(dom::epoch_now(), why))
        .unwrap_or_default();
    for action in actions {
        carry(store, presses, action, None);
    }
    show_held(presses, held);
}

/// The Stream Deck tab's page: the rail with the layout's global controls,
/// then the key grid.
#[component]
pub fn DeckView(global: Vec<Control>, viewport: RwSignal<(f64, f64)>) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let presses = StoredValue::new(Presses::default());
    let held = RwSignal::new(BTreeSet::<u32>::new());
    let area_ref = NodeRef::<html::Div>::new();
    let area = RwSignal::new((0.0_f64, 0.0_f64));
    store.deck_view(true);
    diag::record(&trace::deck_view(dom::epoch_now(), true));
    let hidden = window_event_listener_untyped("visibilitychange", move |_| {
        if dom::hidden() {
            leave_keys(store, presses, held, Why::Hidden);
        }
    });
    on_cleanup(move || {
        hidden.remove();
        leave_keys(store, presses, held, Why::Tab);
        store.deck_view(false);
        diag::record(&trace::deck_view(dom::epoch_now(), false));
    });
    // A closed socket forgets every hold: the hub releases them (Detach).
    Effect::new(move |_| {
        if !store.connected.get() {
            let _ = presses.try_update_value(Presses::clear);
            show_held(presses, held);
        }
    });
    // The grid's room: measured once laid out and on every resize.
    Effect::new(move |_| {
        let _ = viewport.get();
        if let Some(el) = area_ref.get() {
            let _ = area.try_set((f64::from(el.client_width()), f64::from(el.client_height())));
        }
    });
    let shape = Memo::new(move |_| store.deck.with(|d| d.as_ref().map(|d| (d.columns, d.rows))));
    let grid_style = move || {
        let (columns, rows) = shape.get().unwrap_or((1, 1));
        let (width, height) = area.get();
        let side = key_side(width - 2.0 * AREA_PAD, height - 2.0 * AREA_PAD, columns, rows, KEY_GAP);
        format!("--cols:{columns};--rows:{rows};--key:{side:.0}px;")
    };
    let keys = move || {
        let (columns, rows) = shape.get().unwrap_or((0, 0));
        (0..columns * rows)
            .map(|index| view! { <DeckKeyView index=index presses=presses held=held /> })
            .collect_view()
    };
    let global = global
        .into_iter()
        .map(|control| view! { <ControlView control=control /> })
        .collect_view();
    view! {
        <div class="body deck-page" data-testid="deck">
            <nav class="rail" data-testid="rail">
                <div class="rail-main"></div>
                <div class="rail-foot">{global}</div>
            </nav>
            <div class="deck-area" node_ref=area_ref>
                <div class="deck-grid" data-testid="deck-grid" style=grid_style>
                    {keys}
                </div>
            </div>
        </div>
    }
}

/// One key: Companion's image (or colour), Companion's pressed look, the
/// local outline while a finger holds it, the red flash, dimmed offline.
/// (`index` is the key's number: a prop named `key` could read as Leptos'
/// own keyed-list attribute.)
#[component]
fn DeckKeyView(index: u32, presses: StoredValue<Presses>, held: RwSignal<BTreeSet<u32>>) -> impl IntoView {
    let key = index;
    let store = expect_context::<LiveStore>();
    let failed = RwSignal::new(false);
    let look = Memo::new(move |_| store.deck_keys.with(|keys| keys.get(&key).cloned()));
    let online = Memo::new(move |_| store.deck.with(|d| d.as_ref().is_some_and(|d| d.online)));
    let on_down = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        if let Some(el) = dom::current_element(&ev) {
            let _ = el.set_pointer_capture(ev.pointer_id());
        }
        let connected = store.can_send() && online.get_untracked();
        let action = presses.try_update_value(|p| p.down(key, ev.pointer_id(), dom::epoch_now(), connected));
        if let Some(action) = action {
            carry(store, presses, action, Some(failed));
        }
        show_held(presses, held);
    };
    let on_end = move |ev: web_sys::PointerEvent| {
        let Some(why) = Why::of_event(&ev.type_()) else {
            return;
        };
        let action = presses.try_update_value(|p| p.up(key, ev.pointer_id(), dom::epoch_now(), why));
        if let Some(action) = action {
            carry(store, presses, action, Some(failed));
        }
        show_held(presses, held);
    };
    let pressed = move || look.get().is_some_and(|k| k.pressed);
    let is_held = move || held.with(|h| h.contains(&key));
    let offline = move || !online.get();
    let is_failed = move || failed.get();
    let colour = move || {
        look.get()
            .and_then(|k| k.color)
            .map(|c| format!("background-color:{c};"))
            .unwrap_or_default()
    };
    let image = move || {
        look.get()
            .and_then(|k| k.img)
            .map(|src| view! { <img class="deck-img" src=src alt="" draggable="false" /> })
    };
    // A key owns its touches (#43 PR G); it writes no Live key.
    let no_keys: Vec<String> = Vec::new();
    let key_text = key.to_string();
    view! {
        <button
            type="button"
            class="deck-key"
            use:owns_touches=no_keys
            class:pressed=pressed
            class:held=is_held
            class:failed=is_failed
            class:offline=offline
            data-testid="deck-key"
            data-key=key_text
            data-pressed=move || pressed().to_string()
            data-held=move || is_held().to_string()
            data-failed=move || is_failed().to_string()
            style=colour
            on:pointerdown=on_down
            on:pointerup=on_end
            on:pointercancel=on_end
            on:lostpointercapture=on_end
        >
            {image}
        </button>
    }
}
```

`crates/fohmixer-ui/src/pages/mod.rs`: doc "The app's pages: the PIN login, the mixer surface and its Stream Deck tab (#52)."; `pub mod deck;` before `pub mod login;`.

- [ ] **Step 2: The tab** — in `crates/fohmixer-ui/src/pages/surface.rs`:

Doc: the top bar list gains "the Stream Deck tab (#52, last, only from a hub with one)".

`use crate::pages::deck::DeckView;`

`Nav` gains `/// The Stream Deck tab is shown (#52); never remembered: a reload opens the last mixer page.\n    deck: RwSignal<bool>,` and:

```rust
    /// The Stream Deck tab was tapped (#52).
    fn show_deck(self) {
        let _ = self.deck.try_set(true);
    }
```

and `select` starts with `let _ = self.deck.try_set(false);` (a layout tab leaves the deck; the page shown is selected as before).

In `Surface`, the `Nav` literal gains `deck: RwSignal::new(false),`.

In `Shell`:
- `pager_tabs`: after `let index = page.get()?;` add `if nav.deck.get() { return None; }` (written as a statement before the next line).
- `body`:

```rust
    let body = move || {
        if nav.deck.get() {
            let global = for_page.global.clone();
            return Some(view! { <DeckView global=global viewport=nav.viewport /> }.into_any());
        }
        let index = page.get()?;
        let shown = for_page.pages.get(index)?.clone();
        let global = for_page.global.clone();
        Some(view! { <PageView page=shown global=global sub=sub /> }.into_any())
    };
```

`TabBar`: the layout tabs' `lit` becomes:

```rust
            let lit = move || selected.get() == Some(index) && !(level == 0 && nav.deck.get());
```

and before `view! {` of the bar:

```rust
    // The Stream Deck's tab ends the pages' bar, from a hub with one (#52).
    let deck_tab = (level == 0).then(|| {
        let has_deck = move || nav.store.deck.with(Option::is_some);
        view! { <Show when=has_deck><DeckTab /></Show> }
    });
```

with the bar's markup `<div class="seg" …>{buttons}{deck_tab}</div>`.

The new component (after `TabBar`):

```rust
/// The Stream Deck's tab (#52): its title from the hub; a small red dot
/// while Companion (or the hub) is unreachable, no words.
#[component]
fn DeckTab() -> impl IntoView {
    let nav = expect_context::<Nav>();
    let store = nav.store;
    let title = move || {
        store
            .deck
            .with(|d| d.as_ref().map(|d| d.title.clone()).unwrap_or_default())
    };
    let offline = move || store.deck.with(|d| d.as_ref().is_some_and(|d| !d.online));
    let lit = move || nav.deck.get();
    // A tab owns its touches (#43 PR G); it writes no key.
    let no_keys: Vec<String> = Vec::new();
    view! {
        <button
            type="button"
            class="tab deck-tab"
            use:owns_touches=no_keys
            class:selected=lit
            data-testid="deck-tab"
            data-selected=move || lit().to_string()
            data-offline=move || offline().to_string()
            on:pointerdown=move |_| nav.show_deck()
        >
            {title}
            <Show when=offline>
                <i class="deck-dot" data-testid="deck-dot"></i>
            </Show>
        </button>
    }
}
```

- [ ] **Step 3: The stylesheet** — `crates/fohmixer-ui/deck.css`:

```css
/* The Stream Deck tab (#52): Companion's keys in one grid of square keys.
   The key size --key comes from pages/deck.rs (key_side over the measured
   .deck-area); keep KEY_GAP equal to .deck-grid's gap and AREA_PAD to
   .deck-area's padding. Companion draws each key's text, colours and pressed
   look into its image; the page adds the held outline (a finger on it, at
   once), the red flash and the offline dimming. */

.deck-tab {
    position: relative;
}

.deck-dot {
    position: absolute;
    top: 3px;
    right: 3px;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--danger);
}

.deck-area {
    display: flex;
    align-items: center;
    justify-content: center;
    min-width: 0;
    min-height: 0;
    padding: 10px;
    overflow: hidden;
}

.deck-grid {
    display: grid;
    grid-template-columns: repeat(var(--cols), var(--key));
    grid-template-rows: repeat(var(--rows), var(--key));
    gap: 8px;
}

.deck-key {
    position: relative;
    width: var(--key);
    height: var(--key);
    padding: 0;
    border: 0;
    border-radius: 6px;
    background: #000;
    outline: 3px solid transparent;
    outline-offset: 2px;
    overflow: hidden;
    touch-action: none;
}

.deck-img {
    display: block;
    width: 100%;
    height: 100%;
    pointer-events: none;
    -webkit-user-drag: none;
}

.deck-key.held {
    outline-color: var(--accent);
}

.deck-key.failed {
    outline-color: var(--danger);
}

.deck-key.offline {
    opacity: 0.35;
}
```

`index.html`: after `<link data-trunk rel="css" href="intent.css" />` add `<link data-trunk rel="css" href="deck.css" />`. `Trunk.toml` `[watch]`: add `"deck.css"` after `"intent.css"`. `style.css`'s header comment: "… and intent.css (the look of a write Live has not confirmed), then deck.css (the Stream Deck tab, #52)."

- [ ] **Step 4: Mutation excludes and the playbook** — in `.cargo/mutants.toml`, the `pages/surface.rs` entry becomes:

```toml
  # pages/surface.rs: a tab press (store, local storage), the Stream Deck
  # tab's press (#52: Nav::show_deck sets the tab's signal; deck.spec.ts
  # opens it and checks a reload never restores it) and the remembered
  # pages' read; choose and selected_path are the tested decisions.
  "fohmixer-ui/src/pages/surface\\.rs.*(replace|in) (Nav::select|Nav::show_deck|remembered_pages)( |$)",
```

and after it:

```toml
  # pages/deck.rs (#52): the Stream Deck page's glue, one function at a
  # time: carry (a press decision of the tested behave::deck::Presses onto
  # the socket through LiveStore::deck_press, into the recorder through the
  # tested trace::deck, a refused down flashed with fail_flash), show_held
  # (the outline's signal from Presses::held_keys) and leave_keys
  # (Presses::leave_all's ups when the tab is left or the page hidden).
  # deck.spec.ts drives them in Chromium and WebKit (down and up on
  # pointerup, pointercancel and leaving the tab, with the page's deck
  # events and their whys in the event log; the outline; the red flash with
  # no later send).
  "fohmixer-ui/src/pages/deck\\.rs.*(replace|in) (carry|show_held|leave_keys)( |$)",
```

In `.claude/rules/ui-rust.md` "Pure decisions, thin browser glue", add to the pure list "`behave/deck.rs` (#52: a Stream Deck key's `Presses`: down at the first finger, up at the last lift with the page's hold and why — `up`/`cancel`/`lost`/`hidden`/`tab` —, a down that cannot go flashes and its up never goes, `leave_all` for the tab and a hidden page, `clear` on a closed socket), `store/deck.rs` (#52: `key_side`, the square key in the measured area; `DeckWaiting`, the downs waiting for their `deck_ack`, the page's own press numbers)", and a new bullet:

```markdown
- **The Stream Deck tab (#52, `pages/deck.rs`, `store/live/deck.rs`):** the tab is `Nav.deck`, never stored (a reload opens the last mixer page); `DeckView` sends `deck_view` on mount and cleanup (the store sends it again after each hello while open), lifts every held key on `visibilitychange` hidden (`window_event_listener_untyped`: the event bubbles from the document to the window) and on cleanup, and forgets the holds when `connected` goes false (the hub's `Detach` releases them). A press is sent at once or not at all (`LiveStore::deck_press` returns the failure handler when the socket refuses it): never through the intent store, never resent (#43 L1 is deliberately not followed: a light must not switch late). Keys are `<button class="deck-key" use:owns_touches=…>` with Pointer Events only; a dispatched `pointerdown`'s `set_pointer_capture` result is ignored (no console line). Only a viewer gets `deck_keys`; each key reads its `Memo` of the map (a change redraws one key). The key size is `store::deck::key_side` over `.deck-area`'s measured size (deck.css's gap and padding equal `KEY_GAP`, `AREA_PAD`).
```

- [ ] **Step 5: Format and local checks** — `cargo fmt --all`; `python3 scripts/check_integrity.py` (every `on:pointerdown` in `pages/deck.rs` and `surface.rs` has `use:owns_touches` in its start tag; no `on:click`); `python3 scripts/check_disposal_safety.py`.

- [ ] **Step 6: Commit**

```bash
git add crates/fohmixer-ui/src/pages/deck.rs crates/fohmixer-ui/src/pages/mod.rs crates/fohmixer-ui/src/pages/surface.rs \
  crates/fohmixer-ui/deck.css crates/fohmixer-ui/index.html crates/fohmixer-ui/Trunk.toml crates/fohmixer-ui/style.css \
  .cargo/mutants.toml .claude/rules/ui-rust.md
git commit -m "feat(ui): the Stream Deck tab (#52)

The last tab of the top bar, from a hub with [companion], a red dot while
Companion is unreachable; its page shows the global controls on the rail
and Companion's keys as square keys as large as the area allows. A key is
down at the touch and up at the release (pointerup, pointercancel, a lost
capture, the tab left, the page hidden), shows a local outline at once,
flashes red when its press cannot go and never sends it later. A reload
opens the last mixer page.

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

- [ ] **Step 7: Checkpoint B — push** — the local checks of Checkpoint A, the denylist scan, `git fetch origin && git merge origin/dev`, `git push origin dev`; wait for both runs of the head. Expected green: `lint` (clippy native and wasm32), `test` (`fohmixer-ui --lib` with `behave::deck`, `store::deck`, `diag::trace` tests), `wasm` (the Trunk build with deck.css), `e2e` (the existing suite: no `[companion]` yet, so no tab; `pages.spec.ts` still reads three layout tabs), `integrity`, `mutation` (the pure UI functions; the glue excluded above).

---
### Task 13: The forensics timeline's Stream Deck section

**Files:**
- Modify: `tools/forensics/timeline_touch.py` (doc :1-62, a new section at the end)
- Modify: `tools/forensics/timeline_model.py` (`Timeline.__init__` :317-366, `summary` :810-875)
- Modify: `tools/forensics/timeline_report.py` (CSS, a new `_deck_table` before `render` :621, `render`)
- Modify: `tools/forensics/timeline.py` (docstring :2-77)
- Create: `tools/forensics/test_timeline_deck.py`
- Modify: `tools/forensics/test_timeline.py` `test_the_summary_table_holds_stdouts_values` :385-445
- Modify: `.claude/rules/forensics.md`

**Interfaces:**
- Consumes: the hub's `deck_press`, `deck_ok`, `deck_release`, `deck_link` records (Tasks 5, 8) and the page's `deck` events (Task 11); `timeline_read.number`; `timeline_model.percentile`, `ms_text`.
- Produces: in `timeline_touch`: `DeckPress`, `DeckRelease`, `DeckOutage`, `DeckUnsent` (namedtuples), `deck_presses(presses, answers)`, `deck_release(record)`, `deck_outages(links, start, end)`, `deck_unsent(events, presses)`; `Timeline.deck_presses`, `.deck_releases`, `.deck_outages`, `.deck_unsent`; summary names `deck_presses`, `deck_unsent`, `deck_forced_releases`, `deck_link_outages`, `deck_rtt_p50_ms`, `deck_rtt_p99_ms` (after `zooms`); report rows `tr.deck-press`, `tr.deck-release`, `tr.deck-unsent`, `tr.deck-outage`.

- [ ] **Step 1: Write the failing tests** — `tools/forensics/test_timeline_deck.py`:

```python
"""The forensics timeline's Stream Deck section (#52): the hub's
``deck_press`` records with Companion's round trip from their ``deck_ok``
(matched by client and seq), its ``deck_release`` and ``deck_link`` records,
and the page's ``deck`` events (``diag/trace.rs``), listed in the report and
counted on stdout, numbers only. The synthetic logs and the report reader
come from ``test_timeline.py``."""

import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)

from test_timeline import BASE, OFFSET, Log, ReportCase  # noqa: E402
from timeline_touch import DeckOutage, deck_outages, deck_presses  # noqa: E402

PEER = "192.0.2.10"


def press(log, ts, client, key, down, seq, *, forwarded=True, reason=None, why=None, hold=None):
    """A hub ``deck_press`` record as ``deck.rs`` ``press_fields`` writes it."""
    log.add(
        "deck_press",
        ts,
        client=client,
        peer=PEER,
        key=key,
        down=down,
        seq=seq,
        t=ts - OFFSET - 4.0,
        hub_ms=float(ts),
        offset_ms=OFFSET,
        delay_ms=4.0,
        gap_ms=None,
        hold_ms=hold,
        why=why,
        hub_hold_ms=None if hold is None else hold - 4.0,
        forwarded=forwarded,
        reason=reason,
        holders=1 if down else 0,
    )


def ok(log, ts, client, key, down, seq, rtt_ms):
    """Companion's answer, a hub ``deck_ok`` record."""
    log.add("deck_ok", ts, client=client, seq=seq, key=key, down=down, ok=True, error=None, rtt_ms=rtt_ms)


def link(log, ts, state, **fields):
    log.add("deck_link", ts, state=state, **fields)


class DeckSection(ReportCase):
    def deck(self, log):
        """Socket 7's page: key 3 held 1.5 s; Companion away 2 s, a press
        refused then; key 5 released by the hub (silent); a down the page
        could not send (key 6)."""
        log.pings(7, BASE, BASE + 9000)
        link(log, BASE + 100, "up", companion="5.0.7", api="1.12.0", error=None, down_ms=None, attempts=1)
        press(log, BASE + 1000, 7, 3, True, 1)
        ok(log, BASE + 1005, 7, 3, True, 1, 5.0)
        press(log, BASE + 2500, 7, 3, False, 2, why="up", hold=1500.0)
        ok(log, BASE + 2512, 7, 3, False, 2, 12.0)
        link(log, BASE + 3000, "down", companion=None, api=None, error="Companion closed the connection")
        press(log, BASE + 3500, 7, 4, True, 3, forwarded=False, reason="offline")
        link(log, BASE + 5000, "up", companion="5.0.7", api="1.12.0", error=None, down_ms=2000.0, attempts=1)
        press(log, BASE + 6000, 7, 5, True, 4)
        ok(log, BASE + 6004, 7, 5, True, 4, 4.0)
        log.add("deck_release", BASE + 8000, client=7, key=5, reason="silent", hub_hold_ms=2000.0)
        page = -OFFSET
        log.trace(
            BASE + 8500,
            7,
            [
                {"ev": "deck", "t": BASE + 996 + page, "k": 3, "d": 1, "sent": True, "q": 1},
                {"ev": "deck", "t": BASE + 2496 + page, "k": 3, "d": 0, "h": 1500, "why": "up",
                 "sent": True, "q": 2},
                {"ev": "deck", "t": BASE + 7000 + page, "k": 6, "d": 1, "sent": False},
            ],
        )

    def test_presses_releases_flashes_and_outages_are_listed_and_counted(self):
        log = Log()
        self.deck(log)
        summary, page, stdout = self.report(log, BASE, BASE + 9000)
        self.assertEqual(
            {
                k: summary[k]
                for k in (
                    "deck_presses",
                    "deck_unsent",
                    "deck_forced_releases",
                    "deck_link_outages",
                    "deck_rtt_p50_ms",
                    "deck_rtt_p99_ms",
                )
            },
            {
                "deck_presses": "3",
                "deck_unsent": "2",
                "deck_forced_releases": "1",
                "deck_link_outages": "1",
                "deck_rtt_p50_ms": "5.0",
                "deck_rtt_p99_ms": "12.0",
            },
        )
        presses = page.of_class("deck-press")
        self.assertEqual(
            [(a["data-key"], a["data-down"], a["data-forwarded"]) for a in presses],
            [("3", "true", "true"), ("3", "false", "true"), ("4", "true", "false"), ("5", "true", "true")],
        )
        self.assertEqual(float(presses[1]["data-rtt"]), 12.0)
        self.assertEqual(presses[1]["data-why"], "up")
        self.assertEqual(presses[2]["data-reason"], "offline")
        self.assertNotIn("data-rtt", presses[2], "a refused press has no round trip")
        self.assertEqual(
            [(a["data-key"], a["data-where"]) for a in page.of_class("deck-unsent")],
            [("4", "hub"), ("6", "page")],
        )
        self.assertEqual(
            [(a["data-key"], a["data-reason"]) for a in page.of_class("deck-release")],
            [("5", "silent")],
        )
        outages = page.of_class("deck-outage")
        self.assertEqual(len(outages), 1)
        self.assertEqual(float(outages[0]["data-ms"]), 2000.0)
        # stdout names no key and no peer.
        self.assertNotIn(PEER, stdout)
        self.assertNotIn(PEER, page.text)

    def test_a_window_without_the_deck_says_so(self):
        log = Log()
        log.pings(7, BASE, BASE + 1000)
        summary, page, _ = self.report(log, BASE, BASE + 2000)
        self.assertEqual(
            (summary["deck_presses"], summary["deck_unsent"], summary["deck_rtt_p50_ms"]),
            ("0", "0", "n/a"),
        )
        self.assertIn("No Stream Deck activity in the window.", page.text)


class DeckHelpers(unittest.TestCase):
    def test_an_outage_runs_from_the_first_down_to_the_next_up(self):
        def at(ts, state):
            return {"ts": ts, "state": state, "error": f"e{ts}"}

        self.assertEqual(
            deck_outages([at(10, "down"), at(30, "up")], 0, 100),
            [DeckOutage(10, 30, 20, "e10")],
        )
        # Refused attempts during one outage: one outage, from the first.
        self.assertEqual(
            deck_outages([at(10, "refused"), at(15, "refused"), at(40, "up")], 0, 100),
            [DeckOutage(10, 40, 30, "e10")],
        )
        # Still down at the window's end.
        self.assertEqual(deck_outages([at(10, "down")], 0, 50), [DeckOutage(10, 50, 40, "e10")])
        self.assertEqual(deck_outages([at(5, "up")], 0, 50), [])
        # Before the window: left out; across its start: kept.
        self.assertEqual(deck_outages([at(10, "down"), at(20, "up")], 30, 100), [])
        self.assertEqual(
            deck_outages([at(-100, "down"), at(20, "up")], 0, 100),
            [DeckOutage(-100, 20, 120, "e-100")],
        )

    def test_a_press_gets_its_own_answer_by_client_and_seq(self):
        presses = [
            {"ts": 100, "client": 7, "seq": 1, "key": 3, "down": True, "forwarded": True},
            {"ts": 110, "client": 9, "seq": 1, "key": 3, "down": True, "forwarded": False, "reason": "held"},
        ]
        answers = [
            # An older answer of the same seq (a page counts from 1 again).
            {"ts": 50, "client": 7, "seq": 1, "rtt_ms": 99.0},
            {"ts": 105, "client": 9, "seq": 1, "rtt_ms": 77.0},
            {"ts": 106, "client": 7, "seq": 1, "rtt_ms": 6.0},
        ]
        got = deck_presses(presses, answers)
        self.assertEqual([p.rtt_ms for p in got], [6.0, None])
        self.assertEqual([p.reason for p in got], [None, "held"])


if __name__ == "__main__":
    unittest.main()
```

In `test_timeline.py` `test_the_summary_table_holds_stdouts_values`, the expected names list gains, after `"zooms",`: `"deck_presses", "deck_unsent", "deck_forced_releases", "deck_link_outages", "deck_rtt_p50_ms", "deck_rtt_p99_ms",` (one per line, as the list is written).

- [ ] **Step 2: Implement** — append to `tools/forensics/timeline_touch.py`:

```python
# --- the Stream Deck (#52) ---

DeckPress = collections.namedtuple(
    "DeckPress",
    (
        "time",
        "client",
        "seq",
        "key",
        "down",
        "delay_ms",
        "rtt_ms",
        "hold_ms",
        "hub_hold_ms",
        "forwarded",
        "reason",
        "why",
    ),
)
DeckRelease = collections.namedtuple("DeckRelease", ("time", "client", "key", "reason", "hub_hold_ms"))
DeckOutage = collections.namedtuple("DeckOutage", ("start", "end", "ms", "error"))
DeckUnsent = collections.namedtuple("DeckUnsent", ("time", "key", "where"))


def deck_presses(presses, answers):
    """The hub's ``deck_press`` records as ``DeckPress``es, each forwarded one
    with Companion's round trip: the ``rtt_ms`` of the first ``deck_ok`` of the
    same client and seq at or after it (a page's seq repeats across pages and
    after a reload)."""
    out = []
    for record in presses:
        client, seq = record.get("client"), record.get("seq")
        forwarded = record.get("forwarded") is True
        rtt = None
        if forwarded:
            rtt = next(
                (
                    number(a.get("rtt_ms"))
                    for a in answers
                    if a.get("client") == client and a.get("seq") == seq and a["ts"] >= record["ts"]
                ),
                None,
            )
        out.append(
            DeckPress(
                record["ts"],
                client,
                seq,
                record.get("key"),
                record.get("down") is True,
                number(record.get("delay_ms")),
                rtt,
                number(record.get("hold_ms")),
                number(record.get("hub_hold_ms")),
                forwarded,
                record.get("reason"),
                record.get("why"),
            )
        )
    return out


def deck_release(record):
    """A hub ``deck_release`` record (a release the hub made itself)."""
    return DeckRelease(
        record["ts"],
        record.get("client"),
        record.get("key"),
        record.get("reason"),
        number(record.get("hub_hold_ms")),
    )


def deck_outages(links, start, end):
    """Companion's link outages that overlap ``start``..``end``: from the
    first ``deck_link`` down or refused to the next up, else to ``end``
    (``links`` in time order)."""
    outages = []
    down_at, error = None, None
    for record in links:
        state = record.get("state")
        if state in ("down", "refused") and down_at is None:
            down_at, error = record["ts"], record.get("error")
        elif state == "up" and down_at is not None:
            outages.append(DeckOutage(down_at, record["ts"], record["ts"] - down_at, error))
            down_at = None
    if down_at is not None:
        outages.append(DeckOutage(down_at, end, end - down_at, error))
    return [o for o in outages if o.end >= start and o.start <= end]


def deck_unsent(events, presses):
    """The red flashes, in time order: a page's ``deck`` down it did not send
    (``where`` page) and a down the hub refused, Companion offline (hub)."""
    flashes = [
        DeckUnsent(e.hub, e.data.get("k"), "page")
        for e in events
        if e.data.get("d") == 1 and e.data.get("sent") is False
    ]
    flashes += [DeckUnsent(p.time, p.key, "hub") for p in presses if p.down and p.reason == "offline"]
    return sorted(flashes, key=lambda f: f.time)
```

and its module docstring gains a paragraph:

```
**The Stream Deck (#52):** the hub's ``deck_press`` records with Companion's
round trip (``deck_presses``: the ``deck_ok`` of the same client and seq),
its own releases (``deck_release``), Companion's link outages
(``deck_outages``: a down or refused to the next up) and the red flashes
(``deck_unsent``: a page's down it did not send, a down the hub refused
offline).
```

In `timeline_model.py` `Timeline.__init__`, after the `self.system = [...]` block:

```python
        # The Stream Deck (#52): presses with Companion's round trips, the
        # hub's own releases, the red flashes, Companion's link outages.
        self.deck_presses = timeline_touch.deck_presses(self._within(kinds["deck_press"]), kinds["deck_ok"])
        self.deck_releases = [timeline_touch.deck_release(r) for r in self._within(kinds["deck_release"])]
        self.deck_outages = timeline_touch.deck_outages(kinds["deck_link"], start, end)
        self.deck_unsent = timeline_touch.deck_unsent(
            [e for e in self.page if e.ev == "deck" and self.in_window(e.hub)], self.deck_presses
        )
```

and after `wanted`:

```python
    def _within(self, records):
        """``records`` of the window itself (not its lead)."""
        return [r for r in records if self.in_window(r["ts"])]
```

In `summary`, first line `deck_rtts = [p.rtt_ms for p in timeline.deck_presses if p.rtt_ms is not None]`, and after `("zooms", system_count(timeline, "zoom")),`:

```python
        ("deck_presses", str(sum(p.down for p in timeline.deck_presses))),
        ("deck_unsent", str(len(timeline.deck_unsent))),
        ("deck_forced_releases", str(len(timeline.deck_releases))),
        ("deck_link_outages", str(len(timeline.deck_outages))),
        ("deck_rtt_p50_ms", ms_text(percentile(deck_rtts, 0.50))),
        ("deck_rtt_p99_ms", ms_text(percentile(deck_rtts, 0.99))),
```

The module docstring: "… the system's gestures (PR G), the Stream Deck's presses (#52) and the summary."

In `timeline_report.py`, CSS gains `tr.deck-unsent td, tr.deck-release td { color: #a00; }`; before `render`:

```python
DECK_LEGEND = (
    "Stream Deck (#52): each press with the hub's delay (the page's send to the hub), "
    "Companion's round trip, and the hold as the page measured it and as the hub forwarded "
    "it; then the releases the hub made itself, the red flashes (downs never sent) and "
    "Companion's link outages. Keys are their index."
)


def _deck_table(t):
    if not (t.deck_presses or t.deck_releases or t.deck_unsent or t.deck_outages):
        return "<p>No Stream Deck activity in the window.</p>"
    head = "".join(
        f"<th>{h}</th>"
        for h in ("time (local)", "key", "press", "delay", "Companion", "hold (page)", "hold (hub)", "outcome")
    )
    rows = []
    for p in t.deck_presses:
        cells = (
            local_text(p.time),
            str(p.key),
            "down" if p.down else f"up ({p.why or 'n/a'})",
            ms_text(p.delay_ms),
            ms_text(p.rtt_ms),
            ms_text(p.hold_ms),
            ms_text(p.hub_hold_ms),
            "forwarded" if p.forwarded else (p.reason or "n/a"),
        )
        rows.append(
            tag(
                "tr",
                "".join(f"<td>{esc(c)}</td>" for c in cells),
                class_="deck-press",
                data_time=num(p.time),
                data_key=str(p.key),
                data_down="true" if p.down else "false",
                data_forwarded="true" if p.forwarded else "false",
                data_reason=p.reason,
                data_why=p.why,
                data_rtt=None if p.rtt_ms is None else num(p.rtt_ms),
            )
        )
    for r in t.deck_releases:
        cells = (local_text(r.time), str(r.key), "up (the hub's)", "", "", "", ms_text(r.hub_hold_ms), r.reason or "n/a")
        rows.append(
            tag(
                "tr",
                "".join(f"<td>{esc(c)}</td>" for c in cells),
                class_="deck-release",
                data_time=num(r.time),
                data_key=str(r.key),
                data_reason=r.reason,
            )
        )
    for u in t.deck_unsent:
        cells = (local_text(u.time), str(u.key), "down (red flash)", "", "", "", "", f"not sent ({u.where})")
        rows.append(
            tag(
                "tr",
                "".join(f"<td>{esc(c)}</td>" for c in cells),
                class_="deck-unsent",
                data_time=num(u.time),
                data_key=str(u.key),
                data_where=u.where,
            )
        )
    table = f'<table class="deck"><tr>{head}</tr>{"".join(rows)}</table>'
    outages = "".join(
        tag(
            "li",
            esc(f"{local_text(o.start)} to {local_text(o.end)}: {ms_text(o.ms)} ms ({o.error or 'n/a'})"),
            class_="deck-outage",
            data_ms=num(o.ms),
        )
        for o in t.deck_outages
    )
    outage_list = f"<p>Companion link outages:</p><ul>{outages}</ul>" if outages else ""
    return f"<p>{esc(DECK_LEGEND)}</p>{table}{outage_list}"
```

and `render` gains, after the system gestures: `f"<h2>Stream Deck</h2>{_deck_table(t)}"`.

`timeline.py`'s docstring, after the system gestures sentence: "then the Stream Deck (#52, the hub's ``deck_*`` records and the page's ``deck`` events): each press with the hub's delay, Companion's round trip and the holds (page and hub), the releases the hub made itself, the red flashes and Companion's link outages." The summary sentence gains the deck names.

`.claude/rules/forensics.md`: a new bullet after "System gestures (PR G)":

```markdown
- **The Stream Deck (#52):** the hub's `deck_press` records in the window (`timeline_touch.deck_presses`: Companion's round trip from the first `deck_ok` of the same client and seq at or after it; a page's seq repeats across pages), `deck_release` (the hub's own releases: `detach`, `silent`, `reconnect`, `stop`), `deck_link` outages (`deck_outages`: the first down or refused to the next up, records of the lead count, cut to the window's end) and the red flashes (`deck_unsent`: a page `deck` down with `sent` false, a hub `deck_press` down with `reason` offline). The report's "Stream Deck" section rows carry `class="deck-press"` (`data-time`, `data-key`, `data-down`, `data-forwarded`, `data-reason`, `data-why`, `data-rtt`), `deck-release` (`data-key`, `data-reason`), `deck-unsent` (`data-key`, `data-where` page/hub) and `deck-outage` list items (`data-ms`). stdout `deck_presses` (downs), `deck_unsent`, `deck_forced_releases`, `deck_link_outages`, `deck_rtt_p50_ms`, `deck_rtt_p99_ms` after `zooms`; keys are their index, no names. Tests: `test_timeline_deck.py`.
```

and in the stdout bullet, after `zooms` (PR G): `` `deck_presses`, `deck_unsent`, `deck_forced_releases`, `deck_link_outages`, `deck_rtt_p50_ms`, `deck_rtt_p99_ms` (#52), ``.

- [ ] **Step 3: Run the tests locally**

```bash
python3 -m unittest discover -s tools/forensics -p 'test_*.py' -v 2>&1 | tail -5
ruff check tools && ruff format --check tools
```

Expected: every test passes (`Ran N tests … OK`), no skips; ruff clean (run `ruff format tools/forensics` first if it reformats).

- [ ] **Step 4: Commit**

```bash
git add tools/forensics .claude/rules/forensics.md
git commit -m "feat(forensics): the timeline's Stream Deck section (#52)

Each press with the hub's delay, Companion's round trip and the holds of
the page and the hub; the hub's own releases, the red flashes and
Companion's link outages; six new summary counts, numbers only.

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---
### Task 14: The installer writes and keeps `[companion]`

**Files:**
- Modify: `scripts/fohmixer-pc/FohmixerPc.psm1` (`New-FohHubToml` :301-321 and new functions after it, `Invoke-FohInstall` params :555-573 and :622, its result :726-729)
- Modify: `scripts/fohmixer-pc/Install-Fohmixer.ps1` (help :1-60, params :62-110, body :111-125)
- Modify: `scripts/fohmixer-pc/Test-Fohmixer.ps1` (after :452, and a new install block after install 7)
- Modify: `.claude/rules/deploy-pc.md`

**Interfaces:**
- Consumes: the hub's `config check` (Task 2's validation).
- Produces: `Get-FohCompanionToml -Endpoint <host[:port]>` → CRLF text `"\r\n[companion]\r\nhost = \"<host>\"\r\nport = <port>\r\n"` ('' for ''); `Get-FohInstalledCompanionToml -Text <toml>` → the installed table as written, '' without one; `Resolve-FohCompanion -DataDir <dir> [-CompanionHost <host[:port]>]`; `New-FohHubToml … -Companion <text>` (the table between the instances and the remote tables); `Invoke-FohInstall … -Companion <text>`; `Install-Fohmixer.ps1 -CompanionHost <host[:port]>`; the install result's `companion` (bool).

- [ ] **Step 1: Write the failing self-tests** — in `scripts/fohmixer-pc/Test-Fohmixer.ps1`, after the `installed-remote-tables-none-without-tls` assertion (:452):

```powershell
    # ---- the Stream Deck tab (#52): the [companion] table ----
    $wantCompanion = "`r`n[companion]`r`nhost = `"companion.example.org`"`r`nport = 16622`r`n"
    Assert ((Get-FohCompanionToml -Endpoint 'companion.example.org') -ceq $wantCompanion) 'companion-toml-default-port'
    Assert ((Get-FohCompanionToml -Endpoint '10.0.0.7:16700') -ceq "`r`n[companion]`r`nhost = `"10.0.0.7`"`r`nport = 16700`r`n") 'companion-toml-with-a-port'
    Assert ((Get-FohCompanionToml -Endpoint '') -ceq '') 'companion-toml-empty-without-an-endpoint'
    foreach ($bad in @('a b', 'host:', 'host:0', 'host:70000', 'h"st', ':16622', 'host:16622:1')) {
        Assert ((ErrorOf { Get-FohCompanionToml -Endpoint $bad }) -like '*Companion endpoint refused*') "companion-toml-refuses-[$bad]"
    }
    $withBoth = New-FohHubToml -HttpPort 1 -BandPort 2 -MasterPort 3 -Companion $wantCompanion -Remote $gotRemote
    Assert ($withBoth.IndexOf('[companion]') -lt $withBoth.IndexOf('[tls]')) 'companion-table-before-the-remote-tables'
    Assert ((Get-FohInstalledCompanionToml -Text $withBoth) -ceq $wantCompanion) 'installed-companion-table-read-back-before-tls'
    Assert ((Get-FohInstalledRemoteToml -Text $withBoth) -ceq $gotRemote) 'installed-remote-tables-unchanged-by-companion'
    Assert ((Get-FohInstalledCompanionToml -Text ((New-FohHubToml -HttpPort 1 -BandPort 2 -MasterPort 3) + $wantCompanion)) -ceq $wantCompanion) 'installed-companion-table-read-back-at-the-end'
    Assert ((Get-FohInstalledCompanionToml -Text (New-FohHubToml -HttpPort 1 -BandPort 2 -MasterPort 3)) -ceq '') 'installed-companion-none-without-the-table'
    $deckToml = New-FohHubToml -HttpPort 18481 -BandPort 39181 -MasterPort 39182 -Companion $wantCompanion -Remote $gotRemote
    Assert ((Test-FohHubToml -Exe $HubExe -Text $deckToml -Dir $checkDir) -like '*: OK*') 'hub-toml-check-accepts-the-companion-table'
    $badDeck = New-FohHubToml -HttpPort 18481 -BandPort 39181 -MasterPort 39182 -Companion ($wantCompanion + "columns = 17`r`n")
    Assert ((ErrorOf { Test-FohHubToml -Exe $HubExe -Text $badDeck -Dir $checkDir }) -like '*refused by the hub (exit 2)*columns 17*') 'hub-toml-check-refuses-a-bad-companion-table'
    Assert (@(Get-ChildItem -LiteralPath $checkDir -Force).Count -eq 0) 'hub-toml-check-of-the-companion-table-leaves-no-file'
```

and after the install-7 block (`install-7-keeps-the-remote-access`), before the refusals loop that follows it:

```powershell
    # ---- install 8: the Stream Deck tab (#52): -CompanionHost writes [companion], an update keeps it ----
    $dataC = Join-Path $base 'data-companion'
    $r8 = Invoke-Ps $install (Get-InstallArgs @{ DataDir = $dataC; CompanionHost = '10.0.0.7:16700' })
    Assert ($r8.code -eq 0) "install-8-companion-exits-0 ($($r8.out))"
    $wantC = New-FohHubToml -HttpPort 18481 -BandPort 39181 -MasterPort 39182 -Companion (Get-FohCompanionToml -Endpoint '10.0.0.7:16700')
    Assert ([IO.File]::ReadAllText((Join-Path $dataC 'fohmixer-hub.toml')) -ceq $wantC) 'install-8-toml-has-the-companion-table'
    $r9 = Invoke-Ps $install (Get-InstallArgs @{ DataDir = $dataC })
    Assert ($r9.code -eq 0) 'install-9-without-companion-host-exits-0'
    Assert ([IO.File]::ReadAllText((Join-Path $dataC 'fohmixer-hub.toml')) -ceq $wantC) 'install-9-without-companion-host-keeps-the-table'
    $noneC = Join-Path $base 'data-companion-refused'
    $res = Invoke-Ps $install (Get-InstallArgs @{ DataDir = $noneC; CompanionHost = 'host:0' })
    Assert ($res.code -ne 0 -and $res.out -like '*Companion endpoint refused*' -and -not (Test-Path -LiteralPath $noneC)) 'install-refuses-a-bad-companion-host-before-any-change'
```

- [ ] **Step 2: Implement** — in `scripts/fohmixer-pc/FohmixerPc.psm1`, `New-FohHubToml` gains a parameter and its doc a line:

```powershell
function New-FohHubToml {
    # <DataDir>\fohmixer-hub.toml as the hub's config.rs reads it: the HTTP
    # port, the two instances and the layout file (relative to the data folder),
    # then $Companion (Get-FohCompanionToml: the Stream Deck tab, #52), then
    # $Remote (Get-FohRemoteToml: the remote-access tables, #17, always last:
    # Get-FohInstalledRemoteToml reads from [tls] to the end).
    # There is no data_dir key: the hub's data folder is FOHMIXER_DATA.
    param([Parameter(Mandatory)][int]$HttpPort, [Parameter(Mandatory)][int]$BandPort, [Parameter(Mandatory)][int]$MasterPort,
          [AllowEmptyString()][string]$Companion = '', [AllowEmptyString()][string]$Remote = '')
    $lines = @(
        '# fohmixer-hub configuration, written by Install-Fohmixer.ps1: run the install again to change it.',
        ('http_port = {0}' -f $HttpPort),
        'layout = "layout.json"',
        '',
        '[[instances]]',
        'name = "band"',
        ('port = {0}' -f $BandPort),
        '',
        '[[instances]]',
        'name = "master"',
        ('port = {0}' -f $MasterPort))
    return (($lines -join "`r`n") + "`r`n" + $Companion + $Remote)
}

function Get-FohCompanionToml {
    # The [companion] table of fohmixer-hub.toml (#52, CRLF lines, '' without
    # -Endpoint): Bitfocus Companion's Satellite API as host or host:port
    # (default port 16622). The grid, the image size and the title keep the
    # hub's defaults; the hub checks the table (config check).
    param([AllowEmptyString()][string]$Endpoint = '')
    if (-not $Endpoint) { return '' }
    $m = [regex]::Match($Endpoint, '\A(?<host>[^\s:"]+)(:(?<port>\d{1,5}))?\z')
    $port = 16622
    if ($m.Success -and $m.Groups['port'].Success) { $port = [int]$m.Groups['port'].Value }
    if (-not $m.Success -or $port -lt 1 -or $port -gt 65535) {
        throw "Companion endpoint refused: [$Endpoint] (host or host:port, port 1-65535)"
    }
    $lines = @('', '[companion]', ('host = "{0}"' -f $m.Groups['host'].Value), ('port = {0}' -f $port))
    return (($lines -join "`r`n") + "`r`n")
}

function Get-FohInstalledCompanionToml {
    # The [companion] table of an installed fohmixer-hub.toml's text (#52), from
    # the line end before it to the next table or the end, exactly as
    # Get-FohCompanionToml wrote it; '' without one.
    param([Parameter(Mandatory)][AllowEmptyString()][string]$Text)
    $m = [regex]::Match($Text, '(?s)\r?\n\[companion\]\r?\n.*?(?=\r?\n\[|\z)')
    if ($m.Success) { return $m.Value }
    return ''
}

function Resolve-FohCompanion {
    # The [companion] table of an install (#52), checked before any change:
    # from -CompanionHost when given, else the installed toml's as it is (a
    # routine update keeps the Stream Deck tab), else ''.
    param([Parameter(Mandatory)][string]$DataDir, [AllowEmptyString()][string]$CompanionHost = '')
    if ($CompanionHost) { return (Get-FohCompanionToml -Endpoint $CompanionHost) }
    $toml = Join-Path (Resolve-FohPath $DataDir) 'fohmixer-hub.toml'
    if (-not (Test-Path -LiteralPath $toml -PathType Leaf)) { return '' }
    return (Get-FohInstalledCompanionToml -Text ([IO.File]::ReadAllText($toml)))
}
```

(The installed table is read back exactly: `Get-FohCompanionToml` ends its text with `\r\n`, and the lookahead stops right before the `\r\n[` of the next table or at the end.)

`Invoke-FohInstall`: its params gain `[AllowEmptyString()][string]$Companion = '',` (after `$Remote = $null` add it before, keeping `$Remote` last: `[AllowEmptyString()][string]$Companion = '',\n        $Remote = $null`); its doc comment gains "$Companion is the [companion] table (#52, Resolve-FohCompanion)"; the toml line becomes `$tomlText = New-FohHubToml -HttpPort $HttpPort -BandPort $BandPort -MasterPort $MasterPort -Companion $Companion -Remote $remoteToml`; the result gains `companion = [bool]$Companion;` (after `remote = $remoteResult`).

In `scripts/fohmixer-pc/Install-Fohmixer.ps1`: the help's description gains, after the remote-access paragraph:

```
The Stream Deck tab (#52), only with -CompanionHost: the toml gets [companion]
(Bitfocus Companion's Satellite API as host or host:port, default port 16622);
the hub registers there as one more Stream Deck ("fohmixer"). A run without
-CompanionHost keeps an installed [companion] table as it is.
```

the params gain (after `[string]$BandDesktop = ''`, with a comma after it):

```powershell
    # The Stream Deck tab (#52): Bitfocus Companion's Satellite API as host or host:port
    # (default port 16622). Without it an installed [companion] table is kept.
    [string]$CompanionHost = ''
```

and the body, after `$remote = Resolve-FohRemote …`:

```powershell
    # The Stream Deck tab (#52): checked before any change, kept when not given.
    $companion = Resolve-FohCompanion -DataDir $DataDir -CompanionHost $CompanionHost
```

with `-Companion $companion` added to the `Invoke-FohInstall` call (before `-Remote $remote`).

- [ ] **Step 3: Playbook** — in `.claude/rules/deploy-pc.md`, after the remote-access bullet:

```markdown
- **The Stream Deck tab (#52, `-CompanionHost <host[:port]>`):** the install writes `[companion]` (`host`, `port`, default 16622; the grid, image size and title keep the hub's defaults) between the instances and the remote tables (`Get-FohCompanionToml`); a run without it keeps an installed table as it is (`Get-FohInstalledCompanionToml`, read from the line before `[companion]` to the next table), and the new hub checks the toml before the stop. The host is site data: pass it at run time; on the PC read Companion's address from Companion Satellite's established connection (`Get-NetTCPConnection -RemotePort 16622 -State Established`), never print it, write it straight into `install.cmd`. The install result's `companion` says whether the table is there. After the install: the hub log's `Stream Deck registered with Companion` with `companion=5.0.7…`, the event log's `deck_link` `up`, `/api/status` `companion.online` and `keys` 32, and the tab with 32 images in a browser (no key pressed: they switch real lights and sockets). One-time owner step in Companion's Surfaces tab: the "fohmixer" surface's start page, or a group with the physical deck.
```

- [ ] **Step 4: Verify** — nothing runs PowerShell locally. At Checkpoint C, CI `windows` runs `Test-Fohmixer.ps1` (the new assertions: the table's text, the refusals, the read-back before `[tls]` and at the end, the hub's check accepting the table and refusing `columns = 17`, install 8 writing it, install 9 keeping it, the refused `host:0` before any change); `integrity` checks the scripts (no force-kill verb).

- [ ] **Step 5: Commit**

```bash
git add scripts/fohmixer-pc/FohmixerPc.psm1 scripts/fohmixer-pc/Install-Fohmixer.ps1 \
  scripts/fohmixer-pc/Test-Fohmixer.ps1 .claude/rules/deploy-pc.md
git commit -m "feat(pc): the installer writes the Stream Deck's [companion] table (#52)

-CompanionHost <host[:port]> writes it before the remote tables; an
update without it keeps the installed table; the new hub checks it before
the stop.

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---
### Task 15: The fake Companion and the harness

**Files:**
- Create: `e2e/harness/fake_companion.py`
- Create: `e2e/harness/test_fake_companion.py`
- Modify: `e2e/harness/harness.py` (docstring :1-58, imports :62-77, `hub_config` :103-126, new `on_of` and `companion_of`, `Harness.__init__` :407-439, `handle` :453-492, `stop` :494-499, `parse_args` :532-552)
- Modify: `e2e/harness/test_harness.py` (new tests)
- Modify: `.github/workflows/ci.yml` (the `e2e` job's harness step :398-410: `--fake-companion-port 39192`)

**Interfaces:**
- Consumes: the hub's Satellite lines (Task 3).
- Produces: `fake_companion.FakeCompanion(port, host="127.0.0.1")` with `start()`, `stop()`, `down()`, `up()`, `fail(on)`, `clear()`, `state()` (`{"connections", "presses": [{"key", "pressed", "at"}], "down", "failing"}`), `.port`; `fake_companion.params(line)`, `colour(key, pressed)`, `png(rgb, side=8)`, `key_state(device, key, columns, pressed)`; `harness.hub_config(http_port, band_port, master_port, remote=None, companion=None)`; `harness.companion_of(args, fake)`; `harness.on_of(body, route)`; harness args `--fake-companion-port N`, `--companion HOST:PORT`; routes `GET /companion`, `POST /companion/down|up|clear`, `POST /companion/fail {"on"}`, `POST /hub/companion {"on"}`.

- [ ] **Step 1: Write the failing tests** — `e2e/harness/test_fake_companion.py`:

```python
"""The fake Companion (#52, ``fake_companion.py``) as the hub sees it: BEGIN
and CAPS first; ADD-DEVICE answered OK, BRIGHTNESS and one KEY-STATE per key
(a PNG data URL); a press answered OK and then the key's new state (ERROR
while failing), and recorded; PING answered PONG; an unknown command a bare
ERROR; every line ending with a space, as Companion 5.0.7 writes them. down
closes every connection and new ones at once; up lets them in again."""

import base64
import os
import socket
import struct
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import fake_companion  # noqa: E402

# How long a test waits for a line that must come.
COME_S = 3.0
ADD = (
    'ADD-DEVICE DEVICEID="fohmixer-1" SERIAL="fohmixer" PRODUCT_NAME="fohmixer" '
    "KEYS_TOTAL=32 KEYS_PER_ROW=8 BITMAPS=144 BITMAP_FORMAT=webp COLORS=hex TEXT=0 BRIGHTNESS=0"
)


class Conn:
    """A client of the fake: lines in and out."""

    def __init__(self, port):
        self.sock = socket.create_connection(("127.0.0.1", port), timeout=COME_S)
        self.file = self.sock.makefile("rb")

    def read(self):
        """The next line, its line end kept; None once the fake closed it."""
        raw = self.file.readline()
        return raw.decode() if raw else None

    def send(self, text):
        self.sock.sendall((text + "\n").encode())

    def close(self):
        self.file.close()
        self.sock.close()


class FakeCompanionTest(unittest.TestCase):
    def setUp(self):
        self.fake = fake_companion.FakeCompanion(0)
        self.fake.start()
        self.addCleanup(self.fake.stop)

    def connect(self):
        conn = Conn(self.fake.port)
        self.addCleanup(conn.close)
        return conn

    def registered(self):
        """A connection past ADD-DEVICE and its 32 key states."""
        conn = self.connect()
        conn.read()
        conn.read()
        conn.send(ADD)
        conn.read()
        conn.read()
        for _ in range(32):
            conn.read()
        return conn

    def test_the_handshake_and_the_keys(self):
        conn = self.connect()
        self.assertEqual(conn.read(), 'BEGIN CompanionVersion="5.0.7+fake" ApiVersion="1.12.0" \n')
        self.assertEqual(
            conn.read(), 'CAPS SUBSCRIPTIONS=0 NONSQUARE=1 BITMAP_FORMATS="rgb,png,webp" \n'
        )
        conn.send(ADD)
        self.assertEqual(conn.read(), 'ADD-DEVICE OK DEVICEID="fohmixer-1" \n')
        self.assertEqual(conn.read(), 'BRIGHTNESS DEVICEID="fohmixer-1" VALUE=100 \n')
        states = [conn.read() for _ in range(32)]
        self.assertTrue(all(s.startswith("KEY-STATE ") and s.endswith(" \n") for s in states))
        params = fake_companion.params(states[9].rstrip("\n"))
        self.assertEqual(
            (params["DEVICEID"], params["KEY"], params["LOCATION"], params["PRESSED"]),
            ("fohmixer-1", "9", "1/1/1", "0"),
        )
        header, data = params["BITMAP"].split(",", 1)
        self.assertEqual(header, "data:image/png;base64")
        image = base64.b64decode(data)
        self.assertEqual(image[:8], b"\x89PNG\r\n\x1a\n")
        self.assertEqual(struct.unpack(">II", image[16:24]), (8, 8))
        self.assertEqual(params["COLOR"], "#%02x%02x%02x" % fake_companion.colour(9, False))
        self.assertEqual(self.fake.state()["connections"], 1)

    def test_a_press_is_answered_recorded_and_redrawn(self):
        conn = self.registered()
        conn.send('KEY-PRESS DEVICEID="fohmixer-1" KEY=3 PRESSED=1')
        self.assertEqual(conn.read(), 'KEY-PRESS OK DEVICEID="fohmixer-1" \n')
        state = fake_companion.params(conn.read().rstrip("\n"))
        self.assertEqual((state["KEY"], state["PRESSED"]), ("3", "1"))
        self.assertEqual(state["COLOR"], "#%02x%02x%02x" % fake_companion.colour(3, True))
        self.assertNotEqual(fake_companion.colour(3, True), fake_companion.colour(3, False))
        self.assertEqual(
            [(p["key"], p["pressed"]) for p in self.fake.state()["presses"]], [(3, True)]
        )
        self.assertEqual(self.fake.fail(True)["failing"], True)
        conn.send('KEY-PRESS DEVICEID="fohmixer-1" KEY=3 PRESSED=0')
        self.assertEqual(
            conn.read(), 'KEY-PRESS ERROR DEVICEID="fohmixer-1" MESSAGE="test refusal" \n'
        )
        self.assertEqual(len(self.fake.state()["presses"]), 2, "a refused press is recorded too")
        self.assertEqual(self.fake.clear()["presses"], [])
        conn.send("PING 7")
        self.assertEqual(conn.read(), "PONG 7 \n")
        conn.send("FOO")
        self.assertEqual(conn.read(), 'ERROR MESSAGE="Unknown command: FOO" \n')

    def test_down_closes_every_connection_and_up_lets_them_in(self):
        conn = self.registered()
        self.assertEqual(self.fake.down()["down"], True)
        self.assertIsNone(conn.read(), "closed")
        self.assertIsNone(self.connect().read(), "a new connection is closed at once")
        self.assertEqual(self.fake.up()["down"], False)
        self.assertTrue(self.connect().read().startswith("BEGIN "))

    def test_a_png_is_a_solid_square(self):
        image = fake_companion.png((10, 20, 30), side=4)
        self.assertEqual(image[:8], b"\x89PNG\r\n\x1a\n")
        self.assertEqual(struct.unpack(">II", image[16:24]), (4, 4))
        self.assertEqual(fake_companion.params('X A=1 B="x y" C'), {"A": "1", "B": "x y"})


if __name__ == "__main__":
    unittest.main()
```

In `e2e/harness/test_harness.py`, `HarnessTest` gains:

```python
    def test_the_companion_table_of_the_hub_config(self):
        self.assertEqual(
            harness.hub_config(8480, 1, 2, None, ("127.0.0.1", 39192)),
            harness.hub_config(8480, 1, 2) + '[companion]\nhost = "127.0.0.1"\nport = 39192\n',
        )
        both = harness.hub_config(
            8480, 1, 2, {"name": "foh.e2e.test", "https_port": 8443}, ("127.0.0.1", 39192)
        )
        self.assertLess(both.index("[companion]"), both.index("[tls]"))

    def test_without_a_fake_companion_its_routes_are_absent(self):
        self.assertEqual(self.harness.handle("GET", "/companion", {})[0], 404)
        self.assertEqual(self.post("/companion/down")[0], 404)
        self.assertEqual(self.post("/hub/companion", {"on": True})[0], 404, "no hub in this test")
```

and a new class at the end of the file (before `if __name__`… if there is one):

```python
class FakeCompanionRoutes(unittest.TestCase):
    """The harness with ``--fake-companion-port`` (#52): the hub's config names
    the fake, the routes drive it."""

    @classmethod
    def setUpClass(cls):
        cls.data = tempfile.mkdtemp(prefix="fohmixer-harness-test-")
        args = harness.parse_args(
            [
                "--data", cls.data, "--layout", LAYOUT, "--band-port", "0",
                "--master-port", "0", "--meters-hz", "0", "--fake-companion-port", "0",
            ]
        )
        cls.harness = harness.Harness(args)

    @classmethod
    def tearDownClass(cls):
        cls.harness.stop()
        shutil.rmtree(cls.data, ignore_errors=True)

    def test_the_config_names_the_fake_and_the_routes_drive_it(self):
        port = self.harness.fake.port
        with open(os.path.join(self.data, "fohmixer-hub.toml"), encoding="utf-8") as f:
            self.assertIn(f'[companion]\nhost = "127.0.0.1"\nport = {port}\n', f.read())
        handle = self.harness.handle
        self.assertEqual(
            handle("GET", "/companion", {}),
            (200, {"connections": 0, "presses": [], "down": False, "failing": False}),
        )
        self.assertEqual(handle("POST", "/companion/down", {})[1]["down"], True)
        self.assertEqual(handle("POST", "/companion/up", {})[1]["down"], False)
        self.assertEqual(handle("POST", "/companion/fail", {"on": True})[1]["failing"], True)
        self.assertEqual(handle("POST", "/companion/fail", {"on": False})[1]["failing"], False)
        self.assertEqual(handle("POST", "/companion/clear", {})[1]["presses"], [])
        with self.assertRaises(harness.BadRequest):
            handle("POST", "/companion/fail", {})

    def test_a_real_companion_is_named_by_host_and_port(self):
        args = harness.parse_args(["--data", "d", "--layout", "l", "--companion", "127.0.0.1:16622"])
        self.assertEqual(harness.companion_of(args, None), ("127.0.0.1", 16622))
        none = harness.parse_args(["--data", "d", "--layout", "l"])
        self.assertIsNone(harness.companion_of(none, None))
        for bad in ("16622", "host:", ":16622", "host:x"):
            with self.assertRaises(SystemExit):
                harness.companion_of(
                    harness.parse_args(["--data", "d", "--layout", "l", "--companion", bad]), None
                )
```

(`ruff format` rewrites the compact argument list of `setUpClass` one per line; run it before committing.)

- [ ] **Step 2: Implement the fake** — `e2e/harness/fake_companion.py`:

```python
"""A fake Bitfocus Companion for the E2E suite (#52), beside ``impair.py``:
the Satellite API on TCP as Companion 5.0.7 speaks it (checked against the
real one, ``docs/superpowers/plans/2026-10-06-streamdeck-tab.md``), with the
faults the tests turn on:

    fake = FakeCompanion(port)       # 0: any free port, see ``port``
    fake.start()
    fake.down()                      # every connection closed, new ones closed at once
    fake.up()
    fake.fail(True)                  # KEY-PRESS answered ERROR
    fake.clear()                     # forget the presses
    fake.state()                     # connections, presses, down, failing
    fake.stop()

Each key is a small solid PNG (stdlib ``zlib``), coloured by its number and
brighter while pressed; a press is answered ``KEY-PRESS OK`` (or ERROR) and
then the key's new state, as Companion does. Every line ends with a space
before its ``\\n``, as Companion writes them. Stdlib only, one asyncio loop
in a thread of its own; every control call and failure is printed on stderr
(``fake_companion: …``, the harness's log). Nothing is ever force-ended: a
connection is closed, the loop stopped.
"""

import asyncio
import base64
import struct
import sys
import threading
import time
import zlib

# How long a control call waits for the loop.
CALL_S = 5.0
# How long start() waits for the listener.
START_S = 5.0
VERSION = "5.0.7+fake"
API = "1.12.0"


def log(message):
    print(f"fake_companion: {message}", file=sys.stderr, flush=True)


def params(line):
    """A Satellite line's ``NAME=value`` parameters (quotes removed; a value
    may hold spaces inside its quotes); bare words left out."""
    found = {}
    rest = line.split(" ", 1)[1] if " " in line else ""
    while rest:
        rest = rest.lstrip(" ")
        name, eq, after = rest.partition("=")
        if not eq or " " in name:
            # A bare word: skip it.
            rest = rest.partition(" ")[2]
            continue
        if after.startswith('"'):
            value, _, rest = after[1:].partition('"')
        else:
            value, _, rest = after.partition(" ")
        found[name] = value
    return found


def png(rgb, side=8):
    """A solid ``side`` × ``side`` PNG of ``rgb``."""

    def chunk(kind, data):
        crc = zlib.crc32(kind + data) & 0xFFFFFFFF
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", crc)

    raw = (b"\x00" + bytes(rgb) * side) * side
    header = struct.pack(">IIBBBBB", side, side, 8, 2, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", header)
        + chunk(b"IDAT", zlib.compress(raw))
        + chunk(b"IEND", b"")
    )


def colour(key, pressed):
    """A key's colour: its own, brighter while pressed."""
    base = ((key * 53) % 200, (key * 97) % 200, (key * 31) % 200)
    return tuple(min(255, c + 55) for c in base) if pressed else base


def key_state(device, key, columns, pressed):
    """A ``KEY-STATE`` line as Companion 5.0.7 writes it (a PNG bitmap)."""
    rgb = colour(key, pressed)
    data = base64.b64encode(png(rgb)).decode()
    row, column = divmod(key, columns)
    return (
        f'KEY-STATE DEVICEID="{device}" KEY={key} LOCATION="1/{row}/{column}" '
        f'PRESSED={int(pressed)} TYPE="BUTTON" BITMAP="data:image/png;base64,{data}" '
        f'COLOR="#{rgb[0]:02x}{rgb[1]:02x}{rgb[2]:02x}" TEXTCOLOR="#ffffff" '
    )


class FakeCompanion:
    """The fake on ``port`` of ``host`` (0: any free port, see ``port``)."""

    def __init__(self, port, host="127.0.0.1"):
        self.host = host
        self.listen_port = port
        self.port = None
        self.loop = asyncio.new_event_loop()
        self.thread = threading.Thread(target=self._run, name="fake-companion", daemon=True)
        self.listening = threading.Event()
        self.failure = None
        self.server = None
        # Loop state: only touched on the loop.
        self.writers = set()
        self.presses = []
        self.is_down = False
        self.failing = False

    def start(self):
        """Starts the loop and the listener; raises when it cannot listen."""
        self.thread.start()
        if not self.listening.wait(START_S):
            raise RuntimeError(f"the fake Companion did not listen within {START_S} s")
        if self.failure is not None:
            raise RuntimeError(f"the fake Companion cannot listen: {self.failure!r}")
        log(f"listening on {self.host}:{self.port}")

    def _run(self):
        asyncio.set_event_loop(self.loop)
        try:
            self.server = self.loop.run_until_complete(
                asyncio.start_server(self._accept, self.host, self.listen_port)
            )
        except OSError as e:
            self.failure = e
            self.listening.set()
            self.loop.close()
            return
        self.port = self.server.sockets[0].getsockname()[1]
        self.listening.set()
        self.loop.run_forever()
        self.loop.close()

    def _call(self, fn, *args):
        async def call():
            return fn(*args)

        return asyncio.run_coroutine_threadsafe(call(), self.loop).result(CALL_S)

    # The control calls (any thread).

    def down(self):
        """Companion away: every connection closed, new ones closed at once."""
        answer = self._call(self._down)
        log(f"down: {answer}")
        return answer

    def up(self):
        answer = self._call(self._up)
        log(f"up: {answer}")
        return answer

    def fail(self, on):
        """Presses answered ERROR (``on``) or OK."""
        answer = self._call(self._fail, bool(on))
        log(f"fail {bool(on)}: {answer}")
        return answer

    def clear(self):
        """Forgets the presses."""
        return self._call(self._clear)

    def state(self):
        """``connections``, ``presses`` (``key``, ``pressed``, ``at``: epoch ms),
        ``down``, ``failing``."""
        return self._call(self._state)

    def stop(self):
        """Closes every connection and the listener, ends the loop."""
        if not self.thread.is_alive():
            return

        async def close():
            self._down()
            self.server.close()
            await self.server.wait_closed()

        asyncio.run_coroutine_threadsafe(close(), self.loop).result(CALL_S)
        self.loop.call_soon_threadsafe(self.loop.stop)
        self.thread.join(CALL_S)
        log("stopped")

    # On the loop.

    def _down(self):
        self.is_down = True
        for writer in list(self.writers):
            writer.close()
        return self._state()

    def _up(self):
        self.is_down = False
        return self._state()

    def _fail(self, on):
        self.failing = on
        return self._state()

    def _clear(self):
        self.presses = []
        return self._state()

    def _state(self):
        return {
            "connections": len(self.writers),
            "presses": [dict(p) for p in self.presses],
            "down": self.is_down,
            "failing": self.failing,
        }

    async def _accept(self, reader, writer):
        if self.is_down:
            writer.close()
            return
        self.writers.add(writer)
        try:
            await self._serve(reader, writer)
        except (ConnectionError, OSError) as e:
            log(f"a connection ended: {e!r}")
        finally:
            self.writers.discard(writer)
            writer.close()

    async def _serve(self, reader, writer):
        def send(line):
            writer.write((line + "\n").encode())

        send(f'BEGIN CompanionVersion="{VERSION}" ApiVersion="{API}" ')
        send('CAPS SUBSCRIPTIONS=0 NONSQUARE=1 BITMAP_FORMATS="rgb,png,webp" ')
        await writer.drain()
        device, columns = "", 8
        while True:
            raw = await reader.readline()
            if not raw:
                return
            line = raw.decode(errors="replace").rstrip("\r\n")
            cmd = line.split(" ", 1)[0]
            found = params(line)
            if cmd == "PING":
                send("PONG " + line[5:].strip() + " ")
            elif cmd == "ADD-DEVICE":
                device = found.get("DEVICEID", "")
                total = int(found.get("KEYS_TOTAL", "0"))
                columns = int(found.get("KEYS_PER_ROW", "8"))
                send(f'ADD-DEVICE OK DEVICEID="{device}" ')
                send(f'BRIGHTNESS DEVICEID="{device}" VALUE=100 ')
                for key in range(total):
                    send(key_state(device, key, columns, False))
            elif cmd == "KEY-PRESS":
                key = int(found.get("KEY", "-1"))
                pressed = found.get("PRESSED") in ("1", "true")
                self.presses.append({"key": key, "pressed": pressed, "at": time.time() * 1000.0})
                if self.failing:
                    send(f'KEY-PRESS ERROR DEVICEID="{device}" MESSAGE="test refusal" ')
                else:
                    send(f'KEY-PRESS OK DEVICEID="{device}" ')
                    send(key_state(device, key, columns, pressed))
            elif cmd == "REMOVE-DEVICE":
                send(f'REMOVE-DEVICE OK DEVICEID="{device}" ')
            else:
                send(f'ERROR MESSAGE="Unknown command: {cmd}" ')
            await writer.drain()
```

- [ ] **Step 3: Wire the harness** — in `e2e/harness/harness.py`:

The docstring's usage gains `[--fake-companion-port N | --companion HOST:PORT]` and the route list gains:

```
    GET  /companion                      the fake Companion's state (#52,
                                         ``--fake-companion-port``): ``connections``,
                                         ``presses`` [{key, pressed, at}], ``down``,
                                         ``failing``; 404 without the fake
    POST /companion/down                 every connection closed, new ones closed
                                         at once (Companion away)
    POST /companion/up                   new connections served again
    POST /companion/fail {"on": <bool>}  presses answered ERROR, or OK again
    POST /companion/clear                the recorded presses forgotten
    POST /hub/companion {"on": <bool>}   the hub's config with or without
                                         ``[companion]``, and the hub restarted
```

and a paragraph: "The Stream Deck (#52): with ``--fake-companion-port`` the harness starts ``fake_companion.py`` there (0: any free port) and the hub's config gets ``[companion]`` on it; with ``--companion HOST:PORT`` it names a real Companion (the ``companion`` CI job) and starts no fake."

Imports: `from fake_companion import FakeCompanion` after `import impair`.

`hub_config` gains the parameter and, before the `remote = remote or {}` line:

```python
    if companion:
        host, port = companion
        text += f'[companion]\nhost = "{host}"\nport = {port}\n'
```

with its docstring "…, the Stream Deck's ``[companion]`` on ``companion`` (host, port) when given (#52), before the remote tables".

After `block_on`:

```python
def on_of(body, route):
    """The ``on`` of a body: true or false."""
    on = body.get("on")
    if not isinstance(on, bool):
        raise BadRequest(f'{route} wants {{"on": true|false}}, not {body!r}')
    return on


def companion_of(args, fake):
    """The hub's ``[companion]`` endpoint (#52): ``--companion HOST:PORT`` (a
    real Companion), else the fake's port on 127.0.0.1, else none."""
    if args.companion:
        host, _, port = args.companion.rpartition(":")
        if not host or not port.isdigit():
            raise SystemExit(f"--companion {args.companion!r}: HOST:PORT")
        return host, int(port)
    if fake is not None:
        return "127.0.0.1", fake.port
    return None
```

`Harness.__init__`: after `self.jwks = …`, keep `remote` in `self.remote = remote` (built as now) and replace the config write with:

```python
        self.http_port = args.http_port
        self.fake = None
        if args.fake_companion_port is not None:
            self.fake = FakeCompanion(args.fake_companion_port)
            self.fake.start()
        self.companion = companion_of(args, self.fake)
        self.write_config(self.companion)
```

(the `if args.public_name:` certificate copy stays before it), and add:

```python
    def write_config(self, companion):
        """The hub's config, with ``[companion]`` on ``companion`` or without."""
        with open(os.path.join(self.data, "fohmixer-hub.toml"), "w", encoding="utf-8") as f:
            f.write(
                hub_config(
                    self.http_port,
                    self.hosts["band"].port,
                    self.hosts["master"].port,
                    self.remote,
                    companion,
                )
            )
```

`handle`: after the `GET /link` line:

```python
        if method == "GET" and parts == ["companion"]:
            return (200, self.fake.state()) if self.fake else (404, {"error": "no fake Companion"})
```

and after the `/hub/layout/reset` block:

```python
        if parts == ["hub", "companion"] and self.hub is not None:
            on = on_of(body, "/hub/companion")
            self.write_config(self.companion if on else None)
            self.hub.restart(False)
            return 200, {"companion": on}
        if parts and parts[0] == "companion":
            if self.fake is None:
                return 404, {"error": "no fake Companion"}
            if parts == ["companion", "down"]:
                return 200, self.fake.down()
            if parts == ["companion", "up"]:
                return 200, self.fake.up()
            if parts == ["companion", "clear"]:
                return 200, self.fake.clear()
            if parts == ["companion", "fail"]:
                return 200, self.fake.fail(on_of(body, "/companion/fail"))
```

`stop`: after `self.link.stop()`: `if self.fake is not None: self.fake.stop()` (two lines).

`parse_args`: 

```python
    # The Stream Deck (#52): a fake Companion on this port (0: any), or a real one.
    parser.add_argument("--fake-companion-port", type=int, default=None)
    parser.add_argument("--companion", default=None)
```

In `.github/workflows/ci.yml`, the `e2e` job's harness command gains `--fake-companion-port 39192 \` (after `--link-port 8481`), and its comment: "the fake Companion (#52, 127.0.0.1:39192: the hub's Stream Deck tab)".

- [ ] **Step 4: Run the tests locally**

```bash
ruff format e2e/harness && ruff check e2e/harness
python3 -m unittest discover -s e2e/harness -p 'test_*.py' -v 2>&1 | tail -5
python3 scripts/check_integrity.py
```

Expected: all harness tests pass (they start real `sim/host.py` processes, no hub), no skips, integrity clean (no force-kill verb in the fake).

- [ ] **Step 5: Commit**

```bash
git add e2e/harness/fake_companion.py e2e/harness/test_fake_companion.py e2e/harness/harness.py \
  e2e/harness/test_harness.py .github/workflows/ci.yml
git commit -m "test(e2e): a fake Companion for the Stream Deck tab (#52)

The Satellite API as Companion 5.0.7 writes it, PNG keys, presses
recorded, faults on demand (away, refusing presses); the harness puts it
in the hub's [companion], drives it and can restart the hub without the
table.

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---
### Task 16: The Stream Deck in the browser, against the fake Companion

**Files:**
- Create: `e2e/tests/deck.spec.ts`
- Modify: `e2e/tests/support/live.ts` (`PointerStep` :267, new helpers after `impair` :41-46)
- Modify: `.claude/rules/e2e.md`

**Interfaces:**
- Consumes: the harness's `/companion*`, `/hub/companion`, `/hub/events`, `/forensics/timeline` routes (Task 15); the page's DOM (Task 12); the hub's `deck_press` records (Task 8) and the page's `deck` events (Task 11); the timeline's deck summary (Task 13).
- Produces: in `support/live.ts`: `companion` (`state`, `down`, `up`, `fail`, `clear`), `openDeck(page, keys = 32)`, `deckKey(page, key)`; `PointerStep` accepts `"pointercancel"`.

- [ ] **Step 1: The helpers** — in `e2e/tests/support/live.ts`, `PointerStep` becomes:

```ts
export type PointerStep = { type: "pointerdown" | "pointermove" | "pointerup" | "pointercancel"; dy?: number } | { wait: number };
```

and after `impair`:

```ts
/**
 * The harness's fake Companion (#52, `e2e/harness/fake_companion.py`): its
 * presses as it got them (`key`, `pressed`, `at` epoch ms), and its faults:
 * `down` (every connection closed, new ones closed at once), `up`, `fail`
 * (presses answered ERROR), `clear` (the presses forgotten).
 */
export const companion = {
  state: async (): Promise<any> => {
    const response = await fetch(`${HARNESS}/companion`);
    if (!response.ok) throw new Error(`harness /companion: ${response.status}`);
    return response.json();
  },
  down: () => harness("/companion/down"),
  up: () => harness("/companion/up"),
  fail: (on: boolean) => harness("/companion/fail", { on }),
  clear: () => harness("/companion/clear"),
};

/** A Stream Deck key of the page by its number (#52). */
export function deckKey(page: Page, key: number): Locator {
  return page.locator(`[data-testid="deck-key"][data-key="${key}"]`);
}

/** Opens the Stream Deck tab and waits for its `keys` keys, each with Companion's image (#52). */
export async function openDeck(page: Page, keys = 32) {
  const tab = page.getByTestId("deck-tab");
  await tab.click();
  await expect(tab).toHaveAttribute("data-selected", "true");
  await expect(page.getByTestId("deck-key")).toHaveCount(keys);
  await expect(page.locator('[data-testid="deck-key"] img')).toHaveCount(keys);
}
```

- [ ] **Step 2: Write the spec** — `e2e/tests/deck.spec.ts`:

```ts
import type { Locator, Page } from "@playwright/test";
import { expect, test } from "./support/fixtures";
import { companion, deckKey, dispatchPointer, harness, hubEvents, impair, openDeck, openSurface, pageEvents, until } from "./support/live";

// The Stream Deck tab (#52) against the harness's fake Companion
// (e2e/harness/fake_companion.py: 8 × 4 keys, each a small PNG; a press is
// answered OK and then the key's new state). The fake records every press it
// got, so a test reads what reached "Companion" and what never did.

/** The fake's presses of `key` since `since` (epoch ms): each one's `pressed`. */
async function pressesOf(key: number, since: number): Promise<boolean[]> {
  const state = await companion.state();
  return state.presses.filter((p: any) => p.key === key && p.at >= since).map((p: any) => p.pressed);
}

/** Watches `key` for its red flash (`data-failed` true for 400 ms): `flashed(page)` reads it. */
async function watchFlash(key: Locator) {
  await key.evaluate((el: Element) => {
    (window as any).deckFlash = false;
    new MutationObserver(() => {
      if (el.getAttribute("data-failed") === "true") (window as any).deckFlash = true;
    }).observe(el, { attributes: true, attributeFilter: ["data-failed"] });
  });
}

const flashed = (page: Page) => page.evaluate(() => (window as any).deckFlash === true);

/** A tap of `key`: down, 100 ms, up (dispatched pointer events of one finger). */
const tap = (page: Page, key: number) => dispatchPointer(deckKey(page, key), [{ type: "pointerdown" }, { wait: 100 }, { type: "pointerup" }]);

test.describe("The Stream Deck tab", () => {
  test.afterEach(async () => {
    await companion.up();
    await companion.fail(false);
  });

  test("shows 8 × 4 square keys in key order, and only the global controls on the rail", async ({ page }) => {
    await openSurface(page);
    await openDeck(page);
    const boxes = await page.getByTestId("deck-key").evaluateAll((els) =>
      els.map((e) => {
        const r = e.getBoundingClientRect();
        return { key: Number(e.getAttribute("data-key")), x: r.x, y: r.y, w: r.width, h: r.height };
      }),
    );
    expect(boxes.map((b) => b.key)).toEqual([...Array(32).keys()]);
    const viewport = page.viewportSize()!;
    for (const b of boxes) {
      expect(Math.abs(b.w - b.h), `key ${b.key} is square`).toBeLessThanOrEqual(1);
      expect(b.w, `key ${b.key} is large`).toBeGreaterThan(40);
      expect(b.x + b.w).toBeLessThanOrEqual(viewport.width);
      expect(b.y + b.h).toBeLessThanOrEqual(viewport.height);
    }
    for (let key = 1; key < 32; key++) {
      if (key % 8 > 0) {
        expect(boxes[key].x, `key ${key} right of ${key - 1}`).toBeGreaterThan(boxes[key - 1].x);
        expect(Math.abs(boxes[key].y - boxes[key - 1].y)).toBeLessThanOrEqual(1);
      }
      if (key >= 8) {
        expect(boxes[key].y, `key ${key} below ${key - 8}`).toBeGreaterThan(boxes[key - 8].y);
        expect(Math.abs(boxes[key].x - boxes[key - 8].x)).toBeLessThanOrEqual(1);
      }
    }
    await expect(page.locator('[data-testid="deck"] .rail-main > *')).toHaveCount(0);
    await expect(page.locator('[data-testid="deck"] .rail-foot > *').first()).toBeVisible();
    await expect(page.getByTestId("deck-tab")).toHaveAttribute("data-offline", "false");
    await expect(page.getByTestId("deck-dot")).toHaveCount(0);
    // The layout's tabs are not lit while the deck is shown, and the pages'
    // tab bar still lists only them.
    await expect(page.locator('[data-testid="tabbar"][data-level="0"] [data-testid="tab"][data-selected="true"]')).toHaveCount(0);
  });

  test("a key goes down at the touch and up at pointerup, pointercancel and on leaving the tab", async ({ page }) => {
    const since = Date.now();
    await openSurface(page);
    await openDeck(page);
    await tap(page, 3);
    await until(() => pressesOf(3, since), (p) => p.join() === "true,false", "key 3 down and up");
    await dispatchPointer(deckKey(page, 4), [{ type: "pointerdown" }, { wait: 150 }, { type: "pointercancel" }]);
    await until(() => pressesOf(4, since), (p) => p.join() === "true,false", "key 4 down, and up on a cancel");
    await dispatchPointer(deckKey(page, 5), [{ type: "pointerdown" }]);
    await until(() => pressesOf(5, since), (p) => p.join() === "true", "key 5 down");
    await expect(deckKey(page, 5)).toHaveAttribute("data-held", "true");
    await page.locator('[data-testid="tab"][data-page="foh"]').click();
    await expect(page.getByTestId("deck")).toHaveCount(0);
    await until(() => pressesOf(5, since), (p) => p.join() === "true,false", "key 5 up on leaving the tab");
    // The hub's records say why each up came; so does the page's flight recorder.
    const ups = await until(
      async () => (await hubEvents()).filter((r: any) => r.ev === "deck_press" && r.ts >= since && r.down === false),
      (r) => r.length >= 3,
      "the hub's deck_press records",
    );
    expect(new Map(ups.map((r: any) => [r.key, r.why]))).toEqual(new Map([[3, "up"], [4, "cancel"], [5, "tab"]]));
    expect(ups.every((r: any) => r.forwarded === true && r.hold_ms > 0)).toBe(true);
    const deckEvents = await until(
      async () => pageEvents(await hubEvents()).filter((e: any) => e.ev === "deck" && e.t >= since && e.d === 0),
      (e) => e.length >= 3,
      "the page's deck events",
      15000,
    );
    expect(new Map(deckEvents.map((e: any) => [e.k, e.why]))).toEqual(new Map([[3, "up"], [4, "cancel"], [5, "tab"]]));
    expect(deckEvents.every((e: any) => e.sent === true && typeof e.q === "number")).toBe(true);
    // The forensics timeline counts the window's three presses, numbers only.
    const report = await harness("/forensics/timeline", { from_ms: since, to_ms: Date.now() });
    expect(report.exit).toBe(0);
    const summary = new Map<string, string>(report.stdout.trim().split("\n").map((l: string) => l.split("=", 2) as [string, string]));
    expect([summary.get("deck_presses"), summary.get("deck_unsent"), summary.get("deck_forced_releases")]).toEqual(["3", "0", "0"]);
    expect(report.html).toContain('class="deck-press"');
  });

  test("a held key is outlined at once and shows Companion's pressed state", async ({ page }) => {
    await openSurface(page);
    await openDeck(page);
    const key = deckKey(page, 10);
    await dispatchPointer(key, [{ type: "pointerdown" }]);
    await expect(key).toHaveAttribute("data-held", "true");
    await expect(key).toHaveAttribute("data-pressed", "true");
    await dispatchPointer(key, [{ type: "pointerup" }]);
    await expect(key).toHaveAttribute("data-held", "false");
    await expect(key).toHaveAttribute("data-pressed", "false");
  });

  test("with Companion away the keys dim, the tab shows its dot, a press flashes red and is never sent", async ({ page }) => {
    await openSurface(page);
    await openDeck(page);
    await companion.down();
    await expect(page.getByTestId("deck-tab")).toHaveAttribute("data-offline", "true");
    await expect(page.getByTestId("deck-dot")).toBeVisible();
    await expect(deckKey(page, 6)).toHaveClass(/\boffline\b/);
    await expect(page.locator('[data-testid="deck-key"] img')).toHaveCount(32);
    await watchFlash(deckKey(page, 6));
    const since = Date.now();
    await tap(page, 6);
    await expect.poll(() => flashed(page)).toBe(true);
    await companion.up();
    await expect(page.getByTestId("deck-tab")).toHaveAttribute("data-offline", "false", { timeout: 8000 });
    await expect(deckKey(page, 6)).not.toHaveClass(/\boffline\b/);
    await page.waitForTimeout(1000);
    expect(await pressesOf(6, since)).toEqual([]);
  });

  test("Companion refusing a press flashes the key red", async ({ page }) => {
    await openSurface(page);
    await openDeck(page);
    await companion.fail(true);
    await watchFlash(deckKey(page, 9));
    await tap(page, 9);
    await expect.poll(() => flashed(page)).toBe(true);
  });

  test("a touchstart on a key or its image is prevented: no loupe, no callout", async ({ page }) => {
    await openSurface(page);
    await openDeck(page);
    for (const target of [deckKey(page, 11), deckKey(page, 11).locator("img")]) {
      const prevented = await target.evaluate((el: Element) => {
        const event = new Event("touchstart", { bubbles: true, cancelable: true });
        el.dispatchEvent(event);
        return event.defaultPrevented;
      });
      expect(prevented).toBe(true);
    }
    await expect(deckKey(page, 11).locator("img")).toHaveCSS("pointer-events", "none");
  });

  test("a reload opens the last mixer page, never the deck", async ({ page }) => {
    await openSurface(page);
    await openDeck(page);
    await page.reload();
    await expect(page.getByTestId("surface")).toHaveAttribute("data-connected", "true");
    await expect(page.getByTestId("page")).toBeVisible();
    await expect(page.getByTestId("deck")).toHaveCount(0);
    await expect(page.getByTestId("deck-tab")).toHaveAttribute("data-selected", "false");
  });

  test.describe("the page's own socket down", () => {
    // A reset of an open socket is a console error in WebKit only (e2e.md).
    test.use({ allowedConsole: [/^WebSocket connection to '.*' failed/] });

    test.afterEach(async () => {
      await impair.block(false);
    });

    test("a press while the page's socket is down flashes red and is never sent later", async ({ page }) => {
      await openSurface(page);
      await openDeck(page);
      await impair.block(true);
      await impair.drop();
      await expect(page.getByTestId("surface")).toHaveAttribute("data-connected", "false");
      await expect(page.getByTestId("deck-tab")).toHaveAttribute("data-offline", "true");
      await expect(page.locator('[data-testid="deck-key"] img')).toHaveCount(32);
      await watchFlash(deckKey(page, 7));
      const since = Date.now();
      await tap(page, 7);
      await expect.poll(() => flashed(page)).toBe(true);
      await impair.block(false);
      await expect(page.getByTestId("surface")).toHaveAttribute("data-connected", "true", { timeout: 8000 });
      await expect(page.getByTestId("deck-tab")).toHaveAttribute("data-offline", "false");
      // The open tab told the hub again after the hello: a key's new state
      // still reaches it.
      await tap(page, 12);
      await until(() => pressesOf(12, since), (p) => p.join() === "true,false", "key 12 after the reconnect");
      await page.waitForTimeout(1000);
      expect(await pressesOf(7, since)).toEqual([]);
    });
  });

  test("the tab exists only when the hub has a [companion] table", async ({ page }) => {
    try {
      await harness("/hub/companion", { on: false });
      await openSurface(page);
      await expect(page.locator('[data-testid="tabbar"][data-level="0"] [data-testid="tab"]').first()).toBeVisible();
      await expect(page.getByTestId("deck-tab")).toHaveCount(0);
    } finally {
      await harness("/hub/companion", { on: true });
    }
    await page.reload();
    await expect(page.getByTestId("surface")).toHaveAttribute("data-connected", "true", { timeout: 15000 });
    await expect(page.getByTestId("deck-tab")).toBeVisible();
  });
});
```

(Notes for the implementer, true for both projects: the dispatched `pointerdown`'s `set_pointer_capture` fails for its made-up pointer id and the page ignores the result, so nothing reaches the console; `companion.down()` closes the hub's connection, which reconnects every ≤ 2 s and is refused until `up()`; the hub restart of the last test closes every page socket, so it runs last and reloads.)

- [ ] **Step 3: Playbook** — in `.claude/rules/e2e.md`, add:

```markdown
- **The Stream Deck tab (#52, `deck.spec.ts`):** the `e2e` job's harness runs the fake Companion (`e2e/harness/fake_companion.py`, `--fake-companion-port 39192`) and puts it in the hub's `[companion]`, so every page has the deck tab (`data-testid="deck-tab"`, outside the `tab` test ids: the pages' bar specs still read the layout's tabs only). `companion` in `support/live.ts` reads the fake's presses (`key`, `pressed`, `at`) and sets its faults (`down`: Companion away, `fail`: presses refused; put back in `afterEach`); `openDeck` opens the tab and waits for 32 keys with images; `deckKey(page, n)`. A press is dispatched pointer events (`dispatchPointer`, `pointercancel` too); the red flash lasts 400 ms, so a test watches `data-failed` with a `MutationObserver` installed before the press. "Never sent later" is read at the fake after a second. `/hub/companion {on}` restarts the hub with or without the table (the last test; it reloads after). The real Companion 5.0.7 has its own job and config (`playwright.companion.config.ts`, `e2e/companion/`, `.claude/rules/ci.md`).
```

- [ ] **Step 4: Parse locally** — `cd e2e && npx playwright test --list` (lists the deck tests in both projects; no browser runs); `python3 scripts/check_integrity.py` (no `.skip`/`.only`).

- [ ] **Step 5: Commit**

```bash
git add e2e/tests/deck.spec.ts e2e/tests/support/live.ts .claude/rules/e2e.md
git commit -m "test(e2e): the Stream Deck tab in Chromium and WebKit (#52)

Against the fake Companion: the 8 x 4 grid of square keys, down and up on
pointerup, pointercancel and leaving the tab (the hub's records, the
page's flight recorder, the forensics timeline), the held outline and
Companion's pressed look, offline dimming with the red dot and the red
flash and no later send (Companion away, the page's own socket down), a
refused press's flash, the touch guard, no deck after a reload, and the
tab only with [companion].

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---
### Task 17: The real Companion 5.0.7 in CI

**Files:**
- Create: `e2e/harness/companion_config.py`, `e2e/harness/test_companion_config.py`
- Create: `e2e/companion/test.companionconfig` (generated by `companion_config.py`)
- Create: `e2e/companion/seed.mjs`, `e2e/companion/companion.spec.ts`, `e2e/playwright.companion.config.ts`
- Modify: `.github/workflows/ci.yml` (a new job `companion` after `e2e`), `.claude/rules/ci.md`

**Interfaces:**
- Consumes: the harness's `--companion HOST:PORT` (Task 15), `support/live.ts`'s `openSurface`, `openDeck`, `deckKey`, `dispatchPointer`, `until`, `HUB_SOCKET` (Task 16), the hub's Stream Deck (Tasks 1–12).
- Produces: the synthetic export (keys 0 "Light A", 1 "Scene 1", 2 "Hold C"; custom variables `light_a` off/on, `scene_1` idle/short/long, `hold_c` up/down); `companion_config.text()`; the CI job `companion`.

- [ ] **Step 1: Write the failing test** — `e2e/harness/test_companion_config.py`:

```python
"""The synthetic Companion export of the ``companion`` CI job (#52): the
committed ``e2e/companion/test.companionconfig`` is exactly what
``companion_config.py`` writes, and it holds only the three synthetic keys
and their custom variables (invented names, internal actions only)."""

import json
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import companion_config  # noqa: E402

E2E = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXPORT = os.path.join(E2E, "companion", "test.companionconfig")


class CompanionConfig(unittest.TestCase):
    def test_the_committed_export_is_the_builders(self):
        with open(EXPORT, encoding="utf-8") as f:
            self.assertEqual(
                f.read(),
                companion_config.text(),
                "run: python3 e2e/harness/companion_config.py > e2e/companion/test.companionconfig",
            )

    def test_it_holds_the_three_synthetic_keys_and_nothing_of_a_site(self):
        config = json.loads(companion_config.text())
        self.assertEqual((config["version"], config["type"]), (12, "full"))
        controls = config["pages"]["1"]["controls"]
        self.assertEqual(list(controls), ["0"])
        self.assertEqual(list(controls["0"]), ["0", "1", "2"])
        texts = [c["style"]["layers"][2]["text"]["value"] for c in controls["0"].values()]
        self.assertEqual(
            texts,
            ["Light A\n$(custom:light_a)", "Scene 1\n$(custom:scene_1)", "Hold C\n$(custom:hold_c)"],
        )
        self.assertEqual(sorted(config["custom_variables"]), ["hold_c", "light_a", "scene_1"])
        for empty in ("instances", "surfaces", "surfaceInstances", "surfacesRemote", "triggers", "imageLibrary"):
            self.assertFalse(config[empty], empty)
        actions = [
            action
            for control in controls["0"].values()
            for group in control["steps"]["0"]["action_sets"].values()
            for action in group
        ]
        self.assertEqual(len(actions), 5)
        self.assertEqual(
            {(a["connectionId"], a["definitionId"]) for a in actions},
            {("internal", "custom_variable_set_value")},
        )
        scene = controls["0"]["1"]["steps"]["0"]
        self.assertEqual(list(scene["action_sets"]), ["down", "up", "1000"])
        self.assertEqual(scene["options"]["runWhileHeld"], [], "the 1 s group runs on the release")


if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 2: The builder** — `e2e/harness/companion_config.py` (probed: its output imported into three fresh 5.0.7 containers through `seed.mjs`, and Light A toggled, Scene 1 gave short/long/short for 200/1500/900 ms holds, Hold C read `down` while held):

```python
"""The synthetic Companion 5.0.7 configuration of the real-Companion CI job
(#52): ``e2e/companion/test.companionconfig``, Companion's own export format
(``version`` 12, ``button-layered`` buttons, ``custom_variable_set_value``
actions), written by this file so the committed export is reviewable and
reproducible:

    python3 e2e/harness/companion_config.py > e2e/companion/test.companionconfig

Three keys on page 1, invented names only, internal actions only (no
connection, no module):

- key 0, **Light A**: a press toggles the custom variable ``light_a``
  (``off``/``on``); its text shows it and its background turns green while
  ``on``.
- key 1, **Scene 1**: a release after less than 1 s sets ``scene_1`` to
  ``short``; a release after a hold of 1 s or more runs the 1000 ms duration
  group instead and sets ``long``.
- key 2, **Hold C**: down sets ``hold_c`` to ``down``, up to ``up``; its
  background is orange while held.

The tests read Companion's state through its HTTP API
(``GET /api/custom-variable/<name>/value``), never through the hub.
``test_companion_config.py`` keeps the committed file equal to ``text()``.
"""

import json

# The build the file says it comes from (Companion writes it into exports).
BUILD = "5.0.7+9763-stable-cec2f88e6f"
# Colours as Companion stores them (0xRRGGBB): black, green, orange, white.
BLACK = 0
GREEN = 0x00AA00
ORANGE = 0xFFA500
WHITE = 0xFFFFFF


def prop(value, expression=False):
    """A layer property as Companion 5 stores it."""
    return {"value": value, "isExpression": expression}


def layers(text, colour, colour_is_expression=False):
    """A button's style: the canvas, a background box, a centred text."""
    return [
        {
            "id": "canvas",
            "name": "Canvas",
            "usage": "auto",
            "type": "canvas",
            "decoration": prop("default"),
            "showStatusIcons": prop("default"),
        },
        {
            "id": "box0",
            "name": "Background",
            "usage": "auto",
            "type": "box",
            "enabled": prop(True),
            "opacity": prop(100),
            "x": prop(0),
            "y": prop(0),
            "width": prop(100),
            "height": prop(100),
            "rotation": prop(0),
            "color": prop(colour, colour_is_expression),
            "borderWidth": prop(0),
            "borderColor": prop(0),
            "borderPosition": prop("inside"),
        },
        {
            "id": "text0",
            "name": "Text",
            "usage": "auto",
            "type": "text",
            "enabled": prop(True),
            "opacity": prop(100),
            "x": prop(0),
            "y": prop(0),
            "width": prop(100),
            "height": prop(100),
            "rotation": prop(0),
            "text": prop(text),
            "color": prop(WHITE),
            "halign": prop("center"),
            "valign": prop("center"),
            "fontsize": prop(100),
            "fontsizeAllowShrink": prop(True),
            "font": prop("companion-sans"),
            "outlineColor": prop(4278190080),
        },
    ]


def set_variable(action_id, name, value, expression=False):
    """Companion's internal action "Custom Variable: Set value"."""
    return {
        "id": action_id,
        "definitionId": "custom_variable_set_value",
        "connectionId": "internal",
        "options": {
            "name": prop(name),
            "create": prop(False),
            "value": prop(value, expression),
        },
        "type": "action",
        "children": {},
    }


def button(text, colour, colour_is_expression, action_sets):
    """A regular button of one step."""
    return {
        "type": "button-layered",
        "style": {"layers": layers(text, colour, colour_is_expression)},
        "options": {
            "stepProgression": "auto",
            "stepExpression": "",
            "rotaryActions": False,
            "canModifyStyleInApis": False,
            "notes": "",
        },
        "feedbacks": [],
        "steps": {"0": {"action_sets": action_sets, "options": {"runWhileHeld": []}}},
        "localVariables": [],
    }


def variable(description, default, order):
    """A custom variable's definition."""
    return {
        "description": description,
        "defaultValue": default,
        "persistCurrentValue": False,
        "sortOrder": order,
    }


def config():
    """The whole export."""
    light_a = button(
        "Light A\n$(custom:light_a)",
        f"$(custom:light_a) == 'on' ? {GREEN} : {BLACK}",
        True,
        {
            "down": [
                set_variable(
                    "lightA0toggle0000001",
                    "light_a",
                    "$(this:current) == 'on' ? 'off' : 'on'",
                    expression=True,
                )
            ],
            "up": [],
        },
    )
    scene_1 = button(
        "Scene 1\n$(custom:scene_1)",
        BLACK,
        False,
        {
            "down": [],
            "up": [set_variable("scene1short000000001", "scene_1", "short")],
            "1000": [set_variable("scene1long0000000001", "scene_1", "long")],
        },
    )
    hold_c = button(
        "Hold C\n$(custom:hold_c)",
        f"$(custom:hold_c) == 'down' ? {ORANGE} : {BLACK}",
        True,
        {
            "down": [set_variable("holdC0down0000000001", "hold_c", "down")],
            "up": [set_variable("holdC0up000000000001", "hold_c", "up")],
        },
    )
    return {
        "version": 12,
        "type": "full",
        "companionBuild": BUILD,
        "pages": {
            "1": {
                "id": "fohmixerTestPage0001",
                "name": "TEST",
                "controls": {"0": {"0": light_a, "1": scene_1, "2": hold_c}},
                "gridSize": {"minColumn": 0, "maxColumn": 7, "minRow": 0, "maxRow": 3},
            }
        },
        "triggers": {},
        "triggerCollections": [],
        "custom_variables": {
            "light_a": variable("Light A (synthetic test)", "off", 0),
            "scene_1": variable("Scene 1's last release (synthetic test)", "idle", 1),
            "hold_c": variable("Hold C (synthetic test)", "up", 2),
        },
        "customVariablesCollections": [],
        "expressionVariables": {},
        "expressionVariablesCollections": [],
        "instances": {},
        "connectionCollections": [],
        "surfaces": {},
        "surfaceGroups": {},
        "surfacesRemote": {},
        "surfaceInstances": {},
        "surfaceInstanceCollections": [],
        "imageLibrary": [],
        "imageLibraryCollections": [],
    }


def text():
    """The file's text: the export as JSON, one space of indent, a final newline."""
    return json.dumps(config(), indent=1) + "\n"


if __name__ == "__main__":
    print(text(), end="")
```

Generate the export: `python3 e2e/harness/companion_config.py > e2e/companion/test.companionconfig` (14 547 bytes of JSON; Companion takes plain JSON under the `.companionconfig` name).

- [ ] **Step 3: The seeding script** — `e2e/companion/seed.mjs` (probed on fresh containers, 4 of 4):

```js
// Seeds the real Companion of the `companion` CI job (#52) with the
// synthetic test configuration, through Companion 5.0.7's own Import /
// Export page (Companion's HTTP API cannot create actions):
//
//     cd e2e && COMPANION_URL=http://127.0.0.1:8000 node companion/seed.mjs companion/test.companionconfig
//
// The page uploads the file over its tRPC socket, so the file is set only
// once the socket is up (the sidebar's version comes over it). A fresh
// Companion shows its welcome wizard and What's New over the page, which
// leave the page behind them aria-hidden: CSS locators, and the visible
// closers clicked. "Import Preserving Unselected" keeps Companion's settings
// (the Satellite API stays on). Exit 0 once the test variables exist.
import { chromium } from "@playwright/test";

const base = process.env.COMPANION_URL ?? "http://127.0.0.1:8000";
const file = process.argv[2];
if (!file) {
  throw new Error("usage: node companion/seed.mjs <export file>");
}
const launch = process.env.CHROME ? { executablePath: process.env.CHROME } : {};
const browser = await chromium.launch(launch);
try {
  const page = await browser.newPage();
  await page.goto(`${base}/import-export`);
  await page.getByText("v5.0.7", { exact: true }).waitFor({ timeout: 60_000 });
  await page.locator('input[type=file][accept=".companionconfig,.yaml"]').setInputFiles(file);
  const importButton = page.locator("button", { hasText: "Import Preserving Unselected" });
  await importButton.waitFor({ timeout: 30_000 });
  const closers = page.locator('[aria-label="Close modal"]').filter({ visible: true });
  while ((await closers.count()) > 0) {
    await closers.first().click();
    await page.waitForTimeout(500);
  }
  await importButton.click();
  await importButton.waitFor({ state: "detached", timeout: 30_000 });
  const values = {};
  for (const name of ["light_a", "scene_1", "hold_c"]) {
    const response = await page.request.get(`${base}/api/custom-variable/${name}/value`);
    values[name] = (await response.text()).trim();
  }
  console.log(`seeded: ${JSON.stringify(values)}`);
  if (values.light_a !== "off" || values.scene_1 !== "idle" || values.hold_c !== "up") {
    throw new Error(`the import did not create the test variables: ${JSON.stringify(values)}`);
  }
} finally {
  await browser.close();
}
```

- [ ] **Step 4: The Playwright config and spec** — `e2e/playwright.companion.config.ts`:

```ts
import { defineConfig, devices } from "@playwright/test";

// The real Companion 5.0.7 job (#52, ci.yml `companion`): the Stream Deck tab
// against Companion's official container, seeded with
// e2e/companion/test.companionconfig. Its own test folder: the e2e job's
// config (./tests) never runs these, this one runs only these. Both projects,
// one worker, no retries, as the main config.
export default defineConfig({
  testDir: "./companion",
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: 0,
  workers: 1,
  reporter: [["html", { open: "never" }], ["list"]],
  use: {
    baseURL: process.env.E2E_BASE_URL || "http://127.0.0.1:8480",
    trace: "retain-on-failure",
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
    {
      name: "ipad",
      use: { ...devices["iPad Pro 11 landscape"], browserName: "webkit", hasTouch: true },
    },
  ],
});
```

`e2e/companion/companion.spec.ts`:

```ts
import { expect, test } from "../tests/support/fixtures";
import { HUB_SOCKET, deckKey, dispatchPointer, openDeck, openSurface, until } from "../tests/support/live";

// The Stream Deck tab against the real Companion 5.0.7 (#52, ci.yml
// `companion`): Companion's official container, seeded with the synthetic
// e2e/companion/test.companionconfig (key 0 "Light A" toggles light_a, key 1
// "Scene 1" sets scene_1 short on a release and long after a 1 s hold, key 2
// "Hold C" sets hold_c down and up), the hub registered on its Satellite
// API. Companion's own state is read through its HTTP API, never through
// the hub. Both projects run against one Companion, so each test asks for a
// change, never for a fixed starting value it could find spent.

const COMPANION = process.env.COMPANION_URL || "http://127.0.0.1:8000";

/** A custom variable of Companion, as Companion's HTTP API reads it. */
async function variable(name: string): Promise<string> {
  const response = await fetch(`${COMPANION}/api/custom-variable/${name}/value`);
  if (!response.ok) throw new Error(`Companion's ${name}: ${response.status}`);
  return (await response.text()).trim();
}

test.describe("The Stream Deck tab against Companion 5.0.7", () => {
  test("shows Companion's 32 keys, each with its webp image", async ({ page }) => {
    await openSurface(page);
    await openDeck(page);
    const images = await page.locator('[data-testid="deck-key"] img').evaluateAll((els) =>
      els.map((e) => {
        const img = e as HTMLImageElement;
        return { webp: img.src.startsWith("data:image/webp;base64,"), drawn: img.complete && img.naturalWidth > 0 };
      }),
    );
    expect(images).toHaveLength(32);
    expect(images.every((i) => i.webp)).toBe(true);
    await expect.poll(async () =>
      page.locator('[data-testid="deck-key"] img').evaluateAll((els) => els.every((e) => (e as HTMLImageElement).complete && (e as HTMLImageElement).naturalWidth > 0)),
    ).toBe(true);
  });

  test("a tap runs Light A's action and Companion's new image reaches the page", async ({ page }) => {
    await openSurface(page);
    await openDeck(page);
    const before = await variable("light_a");
    const image = deckKey(page, 0).locator("img");
    const src = await image.getAttribute("src");
    await dispatchPointer(deckKey(page, 0), [{ type: "pointerdown" }, { wait: 100 }, { type: "pointerup" }]);
    await until(() => variable("light_a"), (v) => v !== before, "Light A toggled in Companion");
    await expect(image).not.toHaveAttribute("src", src ?? "");
  });

  test("a 1.5 s hold runs Scene 1's duration action and a short tap does not", async ({ page }) => {
    await openSurface(page);
    await openDeck(page);
    await dispatchPointer(deckKey(page, 1), [{ type: "pointerdown" }, { wait: 100 }, { type: "pointerup" }]);
    await until(() => variable("scene_1"), (v) => v === "short", "the release action of a short tap");
    await dispatchPointer(deckKey(page, 1), [{ type: "pointerdown" }, { wait: 1500 }, { type: "pointerup" }]);
    await until(() => variable("scene_1"), (v) => v === "long", "the 1 s duration action");
  });

  test("closing the page's socket mid-hold leaves Hold C released in Companion", async ({ page }) => {
    const sockets: any[] = [];
    await page.routeWebSocket(HUB_SOCKET, (ws) => {
      ws.connectToServer();
      sockets.push(ws);
    });
    await openSurface(page);
    await openDeck(page);
    await dispatchPointer(deckKey(page, 2), [{ type: "pointerdown" }]);
    await until(() => variable("hold_c"), (v) => v === "down", "Hold C held in Companion");
    // The page's socket closes under the finger (Playwright closes the hub's
    // side with the same code): the hub releases the key itself.
    await sockets[sockets.length - 1].close({ code: 4000 });
    await until(() => variable("hold_c"), (v) => v === "up", "Hold C released by the hub", 5000);
    await expect(page.getByTestId("surface")).toHaveAttribute("data-connected", "true");
  });
});
```

- [ ] **Step 5: The CI job** — in `.github/workflows/ci.yml`, after the `e2e` job (the same action SHAs as `e2e`):

```yaml
  companion:
    name: companion
    # The real Companion (#52): the site's version, 5.0.7, from its official
    # container pinned by digest, seeded with the synthetic test
    # configuration through its own import page (e2e/companion/seed.mjs), the
    # hub registered on its Satellite API, and Playwright (Chromium and WebKit
    # iPad) on the Stream Deck tab (playwright.companion.config.ts).
    needs: [wasm]
    runs-on: ubuntu-24.04
    timeout-minutes: 30
    services:
      companion:
        image: ghcr.io/bitfocus/companion/companion:v5.0.7@sha256:8d98cc779cec5be2a77b7b36fb0da8d1a571d97b668e7ffa98e0f720393dbc35
        ports:
          - 8000:8000
          - 16622:16622
        options: >-
          --health-cmd "curl -fsS -o /dev/null http://localhost:8000/"
          --health-interval 2s
          --health-timeout 3s
          --health-retries 30
          --health-start-period 5s
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          persist-credentials: false
      - name: Rust toolchain (rust-toolchain.toml)
        run: rustup toolchain install
      - uses: actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c # v8.0.1
        with:
          name: wasm-dist
          path: crates/fohmixer-ui/dist
      - uses: Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6 # v2.9.2
      - name: Build the hub (release, the real UI embedded)
        run: cargo build --locked --release -p fohmixer-hub
      - uses: actions/setup-node@820762786026740c76f36085b0efc47a31fe5020 # v7.0.0
        with:
          node-version: "22"
          cache: npm
          cache-dependency-path: e2e/package-lock.json
      - uses: actions/cache@55cc8345863c7cc4c66a329aec7e433d2d1c52a9 # v6.1.0
        with:
          path: ~/.cache/ms-playwright
          key: ${{ runner.os }}-playwright-${{ hashFiles('e2e/package-lock.json') }}
      - name: Install Playwright (Chromium and WebKit)
        working-directory: e2e
        run: |
          set -euo pipefail
          npm ci
          npx playwright install --with-deps chromium webkit
      - name: Seed Companion with the synthetic test configuration (its own import page)
        working-directory: e2e
        env:
          COMPANION_URL: http://127.0.0.1:8000
        run: node companion/seed.mjs companion/test.companionconfig
      - name: Engineer PIN (random per run, masked)
        run: |
          set -euo pipefail
          export FOHMIXER_DATA="$RUNNER_TEMP/hub-data"
          mkdir -p "$FOHMIXER_DATA"
          pin="$(python3 -c 'import secrets; print(f"{secrets.randbelow(10000):04d}")')"
          echo "::add-mask::$pin"
          printf '%s\n' "$pin" | ./target/release/fohmixer-hub pin set-engineer
          echo "E2E_PIN=$pin" >> "$GITHUB_ENV"
      - name: Start the hub against Companion (e2e/harness/harness.py --companion)
        env:
          RUST_LOG: info
        run: |
          set -euo pipefail
          mkdir -p "$RUNNER_TEMP/e2e-logs"
          nohup python3 e2e/harness/harness.py --hub ./target/release/fohmixer-hub \
            --data "$RUNNER_TEMP/hub-data" \
            --layout tools/import-tosc/fixtures/expected-layout.json \
            --companion 127.0.0.1:16622 \
            --log-dir "$RUNNER_TEMP/e2e-logs" > "$RUNNER_TEMP/e2e-logs/harness.log" 2>&1 &
          for i in $(seq 1 60); do
            if curl -sf http://127.0.0.1:39190/health > /dev/null \
              && curl -sf http://127.0.0.1:8480/api/version > /dev/null; then
              echo "harness ready after ${i}s"
              exit 0
            fi
            sleep 1
          done
          cat "$RUNNER_TEMP/e2e-logs/harness.log"
          exit 1
      - name: The hub registered with Companion (/api/status companion, 32 keys)
        run: |
          set -euo pipefail
          token="$(curl -sf -X POST -H 'content-type: application/json' \
            -d "{\"pin\": \"$E2E_PIN\"}" http://127.0.0.1:8480/api/auth | jq -r .token)"
          for i in $(seq 1 30); do
            if curl -sf -H "authorization: Bearer $token" http://127.0.0.1:8480/api/status \
              | jq -e '.companion.online and .companion.keys == 32 and .companion.api_version == "1.12.0"' > /dev/null; then
              echo "registered after ${i}s"
              exit 0
            fi
            sleep 1
          done
          curl -sf -H "authorization: Bearer $token" http://127.0.0.1:8480/api/status | jq .companion
          exit 1
      - name: Core dumps off for the test browsers (#9)
        run: |
          set -euo pipefail
          echo "runner core_pattern: $(cat /proc/sys/kernel/core_pattern)"
          sudo sysctl -w kernel.core_pattern=core
      - name: Playwright against Companion (Chromium and WebKit iPad, clean console)
        working-directory: e2e
        env:
          CI: "true"
          E2E_BASE_URL: http://127.0.0.1:8480
          E2E_HARNESS_URL: http://127.0.0.1:39190
          COMPANION_URL: http://127.0.0.1:8000
        run: |
          set -euo pipefail
          ulimit -c 0
          pattern="$(cat /proc/sys/kernel/core_pattern)"
          if [ "${pattern#|}" != "$pattern" ]; then
            echo "::error::core_pattern pipes core dumps (${pattern})"
            exit 1
          fi
          npx playwright test --config playwright.companion.config.ts --reporter=list,html
      - name: Companion's log
        if: failure()
        run: |
          set -euo pipefail
          mkdir -p "$RUNNER_TEMP/e2e-logs"
          docker logs "${{ job.services.companion.id }}" > "$RUNNER_TEMP/e2e-logs/companion.log" 2>&1
      - name: Upload failure evidence
        if: failure()
        uses: actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a # v7.0.1
        with:
          name: companion-failure
          path: |
            e2e/test-results/
            e2e/playwright-report/
            ${{ runner.temp }}/e2e-logs/
          retention-days: 7
```

The workflow's header comment gains "…, Playwright E2E in Chromium and WebKit (iPad) with the console guard, the Stream Deck tab against the real Companion 5.0.7 (#52), …".

- [ ] **Step 6: Playbook** — in `.claude/rules/ci.md`, add:

```markdown
- **The real Companion job (`companion`, #52):** `ghcr.io/bitfocus/companion/companion:v5.0.7` pinned by digest as a service container (ports 8000 and 16622; its health check every 2 s, the image's own is every 30 s). `e2e/companion/seed.mjs` imports `e2e/companion/test.companionconfig` through Companion's Import / Export page (Chromium; it waits for the sidebar's `v5.0.7`, the tRPC socket the upload needs; closes the welcome and What's New modals with CSS locators: they make the page `aria-hidden`; "Import Preserving Unselected"), and fails unless the test variables exist. The export is `e2e/harness/companion_config.py`'s output (`test_companion_config.py` in the `python` job keeps them equal): regenerate it, never edit it by hand. The harness runs with `--companion 127.0.0.1:16622` (no fake), a step waits for `/api/status` `companion.online` with 32 keys, and `playwright.companion.config.ts` runs `e2e/companion/*.spec.ts` only, both projects. Companion's state is read with its HTTP API (`/api/custom-variable/<name>/value`). On failure `docker logs` of the service goes into `companion-failure`. A new Companion version is a deliberate change: bump the tag and digest together with the site's Companion.
```

- [ ] **Step 7: Local checks** — `python3 -m unittest discover -s e2e/harness -p 'test_*.py'` (the export test passes against the generated file); `ruff format --check e2e/harness`; `cd e2e && npx playwright test --config playwright.companion.config.ts --list` (4 tests × 2 projects); `actionlint .github/workflows/ci.yml` if installed; `python3 scripts/check_integrity.py` (pins and comments of every `uses:` in the new job).

- [ ] **Step 8: Commit**

```bash
git add e2e/harness/companion_config.py e2e/harness/test_companion_config.py e2e/companion \
  e2e/playwright.companion.config.ts .github/workflows/ci.yml .claude/rules/ci.md
git commit -m "test(ci): the Stream Deck tab against the real Companion 5.0.7 (#52)

Companion's official container, pinned by digest, seeded with a synthetic
export through its own import page; the hub registered on its Satellite
API; Playwright in Chromium and WebKit: 32 keys with images, a tap's new
image, a 1.5 s hold's duration action and a short tap's release action,
and a key released by the hub when the page's socket closes mid-hold.

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

- [ ] **Step 9: Checkpoint C — push** — the local checks of Checkpoints A and B plus `python3 -m unittest discover -s tools/forensics -p 'test_*.py'`, `python3 -m unittest discover -s e2e/harness -p 'test_*.py'`, `ruff format --check live-script sim tools e2e/harness scripts/cloudflare`, `cd e2e && npx playwright test --list`; the denylist scan; `git fetch origin && git merge origin/dev`; `git push origin dev`. Wait for both runs of the head. Expected green: everything of B plus `python` (forensics, harness, fake Companion, export tests), `windows` (`Test-Fohmixer.ps1` with the `[companion]` assertions; `companion_client` and `deck` on Windows), `e2e` (the whole suite with the fake Companion in the hub's config, `deck.spec.ts` in both projects), `companion` (the real Companion), `mutation`.

---
### Task 18: The PR, every gate green, the merge

**Files:**
- Create (outside the repo): `~/.claude/work-products/fohmixer/pr-52-body.md`

**Interfaces:**
- Consumes: Checkpoints A–C.
- Produces: the merged PR; the `master` run's `bundle` artifact `fohmixer-windows-0.1.0-dev.34-<sha>`.

- [ ] **Step 1: Open the PR (at Checkpoint A)** — write `~/.claude/work-products/fohmixer/pr-52-body.md`:

```markdown
Closes #52 (Stream Deck tab: Companion buttons inside fohmixer).

Design: `docs/superpowers/specs/2026-10-06-streamdeck-tab-design.md` (approved on #52); plan: `docs/superpowers/plans/2026-10-06-streamdeck-tab.md` (it records the Companion 5.0.7 probe and the spec gaps it settled).

- **Hub:** one more Stream Deck on Companion's Satellite API (`companion.rs`, `companion/client.rs`): BEGIN and the API 1.12 gate, ADD-DEVICE with a fresh device id under the serial `fohmixer`, a ping every 2 s, the link lost after 5 s of silence or a line over 256 KiB, presses written at once and answered first in first out, never queued, REMOVE-DEVICE on the stop.
- **Router:** the deck state (`deck.rs`, `deck/holders.rs`, `router/deck.rs`): the key cache, keys only to the pages viewing the tab and after their acks, one down and one up per key whatever the fingers and tablets, the releases only the hub can make (a closed page socket, a holding page silent for 2 s, the keys held when Companion was lost, which Companion 5.0.7 keeps held, and the stop), the event log's `deck_*` records; `/api/status` `companion`.
- **Page:** the "Stream Deck" tab (last, a red dot while Companion is unreachable, never restored by a reload), square keys with Companion's images, down at the touch and up at the release (pointerup, pointercancel, a lost capture, the tab left, the page hidden), a local outline at once, a red flash for a press that cannot go and no later send; the flight recorder's `deck` events.
- **Tools:** the forensics timeline's Stream Deck section; the installer's `-CompanionHost` (kept across updates).
- **Tests:** unit and integration tests against a scripted fake Companion (also on Windows), Playwright against the harness's fake Companion, and a new CI job against the real Companion 5.0.7 container with a synthetic configuration.

The live check on the PC opens the tab and counts the keys; it presses no key (they switch real lights and sockets).

🤖 Generated with [Claude Code](https://claude.com/claude-code)
```

then:

```bash
gh pr create --base master --head dev \
  --title "Stream Deck tab: Companion buttons inside fohmixer (#52)" \
  --body-file ~/.claude/work-products/fohmixer/pr-52-body.md
```

and note the PR number (`gh pr view --json number --jq .number`).

- [ ] **Step 2: Review before the last push** — after Checkpoint C is green, run a fresh-context review of the whole diff (`superpowers:requesting-code-review` with the spec, this plan and the diff `git diff origin/master...dev`); fix every finding in one commit (a bug with a red test first, per `regression-test-first`), push once, wait for both runs.

- [ ] **Step 3: Gates** — every job of both runs of the head green (`integrity`, `python`, `lint`, `test`, `windows`, `wasm`, `e2e`, `companion`, `supply-chain`, `secrets`, `version`, `mutants-list`, `mutation-warmup`, every `mutation` shard), then:

```bash
gh pr view <N> --json mergeable,mergeStateStatus --jq '.mergeable + " " + .mergeStateStatus'
gh api repos/{owner}/{repo}/pulls/<N> --jq .mergeable_state
```

Expected `MERGEABLE CLEAN` and `clean`. `unstable` or a red job is not mergeable: investigate, fix, push, wait again (never `--admin`, never a rerun of a real failure; one rerun only for the named WebKit crash of `.claude/rules/e2e.md`).

- [ ] **Step 4: Merge** — `gh pr merge <N> --merge` (a merge commit; GitHub authors it with the noreply identity).

- [ ] **Step 5: The master run** — wait for the `master` push run to end green, `bundle` included:

```bash
gh run list --branch master --workflow CI --limit 1 --json databaseId,headSha,status,conclusion
```

one foreground bounded poll on that run id until `completed`; `success` required; the artifact name `fohmixer-windows-0.1.0-dev.34-<first 7 of headSha>` (`gh run view <id> --json artifacts` or the run page).

---
### Task 19: Deploy on the Ableton PC and the live check (no key pressed)

**Files:**
- Create (outside the repo): `~/.claude/work-products/fohmixer/deploy-0.1.0-dev.34/` (the bundle, its SHA-256), `~/.claude/work-products/fohmixer/verify-deck.cjs`
- On the PC: `C:\temp\fohmixer-deploy\0.1.0-dev.34\` (download, unzip, `install.cmd`, `install.log`)

**Interfaces:**
- Consumes: the `bundle` artifact (Task 18), `.claude/rules/deploy-pc.md` (the procedure: serve the bundle over a short-lived LAN server, the elevated `install.cmd` over MCP, the minted engineer token), the MCP tools `mcp__win-<pc>__Shell`, `…__FileRead`, `…__FileWrite`.
- Produces: fohmixer 0.1.0-dev.34 on the PC with `[companion]`; the evidence comment on #52.

- [ ] **Step 1: The bundle** — `gh run download <master run id> -n fohmixer-windows-0.1.0-dev.34-<sha7> -D ~/.claude/work-products/fohmixer/deploy-0.1.0-dev.34`; `sha256sum` of the zip. Serve it as `.claude/rules/deploy-pc.md` says (`read ADDR PORT < <the saved serve-addr.txt>`; `setsid nohup timeout 900 python3 -m http.server "$PORT" --bind "$ADDR" --directory <a folder holding <random path>/bundle.zip> < /dev/null & disown`). On the PC (MCP Shell): create `C:\temp\fohmixer-deploy\0.1.0-dev.34`, `Invoke-WebRequest` the zip there, `Get-FileHash -Algorithm SHA256` equal to the dev box's; `Expand-Archive` into `unzipped`. Stop the server by its listener (`ss -ltnp`, then `kill <pid>`; never a `pkill -f` whose pattern matches the calling shell).

- [ ] **Step 2: `install.cmd` with `-CompanionHost`** — one MCP PowerShell call (helper names long, the whole call in `try { … } catch { "failed: $($_.Exception.GetType().Name)" }`, nothing site-specific printed):

```powershell
try {
  $previousDir = 'C:\temp\fohmixer-deploy\0.1.0-dev.33'
  $newDir = 'C:\temp\fohmixer-deploy\0.1.0-dev.34'
  $companionConnection = Get-NetTCPConnection -RemotePort 16622 -State Established -ErrorAction Stop | Select-Object -First 1
  $companionEndpoint = '{0}:16622' -f $companionConnection.RemoteAddress
  $oldZipName = (Get-ChildItem -LiteralPath $previousDir -Filter 'fohmixer-windows-*.zip' | Select-Object -First 1).Name
  $newZipName = (Get-ChildItem -LiteralPath $newDir -Filter 'fohmixer-windows-*.zip' | Select-Object -First 1).Name
  $installText = [IO.File]::ReadAllText((Join-Path $previousDir 'install.cmd'))
  $installText = $installText.Replace($oldZipName, $newZipName).Replace('0.1.0-dev.33', '0.1.0-dev.34')
  $installText = $installText.Replace('Install-Fohmixer.ps1 ', 'Install-Fohmixer.ps1 -CompanionHost ' + $companionEndpoint + ' ')
  [IO.File]::WriteAllText((Join-Path $newDir 'install.cmd'), $installText, [Text.Encoding]::ASCII)
  "ok: old versions left " + ([regex]::Matches($installText, '0\.1\.0-dev\.33').Count) + ", new zip named " + $installText.Contains($newZipName) + ", companion host set " + $installText.Contains('-CompanionHost ')
} catch { "failed: $($_.Exception.GetType().Name)" }
```

(Expected output `ok: old versions left 0, new zip named True, companion host set True`. Companion Satellite on the PC holds the established connection to Companion's port 16622; if none is found the call prints `failed: …`: then ask the owner for Companion's address through a `❓` question, never guess. The zip names hold no site data; the account names and paths of `install.cmd` are never printed.)

- [ ] **Step 3: The elevated install** — MCP Shell: `Start-Process cmd.exe -ArgumentList '/c "C:\temp\fohmixer-deploy\0.1.0-dev.34\install.cmd"' -Verb RunAs -Wait -PassThru | Select-Object ExitCode`; then read `install.log` (`Get-Content … | Select-String 'fohmixer install:|exit='` and the JSON's `"version"`, `"companion"` lines). Expected: `fohmixer install: the new hub accepts the config (…: OK)`, `fohmixer install: the hub answers version 0.1.0-dev.34`, `"companion": true`, `exit=0`.

- [ ] **Step 4: The hub reached Companion** — MCP Shell:

```powershell
try {
  $logLine = Select-String -Path 'C:\ProgramData\fohmixer\logs\hub.out.log' -Pattern 'Stream Deck registered with Companion' | Select-Object -Last 1
  $companionVersion = [regex]::Match($logLine.Line, 'companion=(\S+)').Groups[1].Value
  $apiVersion = [regex]::Match($logLine.Line, 'api=(\S+)').Groups[1].Value
  $dayFile = Get-ChildItem 'C:\ProgramData\fohmixer\logs\events-*.jsonl' | Sort-Object Name | Select-Object -Last 1
  $deckLinks = Get-Content $dayFile.FullName -Tail 20000 | Where-Object { $_.Contains('"ev":"deck_link"') } | ForEach-Object { $_ | ConvertFrom-Json }
  $lastLink = $deckLinks | Select-Object -Last 1
  "registered: companion=$companionVersion api=$apiVersion; last deck_link: state=$($lastLink.state) companion=$($lastLink.companion) api=$($lastLink.api)"
} catch { "failed: $($_.Exception.GetType().Name)" }
```

Expected `companion=5.0.7+9763-stable-cec2f88e6f api=1.12.0` and the last `deck_link` `state=up` with the same versions.

- [ ] **Step 5: `/api/status` and the tab in a browser (no key pressed)** — mint a short-lived engineer token on the PC and POST it to a one-shot receiver on the dev box (`.claude/rules/deploy-pc.md`, "No engineer PIN on the dev account"); read the public name from the PC's `fohmixer-hub.toml` `[tls] name` into a shell variable (never written anywhere). Then on the dev box:

```bash
curl -s -H "authorization: Bearer $(cat "$TOKEN_FILE")" "https://$FOH_NAME/api/status" \
  | jq '.companion | {online, companion_version, api_version, keys, connect_failures}'
```

Expected `online: true`, `api_version: "1.12.0"`, `keys: 32`, `connect_failures: 0`.

`~/.claude/work-products/fohmixer/verify-deck.cjs`:

```js
// The Stream Deck tab on the live hub (#52), opened through the public name
// with a minted engineer token. NO KEY IS PRESSED: the keys switch real
// lights and power sockets. Prints numbers only.
const { chromium } = require("/home/fohmixer/devel/fohmixer/e2e/node_modules/@playwright/test");
const fs = require("fs");

(async () => {
  const url = `https://${process.env.FOH_NAME}/`;
  const token = fs.readFileSync(process.env.TOKEN_FILE, "utf8").trim();
  const browser = await chromium.launch({ executablePath: "/usr/bin/google-chrome" });
  const context = await browser.newContext({ viewport: { width: 1194, height: 834 } });
  const page = await context.newPage();
  const problems = [];
  page.on("console", (m) => {
    if (m.type() === "error" || m.type() === "warning") problems.push(`${m.type()}: ${m.text()}`);
  });
  page.on("pageerror", (e) => problems.push(`pageerror: ${e}`));
  page.on("crash", () => problems.push("crash"));
  await context.addInitScript((t) => localStorage.setItem("fohmixer_token", t), token);
  await page.goto(url);
  await page.getByTestId("stage").waitFor({ timeout: 20000 });
  await page.waitForFunction(() => document.querySelector('[data-testid="surface"]')?.getAttribute("data-connected") === "true");
  const opened = Date.now();
  // The tab only: no key is touched.
  await page.getByTestId("deck-tab").click();
  await page.waitForFunction(() => document.querySelectorAll('[data-testid="deck-key"] img').length === 32, null, { timeout: 20000 });
  await page.waitForTimeout(1000);
  const keys = await page.locator('[data-testid="deck-key"]').evaluateAll((els) =>
    els.map((e) => {
      const img = e.querySelector("img");
      return { webp: !!img && img.src.startsWith("data:image/webp;base64,"), drawn: !!img && img.complete && img.naturalWidth > 0 };
    }),
  );
  const offline = await page.getByTestId("deck-tab").getAttribute("data-offline");
  await page.locator('[data-testid="tabbar"][data-level="0"] [data-testid="tab"]').first().click();
  await page.waitForTimeout(500);
  console.log(JSON.stringify({ opened, keys: keys.length, webp: keys.filter((k) => k.webp).length, drawn: keys.filter((k) => k.drawn).length, offline, problems }));
  await browser.close();
})();
```

Run: `FOH_NAME=… TOKEN_FILE=… node ~/.claude/work-products/fohmixer/verify-deck.cjs`. Expected `keys 32, webp 32, drawn 32, offline "false", problems []`. Delete the token file afterwards.

- [ ] **Step 6: Proof that nothing was pressed** — MCP Shell over the day file: count `"ev":"deck_press"` records with `ts` at or after the check's `opened` (UTC ms, from Step 5's output) — expected 0 — and the `deck_view` records of that window (expected one `on`, one `off`); print numbers only.

- [ ] **Step 7: The routine post-deploy checks** — as `.claude/rules/deploy-pc.md` lists them: the tray runs from `app\0.1.0-dev.34` and its log says `the hub answers version=0.1.0-dev.34`; `python3 scripts/check_lan_dns.py --name <public name> --resolver <LAN resolver>` prints `OK` (the values from the PC at run time); the tablets' `client report` `load` lines carry `build=0.1.0-dev.34` once they reconnect.

- [ ] **Step 8: Evidence on #52** — `gh issue comment 52 --body-file <file>` with (no address, host, path or name): the version and merge SHA deployed; `deck_link up` with Companion `5.0.7+9763-stable-cec2f88e6f`, API `1.12.0`; `/api/status` `companion` online, 32 keys, 0 failures; the browser check: 32 keys, 32 webp images drawn, the tab not offline, 0 console problems; 0 `deck_press` records in the check window (no key pressed); the owner's one-time step: in Companion's Surfaces tab, give the "fohmixer" surface its start page or put it in a group with the physical deck; the first real presses are the owner's, read back afterwards with `timeline.py` (the Stream Deck section).

---
### Task 20: The version bump after the merge

**Files:**
- Modify: `Cargo.toml` :6, `Cargo.lock` (the workspace crates), `live-script/FohMixer/version.py`

- [ ] **Step 1: Sync dev** — `git fetch origin && git checkout dev && git merge origin/master` (the merge commit of the PR comes back to `dev`).

- [ ] **Step 2: Bump** — `[workspace.package] version = "0.1.0-dev.35"` in `Cargo.toml`; `VERSION = "0.1.0-dev.35"` in `live-script/FohMixer/version.py`; `cargo update --workspace` (non-compiling: it rewrites the workspace crates' versions in `Cargo.lock`); `python3 scripts/check_version.py` (OK).

- [ ] **Step 3: Commit, then push in a separate call** (the fleet's pre-push hook reads the commits before the command runs: `.claude/rules/ci.md`)

```bash
git add Cargo.toml Cargo.lock live-script/FohMixer/version.py
git commit -m "chore: bump version to 0.1.0-dev.35 after the #52 merge [no-test: version bump only]

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

```bash
python3 scripts/denylist_scan.py --denylist ~/.local/share/fohmixer-private/denylist.txt \
  --identities scripts/allowed-identities.txt --boundary scripts/denylist-boundary.txt \
  --accepted scripts/denylist-accepted.txt --tree HEAD --commits HEAD && git push origin dev
```

Wait for the `dev` push run to end green.
