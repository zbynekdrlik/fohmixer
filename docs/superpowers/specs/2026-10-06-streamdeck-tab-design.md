# Stream Deck tab: Companion buttons inside fohmixer (#52)

Status: approach and all three design sections approved by the owner on #52 (2026-10-06). This note collects them into one buildable design. The implementation plan follows the owner's review of this note.

## 0. Zhrnutie pre vlastníka

**Záložka a tlačidlá**
- **Záložka „Stream Deck“** je posledná v hornej lište. Ťuk ukáže tlačidlá Stream Decku z Companionu (predvolene 8 × 4) s ich obrázkami, farbami a textom. Návrat je ťuk na inú záložku: mix je tam, kde si ho nechal.
- **fohmixer je v Companione ďalší Stream Deck** („fohmixer“). Hub sa ku Companionu pripája rovnako ako dnes Companion Satellite na Ableton PC. Všetko, čo zmeníš v Companione (obrázky, presunuté tlačidlá, stránky, spätná väzba), sa na tablete prejaví samo, bez novej verzie fohmixera.
- **Stlačenie platí od dotyku po pustenie**, takže dlhé stlačenie a akcie podľa dĺžky držania fungujú ako na fyzickom decku. iPad pri držaní neukáže lupu ani ponuku.

**Istoty**
- **Fadre majú vždy prednosť.** Obrázky tlačidiel dostane iba zariadenie, ktoré má otvorenú záložku Stream Deck. Kým si na mixe, cez spojenie neide nič navyše.
- **Každé „stlačené“ má svoje „pustené“.** Pošle ho tablet pri pustení, prerušení dotyku, odchode zo záložky alebo z aplikácie. Hub ho pošle za tablet, ak tabletu spojenie vypadne alebo sa tablet 2 s neozve. Ak počas držania vypadne spojenie hubu s Companionom, hub tlačidlo pustí hneď po jeho obnovení. Tlačidlo v Companione tak nikdy neostane „visieť“.
- **Nič sa neodošle neskoro.** Stlačenie, ktoré sa nedá doručiť hneď, sa zahodí a tlačidlo blikne načerveno. Stlačenie, ktoré je na ceste k hubu dlhšie ako 0,5 s (napríklad keď sa Wi-Fi na chvíľu zasekne), hub odmietne a tlačidlo blikne načerveno. Kým tablet nepočuje hub (červené počítadlo výpadkov), stlačenie ani neodíde. Svetlo ani zásuvka sa neprepnú prekvapivo o pár sekúnd.
- **Keď Companion nie je dostupný**, tlačidlá stmavnú a záložka má malú červenú bodku. Bez textu.

**Logy, testy, nasadenie**
- **Logy:** každé stlačenie a pustenie s dĺžkou držania a časom potvrdenia od Companionu, každý výpadok spojenia s Companionom a zmena tlačidla po stlačení. Nástroj na analýzu logov dostane časť pre Stream Deck.
- **Testy:** aj proti skutočnému Companionu 5.0.7 v CI (jeho oficiálny kontajner, rovnaká verzia ako v kostole).
- **Pri kontrole naživo sa nestlačí žiadne tlačidlo.** Ovládajú skutočné svetlá a zásuvky. Prvé ostré stlačenia urobíš ty a overia sa z logov.
- **Raz v Companione:** na karte Surfaces nastavíš Stream Decku „fohmixer“ úvodnú stránku, alebo ho spojíš s fyzickým deckom.

## 1. Request, findings, decision

**Request** (owner, #52, 2026-10-06): today the engineer walks to the physical Stream Deck, or opens Companion's web emulator in another browser window, to switch scenes, power sockets or lights. Wanted: a "Stream Deck" tab in fohmixer's top bar, from which the engineer does what they need and goes back. It is the first of several such integrations.

**Topology found on site** (no addresses in this repo):
- Companion 5.0.7 runs on a separate computer on the LAN. Its Satellite API reports ApiVersion 1.12.0, with `CAPS SUBSCRIPTIONS=0 NONSQUARE=1 BITMAP_FORMATS="rgb,png,webp"`.
- The Ableton PC runs Companion Satellite 1.7.5. That client connects a Stream Deck to Companion over the Satellite API (TCP 16622), and the hub can reach the same port.
- Companion's web UI is plain HTTP with absolute asset paths. fohmixer is HTTPS, so a browser blocks the HTTP page inside the app (mixed content).

**Approaches** (decided by the owner on #52: approach 1):

| | look | Companion admin exposed | presses logged | touch guard | open time | remote |
|---|---|---|---|---|---|---|
| 1 native Satellite surface | Companion's own images | no | yes | yes | instant | behind the PIN |
| 2 emulator through a hub proxy | identical | yes, through the hub | no | no | loads each time | yes |
| 3 link to the emulator | identical | no | no | no | loads each time, Safari sheet | LAN only |

**Protocol facts**, checked against Companion's own server (`companion/lib/Service/Satellite/SatelliteApi.ts`, API 1.14.0 on `main`; [docs](https://companion.free/for-developers/Satellite-API)):
- **Lines.** Each line is `CMD KEY=VALUE KEY="quoted value"`. Booleans are sent as `1`/`0`.
- **Replies.**
  - `ADD-DEVICE`, `KEY-PRESS` and `REMOVE-DEVICE` answer `<CMD> OK DEVICEID="…"` or `<CMD> ERROR DEVICEID="…" MESSAGE="…"`. Lines are handled in order, so OKs come back in the order sent.
  - `PING x` is answered `PONG x`. Companion 5.0.7 closes a Satellite socket after 5 s without any byte from the client (probed), so the surface must ping.
  - A key held when its surface goes away stays held in Companion: its release actions do not run. A release from a new surface (another `DEVICEID`) runs them. A release of a key Companion does not hold does nothing. A second down on a held key is ignored (probed on 5.0.7).
- **Devices.**
  - `ADD-DEVICE` for a `DEVICEID` still registered on another socket fails with "Device exists elsewhere".
  - Companion keys a surface's configuration on its `SERIAL`, which defaults to the `DEVICEID`.
  - `SERIAL`s starting `emulator:` or `group:` are reserved.
- **Limit.** A line over 2 MB closes the socket.

## 2. What the engineer sees (design section 1)

- **The tab.** "Stream Deck" is the last tab of the top bar, after the layout's pages. Its title comes from the hub configuration.
  - It exists only when the hub has a `[companion]` table (§7).
  - A small red dot on it means Companion is unreachable.
- **The page.** The top bar stays: tabs, SOLO ✕, the dropout counter, badges and version. The rail shows only the layout's global controls, as on the Conf page. The rest is the key grid.
- **The grid.**
  - `columns` × `rows` keys (default 8 × 4, Companion's standard page) in key order: row-major, key `n` = `row × columns + column`.
  - The keys are square and as large as the area allows: about 110 px on the FOH iPad.
- **A key** shows Companion's image (text and colours drawn in by Companion) and Companion's pressed look.
  - While a finger holds a key, the page also draws a thin local outline at once. The tablet has no tactile feedback, and Companion's pressed look needs a round trip.
- **Presses.** A key sends down at the touch and up at the release. Companion's long-press and duration actions therefore work.
- **The way back.**
  - Tapping another tab leaves the deck; the mixer page is where it was left.
  - The deck tab is never remembered: a reload opens the last mixer page, so the faders are at hand first.
- **One surface for all devices.** The hub registers one Companion surface, so every fohmixer device shows the same Companion page. The owner sets, once, its start page or a group with the physical deck in Companion's Surfaces tab.
- **Companion unreachable.**
  - The keys keep their last images, dimmed, and the tab gets the red dot.
  - A press then flashes the key red (`fail_flash`) and is never sent later.
  - The same holds while the page's own socket is down.

## 3. Architecture

**Structure and topology:**

```
iPad page ──WSS (protocol 2 + deck messages)──▶ hub router ──mpsc──▶ companion task ──TCP 16622──▶ Companion
   ▲                                              │  ▲                    │
   └──────── outbox (deck items last) ◀───────────┘  └──── events ◀───────┘
```

- **Hub: a new `companion` module**, shaped like `live/client.rs` and owning one task. It holds:
  - an unbounded request channel;
  - a writer task, so the session loop never waits on a socket write (the `hub-rust.md` rule from bug #9);
  - an `Arc<Mutex<Snapshot>>` for `/api/status`;
  - an events closure that becomes `RouterMsg::Deck { event }`.
- **The router owns all deck state:**
  - the online flag;
  - the key cache;
  - the viewers;
  - the holder set (`deck/holders.rs`, a pure module).

  It fans the state out through each client's `Outbox`, as it does for instances. No `broadcast`/`watch` channels: the hub uses none.
- **Page:**
  - a `DeckView` page component (`pages/deck.rs`);
  - a pure press state machine (`behave/deck.rs`);
  - store handling of the new messages (`store/live.rs` plus a pure part in `store/deck.rs`);
  - a recorder builder (`diag/trace.rs`).

**Framework and candidates** (`Architektúra:`):
- **Companion Satellite**, the official TypeScript client, run as a second process: rejected.
  - It is a desktop app for USB decks, and would add a second runtime and service to the PC.
  - The hub could not then guarantee releases on a page's disconnect, nor log presses on the fader timeline.
- **Rust Stream Deck crates** (`streamdeck`, `elgato-streamdeck`, `streamdeck-rs`, `streamdeck-plugin*`): rejected, wrong layer. They are HID drivers and Elgato-app plugin SDKs, not Companion's Satellite API. No crates.io crate implements the Satellite API (searched 2026-10-06).
- **Companion's WebSocket transport (16623)**: not used. It carries the same protocol. TCP is what Companion Satellite uses on site, and it needs no extra framing.
- **Chosen:**
  - `tokio::net::TcpStream` with `tokio::io::BufReader` line reading (the hub's `tokio` gains the `io-util` feature in its main dependencies);
  - a pure line parser and pure decisions, mutation-tested;
  - the existing `Backoff` (`live/mod.rs`).

## 4. Hub ⇄ Companion

- **Connect.**
  - TCP to `host:port` from `[companion]`, with a connect timeout of 3 s (as Live).
  - While not connected, requests are refused at once, never queued (as `refusing()` in `live/client.rs`).
- **Handshake.**
  1. Read `BEGIN`. Parse `ApiVersion`: major 1 and minor ≥ 12 is required (for `BITMAP_FORMAT`). Anything else is refused: logged as `deck_link refused`, then retried with backoff.
  2. `CAPS` is read and logged. The hub needs none of its flags.
  3. Send `ADD-DEVICE DEVICEID="fohmixer-<n>" SERIAL="fohmixer" PRODUCT_NAME="fohmixer" KEYS_TOTAL=<columns×rows> KEYS_PER_ROW=<columns> BITMAPS=<bitmap_px> BITMAP_FORMAT=webp COLORS=hex TEXT=0 BRIGHTNESS=0`.
     - `<n>` counts this hub's connection attempts (its own counter).
     - A fresh `DEVICEID` per connection avoids "Device exists elsewhere" while Companion still holds a dead socket.
     - The stable `SERIAL` keeps the owner's Companion settings for the surface.
  4. The link is up only after `ADD-DEVICE OK` arrives within 3 s. `ADD-DEVICE ERROR` or the timeout counts as a failed attempt, logged with Companion's message.
- **Liveness.**
  - `PING <counter>` every 2 s.
  - The link is lost after 5 s without any inbound line, on a read or write error, or on a line longer than 256 KiB. A webp key image at 144 px is a few KB; the cap bounds memory.
  - Reconnect uses `Backoff` (250 ms doubling to 2 s). The backoff is reset only after a session that reached `ADD-DEVICE OK`.
  - Only the first failure of an outage logs a warning (`first_of_outage`).
- **Inbound.**
  - `KEY-STATE DEVICEID=… KEY=<n> TYPE=… BITMAP=<data URL> COLOR=<hex> PRESSED=<0|1>` updates the cache entry of key `n`: `img`, `color`, `pressed` (nothing reads `TYPE`, so it is not cached). A missing field keeps its old value.
  - `KEYS-CLEAR` resets every entry to black and released.
  - `KEY-PRESS OK|ERROR` is matched to the oldest forwarded press, first in first out.
  - `PONG` only refreshes liveness. `PING x` is answered `PONG x`.
  - Any other command is ignored, counted per command name and logged once per name per session. That covers `BRIGHTNESS`, `LOCKED-STATE`, `VARIABLE-VALUE` and future additions.
- **Outbound presses.** `KEY-PRESS DEVICEID="fohmixer-<n>" KEY=<n> PRESSED=1|0`, written by the writer task at once. Each forwarded press records its send time, so the matched OK gives the Companion round trip.
- **Nothing late.** A down that took more than 0.5 s from the page to the hub (`DECK_LATE_MS`, §5) is never written to Companion: a press that waited in a stalled page link must not switch a light or a socket seconds later.
- **Disconnect.**
  - The cache is kept and marked offline.
  - Keys held when the link went down stay held in Companion (§1). Right after the next `ADD-DEVICE OK` the hub releases each of them (`deck_release reason=reconnect`) and clears the holder set. The release of a key Companion no longer holds does nothing. A finger still down at that moment loses its hold, and its later up is acknowledged as `not held`.
  - A release forwarded while the link was already dead (a page's up or the hub's own release, sent before the 5 s of silence end the link) is answered `offline` and may never have reached Companion. Its key joins the keys released after the next `ADD-DEVICE OK` (`deck_release reason=reconnect`).
  - Every press still waiting for an OK is answered to its page as `error: "offline"`.
- **Graceful stop.** Every held key is released first (`deck_release reason=stop`), because Companion would keep it held after the surface goes away. One double fault is accepted: if the hub stops while Companion is unreachable, the keys held at the link loss (and the releases it lost) cannot be released; each is logged as `deck_release reason=lost` (warn class). Such a key stays held in Companion until its next press, whose release then runs the key's release actions with the long hold. Then `REMOVE-DEVICE DEVICEID=…`, best effort with a 500 ms bound, then close.

## 5. Hub ⇄ page

**Protocol additions** (`fohmixer-proto/src/client.rs`, round-trip tests in `client/tests.rs`):

```rust
// ServerMsg
Deck { online: bool, columns: u32, rows: u32, title: String },
DeckKeys { items: Vec<DeckKey> },          // DeckKey { key: u32, img: Option<String>, color: Option<String>, pressed: bool }
DeckAck { seq: u64, ok: bool, error: Option<String>, rtt_ms: Option<f64> },
// ClientMsg
DeckView { on: bool },
DeckPress { key: u32, down: bool, seq: u64, t: f64, hold_ms: Option<f64>, why: Option<String> },
```

- `Deck` is sent on `Attach` and whenever `online` changes. Without a `[companion]` table the hub never sends `Deck`, and the page shows no tab. A `DeckPress` to such a hub is answered `ok: false, error: "no Stream Deck"`.
- **Viewers.** `DeckView { on }` adds the client to, or removes it from, the router's viewers.
  - `DeckKeys` items go only to viewers. On the mixer tabs, no deck bytes share the socket with fader `set`/`ack`.
  - A new viewer gets the whole cache at once.
  - The page resends `DeckView { on: true }` after every `Hello` while the tab is open, as it resends its subscriptions.
- **Outbox.**
  - `DeckKeys` items are coalesced to the latest per key, in a new `BTreeMap` in `Inner`, written last by `take()` (after `ack`). `is_empty()` accounts for them.
  - A `take()` writes at most 4 of them (`DECK_KEYS_PER_TAKE`), round the key numbers from the one after the last written; the rest go in the next batches, after the replies, acks and values queued meanwhile. A page of images (25–200 KB) then never holds a pong behind it on a slow link, where the page's dropout counter would turn red and its downs flash.
  - `Deck` and `DeckAck` are ordered replies.
- **The page's `seq`.** `DeckPress.seq` is its own counter, separate from `set`'s (one counter per kind of id). `t` is the page clock, as for `set`: the pointer event's own time, so a press a frozen page held back counts its wait too.
- **Late downs** (`deck::late`, the C1 review of #52). The hub computes each press's one-way delay as a `set`'s: hub ms − `t` − the socket's clock offset (§8). A down whose delay is over 0.5 s (`DECK_LATE_MS`; 500 ms is not late, 500.001 ms is) is not forwarded and not added to the holder set; it is recorded with `reason: "late"` and acknowledged `ok: false, error: "late"`, so the page flashes the key. An up is never refused for its delay: a release is always wanted, and its holder bookkeeping is unchanged (the up of a late down is `not held`). A down with no offset yet (no paired ping on a fresh socket; pings go every 100 ms) is forwarded. A clock step (the PC's time synced, or a tablet's clock paused over a sleep) starts the socket's offset over from the first exchange after it (`clock::step`: over 500 ms from the estimate beyond half of each round trip), so the gate never refuses on-time downs, or lets late ones through, for the minute the old estimate would otherwise last; the step is logged once (`clock_step_ms` on that `ping` record).
- **Holder set** (`deck/holders.rs`, pure): `key → set of client ids`.
  - **Down:** the client joins the key's set. If the set was empty, the press is forwarded; otherwise it is acknowledged `ok` without forwarding (Companion already holds the key).
  - **Up:** the client leaves the set. If the set is now empty, the release is forwarded; otherwise it is acknowledged `ok`.
    - An up from a client that does not hold the key is acknowledged `ok`, not forwarded, and logged as `not held`.
  - **Companion offline:** the press is not forwarded, not added to the set, and acknowledged `ok: false, error: "offline"`.
  - **A late down:** as offline, with `error: "late"` (above).
  - **`Detach`:** for every key the client holds, it leaves the set, and the release is forwarded when the set empties (`deck_release reason=detach`). This sits next to the setters' `drop_client`.
  - **Silence.** A client that holds a deck key and sends nothing for 2 s has every key it holds released as on `Detach` (`deck_release reason=silent`).
    - The page pings every 100 ms while visible, and #43's measurements found uplink gaps far below 2 s in use.
    - Without this, a tablet that drops off the Wi-Fi mid-hold leaves its key held in Companion until TCP gives up, minutes later.
    - A later up from that client is acknowledged `ok` as `not held`.
- **`DeckAck`.**
  - For a forwarded press it comes when Companion's OK or ERROR arrives, with `rtt_ms`.
  - For one acknowledged without forwarding it comes at once, with `rtt_ms: None`.

## 6. Page

- **Nav** gains `deck: RwSignal<bool>`, which is never stored.
  - Tapping the deck tab sets it.
  - Tapping a layout tab clears it and selects as today.
  - The body shows `DeckView` while it is set.
  - `fohmixer_pages` is untouched, so a reload opens the last mixer page.
- **`DeckView`** sends `DeckView { on: true }` on mount and `{ on: false }` on cleanup.
  - It lays the grid out with CSS grid: `--cols`/`--rows`, square keys sized from the measured area.
  - Each key is a `<button class="deck-key">` with `use:owns_touches=keys` (the integrity gate's rule) and pointer events only.
  - The key shows an `<img>` of the data URL, or Companion's colour when there is no image yet.
  - Classes: `pressed` (Companion's flag), `held` (local outline), `failed` (the red flash), `offline` (dim).
- **Press state machine** (`behave/deck.rs`, pure, native tests). The page holds `key → set of pointer ids`.
  - **`down(key, pointer, t, connected)`:**
    - `connected` is `can_press`: the socket open and past its hello, Companion online, and the link not dropping out (the dropout counter red: nothing from the hub for 300 ms). A down into a stalled link would wait there and arrive late.
    - The first pointer on a key gives `Send(down)` when connected, otherwise `Flash`.
    - Later pointers on the same key give `Nothing`.
  - **`up(key, pointer, t, why)`:** the last pointer leaving a key whose down was sent gives `Send(up, hold_ms)`. A key whose down was never sent gives `Nothing`.
  - **`leave_all(why)`** gives an up for every held key. It runs for `hidden` (`visibilitychange`), for `tab` (the DeckView cleanup), and when the socket closes. On a socket close the state is cleared and no up is sent: the hub's `Detach` releases them.
  - **The `why` of an up:** `up` (`pointerup`), `cancel` (`pointercancel`), `lost` (`lostpointercapture` without `pointerup`), `hidden`, `tab`.
  - **An error `DeckAck` for a down** flashes the key. Nothing is retried.
- **Nothing queued.** A deck press is never put in the intent store and never resent after a reconnect. This is deliberately unlike a fader (#43 L1).
- **The tab's red dot** is `!online`, from the latest `Deck`. On a socket close the store sets the deck offline and keeps the keys (dimmed) and the tab (with its red dot), as §2 describes.

## 7. Configuration and installer

`fohmixer-hub.toml` gains an optional table:

```toml
[companion]
host = "…"          # required; site data, written by the installer, never in this repo
port = 16622        # default
columns = 8         # 1..=16
rows = 4            # 1..=8
bitmap_px = 144     # 32..=288
title = "Stream Deck"  # 1..=24 characters
```

- The table follows the `Config` patterns: `#[serde(deny_unknown_fields)]` and default functions. Every field is written out in `Config::defaults`.
- `validate()` reports the bounds above. Its tests are a table of `(text, message)` rows.
- **Installer.**
  - `Install-Fohmixer.ps1` gains an optional `-CompanionHost <host[:port]>`. `New-FohHubToml` writes the `[companion]` table from it.
  - Without the parameter, an existing `[companion]` table is carried across the reinstall. Today the installer keeps only the text from `[tls]` to the end of the file.
  - `fohmixer-hub config check` then validates the result, as today.
  - `.claude/rules/deploy-pc.md` records the parameter.
- `/api/status` gains a `companion` block: online, `last_error`, `connect_failures`, Companion version, API version and key count. The block is read from the task's snapshot.

## 8. Audit trail

**Hub event log** (`events.rs`; pure `*_fields` builders with unit tests; documented in `.claude/rules/forensics.md` and `hub-rust.md`):

| `ev` | when | fields |
|---|---|---|
| `deck_link` (warn class) | up, down, refused | `state`, `companion`, `api`, `error`, `down_ms` (on up: how long it was down), `attempts` |
| `deck_press` (warn class) | every `DeckPress` | `client`, `key`, `down`, `seq`, `t`, `delay_ms`/`gap_ms`/`offset_ms` (as `set`), `hold_ms` (page), `hub_hold_ms` (between the forwarded down and up), `forwarded`, `reason` (`offline`, `late`, `held`, `not held`), `holders`, `peer`, `why` |
| `deck_ok` (warn class) | Companion's answer to a forwarded press | `client`, `seq`, `key`, `down`, `ok`, `error`, `rtt_ms` |
| `deck_release` (warn class) | a release the hub made itself | `client`, `key`, `reason` (`detach`, `silent`, `reconnect`, `stop`, `lost`) |
| `deck_key` | a key's state change, when its pressed flag changes; for 10 s after a press on that key; at most 1/s per key, with a change count, while a client views the tab | `key`, `pressed`, `color`, `img_hash`, `img_bytes`, `changes` |
| `deck_keys` | every 60 s while the link is up | change count per key since the last summary |
| `deck_view` (warn class) | a client opens or closes the tab | `client`, `on` |

Images are never logged; only their hash and size are. Blinking feedback therefore stays within the 256 MB daily cap. Past a day's cap only warn-class records are written (`events.rs` `is_warn`): the presses, Companion's answers and the tab's views are rare, tiny and the most consequential deck records, so they are warn class with the link changes and the hub's releases; `deck_key` and `deck_keys` stop at the cap like the pings and the writes.

**Page flight recorder** (`diag/trace.rs`):
- `deck` (essential rank, never dropped first): `k`, `d` (1 down / 0 up), `h` (hold, on up), `why`, `sent`, `q` (seq).
- `deck_view`: on/off.

**`tools/forensics/timeline.py`:** a deck section for the window:
- presses, with the hub delay, Companion's round trip and answer (ok, refused with its error, offline), and the hold as the page measured it vs as forwarded;
- red flashes (downs that did nothing: not sent by the page, refused by the hub offline or late, or answered not ok by Companion or the lost link);
- forced releases, each with Companion's answer (a `deck_ok` without a client, matched by key and order), so a release lost again shows;
- Companion's key changes (`deck_key`) within 10 s of a press of the key: whether Companion reacted;
- Companion link outages with their lengths.

Summary fields: `deck_presses`, `deck_unsent` (every red flash: page, hub and Companion's not-ok answers to downs), `deck_forced_releases`, `deck_link_outages`, `deck_rtt_p50/p99`. Stdout stays numbers-only: key indexes, no names.

## 9. Proof

**Hub, unit:**
- the line parser: `BEGIN`, `CAPS`, `KEY-STATE` with quoted, base64 and data-URL values, `OK`/`ERROR` with and without `MESSAGE`;
- the version gate at its boundaries (1.11.x refused, 1.12.0 accepted, 2.0.0 refused);
- the `ADD-DEVICE` and `KEY-PRESS` builders;
- the holder set; the FIFO OK matcher;
- the liveness decision at 5 s and the next millisecond;
- every `*_fields` builder; config validation.

**Hub, integration** (`tests/companion_client.rs`; a scripted fake Companion on a tokio `TcpListener`; host-free, so it also runs on Windows):
- handshake to up; `KEY-STATE` fan-out to viewers only;
- the press round trip with `DeckAck`;
- the ping timeout and reconnect with backoff;
- `ADD-DEVICE ERROR`; an old API refused;
- releases on `Detach`, and on a holding client's 2 s silence (not at 1999 ms); holds forgotten and pending presses answered `offline` on a disconnect;
- `REMOVE-DEVICE` on stop.

**Real Companion in CI** (a new job):
- `ghcr.io/bitfocus/companion/companion:v5.0.7`, the site's version, as a service container.
- It is seeded with a synthetic test configuration committed under `e2e/companion/`, which uses only Companion's internal actions:
  - a key that changes its own text and colour on press;
  - a key with a 1 s duration action on release;
  - a key whose look shows it is held.
- The hub runs against it, and Playwright (Chromium and WebKit iPad) checks:
  - the tab appears with 32 keys with images;
  - a tap changes the key's image on the page;
  - a 1.5 s hold runs the duration action and a short tap does not;
  - closing the page's socket mid-hold leaves the key released in Companion;
  - zero console errors.

  **Seeding the configuration.**
  - Companion's HTTP API can only restyle or press existing buttons; it cannot create actions. So the job's setup imports a committed synthetic export, `e2e/companion/test.companionconfig`, through Companion's own web UI import page. Playwright drives it, against the pinned v5.0.7.
  - The export is generated in Companion's export schema by `e2e/harness/companion_config.py`, and a test pins the committed file to the generator. A raw export would carry machine data. It holds only the synthetic buttons above; the test reads Companion's state through its HTTP API (custom variables).

**UI:**
- `behave/deck.rs` native tests: multi-pointer, every `why`, unsent downs, `leave_all`.
- Playwright against the fake Companion (`e2e/harness/fake_companion.py`, fault-injecting, beside `impair.py`):
  - the tab exists only with `[companion]`;
  - the grid's geometry;
  - down/up on `pointerup`, `pointercancel` and on leaving the tab;
  - offline dimming, the red dot and the red flash, with no later send;
  - a down into a 2 s stall of the page's link flashes and never reaches Companion (`late` at the hub), a down while the dropout counter is red flashes at once and is never sent, and the outline shows at once under a 300 ms stall;
  - the deck tab not restored after a reload;
  - a `touchstart` on a key is `defaultPrevented` (WebKit);
  - zero console errors.
- The integrity gate covers `use:owns_touches` on the keys.

**Live check on the PC** (after the installer run with `-CompanionHost`):
- The hub log shows `deck_link up` with Companion 5.0.7.
- `/api/status` shows the companion block online.
- Playwright through the public name, with a minted engineer token, opens the tab and finds 32 keys with images.
- **No key is pressed:** the keys switch real lights and power sockets. The owner makes the first real presses, and they are read back from the logs with `timeline.py`.

## 10. Delivery

One feature, one PR from `dev` to `master`, carrying:
- the hub module, router and outbox changes;
- the protocol;
- the page;
- the config and installer;
- the event log and recorder records, and the timeline section;
- the CI job with Companion, and the tests;
- the playbook updates (`hub-rust.md`, `ui-rust.md`, `forensics.md`, `deploy-pc.md`, `e2e.md`, `ci.md`).

## 11. Not in scope

- Configuring Companion from fohmixer: buttons, pages and actions stay in Companion's web UI.
- Stream Deck+ encoders and LCD strips (`KEY-ROTATE`, advanced `LAYOUT_MANIFEST`), Companion variables, the PIN lock (`PINCODE_LOCK`), brightness, and Satellite subscriptions.
- A separate Companion surface per device. The owner approved one shared surface; per-device surfaces would be a later change of `DEVICEID`/`SERIAL` only.
- Battery behaviour of the tab: #51 (battery-friendly idle mode). Images flow only while the tab is open.
