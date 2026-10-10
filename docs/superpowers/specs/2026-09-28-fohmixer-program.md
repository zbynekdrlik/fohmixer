# fohmixer: program spec

**Status:** DRAFT 2026-09-28. It is awaiting owner review on ticket #3. The architecture and D1–D5, D10 and D12 were agreed on #3 (the ROZHODNUTÉ comments of 2026-09-27/28).

**Terms**
- **Live**: Ableton Live 12 Suite.
- **Band Live**: the Live instance that runs the band set (stems, song automation, in-ear mixes, lyrics/visual/lighting MIDI).
- **Master Live**: the Live instance that runs the final PA mix (wireless hand mics, media, the band bus, the sub).
- **LOM**: the Live Object Model, Live's native Python API for remote scripts.
- **Ableton PC**: the Windows PC that runs both Live instances.
- **Engineer iPad**: the FOH engineer's tablet. Today it runs TouchOSC.
- **TouchOSC surface**: the engineer's TouchOSC Mk2 project (newest copy 2026-08-02) plus its Lua scripts from `zbynekdrlik/abl-touchosc`.
- **Reference script**: `leolabs/ableton-js` (MIT).
  - AbleSet runs v3.4.5 in the band Live on this PC: UDP, 10 ms timer.
  - The WebSocket/threaded transport is new upstream in v5 (2026-09-11) and does not yet run on this PC.

**Owner reading path:** §0, then §3.1 and §8. About 1,800 words.

---

## 0. Zhrnutie pre vlastníka

**Čo staviame:** vlastný dotykový mixér šitý na naše dva Abletony. Nahradí TouchOSC.

**Ako to funguje**
- V každom Abletone beží malý doplnok **FohMixer** (remote script). Sprístupní natívne Python API Abletonu (LOM) tak, ako je, bez vlastnej prekladovej vrstvy.
- Na Ableton PC beží jeden program v Ruste, **fohmixer-hub**. Pripojí sa k obom Abletonom, rozposiela dáta na ľubovoľný počet zariadení a servuje appku.
- Na iPade je webová appka pridaná na plochu, podobne ako iemmixer.

**Prvá verzia = dnešné TouchOSC, len rýchlo a pekne**
- Rozloženie aj nastavenia **naimportujeme priamo z TouchOSC projektu**. Nič sa nezadáva nanovo.
  - Rozloženie obsahuje len názvy stôp, na ktoré sa ovládače viažu, tak ako ich malo TouchOSC.
  - Preto zostane na Ableton PC a nebude vo verejnom repe. Hodnoty aj názvy sa čítajú z Abletonu za behu.
- Stránky a plochy ostanú rovnaké:
  - stránka pre muzikantov;
  - hlavná stránka FOH/WORSHIP s pod-stránkami STAGE, BAND B a OTHERS;
  - bočný panel, EFFECTS, MASTER A, HANDS a stripy vpravo hore;
  - Conf.
- Správanie ostane rovnaké:
  - fader (aj dvojklik na 0 dB), pan, mute s ochranou dvojklikom;
  - merače, sóla, STAGE mikrofóny a STAGE AUT;
  - TechAlert a refresh.
- **Po otvorení appky ukáže každý ovládač hneď skutočný stav z Abletonu**, skôr než ho dovolí meniť. To bola hlavná chyba TouchOSC.
- **Najprv jadro, potom bývalé MIDI prepínače** (D12):
  - Jadro je to, čo TouchOSC robilo cez OSC.
  - Bývalé MIDI prepínače (VOC MIC, REVERB, AUTOTUNE, ZVUKAR, REPRO, Podklady All, stránka muzikantov) prídu po jadre, už bez MIDI, priamo cez API.
  - Pôjdu len tie, ktorých mapovanie v sete je čisté. Polofunkčné vynecháme a vypíšeme.
- Presúvajú sa **všetky tablety, ktoré dnes používajú TouchOSC**, vrátane druhého tabletu na porte 12000, ak sa ešte používa.

**Prečo to bude rýchlejšie**
- Dnešné AbletonOSC berie príkazy len asi 10× za sekundu a najviac 20 naraz.
- FohMixer ich spracuje priebežne, približne každých 1–10 ms. AbleSet tu beží na 10 ms; presné číslo zmeria skúška K1.
- Odhad od dotyku po zmenu v Abletone: **dnes 60–130 ms, nové riešenie približne 20–40 ms.** Väčšinu tvorí jedna snímka obrazu na iPade a Wi-Fi.
- Krátke zaseknutia samotného Abletonu nevyrieši žiadny ovládač, ale mixér ich ukáže.

**Čo príde potom:** čokoľvek, čo API Abletonu dovolí, pridané na požiadanie ako nový ovládací prvok.
- Príklady: parameter pluginu, rack chain, grafický EQ, sendy.
- Veľký plugin môže vyžadovať jednorazové „Configure“ v Abletone, aby jeho parameter bol v API vidieť.
- Veci, ktoré API nedáva (napr. LUFS), potrebujú ďalší zdroj dát.

**Skúšobná prevádzka:** TouchOSC beží ďalej. FohMixer pôjde do voľného slotu v oboch Abletonoch a staré skripty odstránime až po tvojom súhlase.

**Merače:** ukazujú stĺpec z Abletonu. Číselný údaj merača v dBFS vypadne (D11): Ableton pre merač nedáva text a ten v TouchOSC aj tak zamŕzal pod −23 dB.

**Prístup zvonku (#17, D7):** mixér má jedno meno (`foh.<tvoja doména>`) pre kostolnú sieť aj internet.
- V kostole router posiela to meno priamo na Ableton PC; mixér ide cez HTTPS s certifikátom Let's Encrypt, ktorý si hub sám obnovuje.
- Mimo kostola (napr. iPad na mobilných dátach) ide to isté meno cez Cloudflare Tunnel; pustí len e-maily povolené v Cloudflare Access.
- Bez internetu aj bez Wi-Fi mixér ide otvoriť na Ableton PC (ikona na ploche) a na každom PC na kábli cez núdzovú adresu `http://<IP PC>:8480`.
- PIN zvukára ostáva všade.

---

## 1. Goal, scope, principles

**Goal.** Replace the TouchOSC surface with a tailor-made, fast touch mixer for both Live instances. Then grow it, prompt by prompt, into any control the LOM allows.

**v1 scope.** Parity with the TouchOSC surfaces in use today (§3). Same pages, strips and behaviours, faster and better looking.

**Non-goals for v1:** sends, device parameters, graphic EQ, LUFS, AbleSet, lyrics, Resolume, lighting, VB-Matrix routing, a third Live on another PC. All of them stay possible later (§4). (Internet access was one until the owner's #17, D7.)

**Principles**

- **P1 Thin layer over the native LOM.**
  - The Live script exposes the LOM 1:1: paths, properties, children, functions and listeners.
  - It uses the LOM's own names, the reference script's operation names and native values.
  - There is no curated or converted data model, no OSC and no binary framing.
- **P2 Live is the source of truth.**
  - Every displayed value comes from a Live listener, never from a local toggle memory.
  - Display strings come from Live (`str(DeviceParameter)`, `str_for_value`) wherever Live provides them.
- **P3 v1 parity first.**
  - v1 reproduces the TouchOSC surfaces. Their layout and settings are **imported from the TouchOSC project files**, never retyped.
  - Every deviation is recorded in §3.3 with a reason.
- **P4 Extensible by prompt.** A new control is a new UI component bound by the general binding form (§2.5). It needs no change to the Live script and no change to the hub protocol.
- **P5 Many Lives, many clients.** One hub serves N Live instances (v1: 2) to any number of simultaneous clients, and every client gets full feedback.
- **P6 Follow the reference script.**
  - Threading, lifecycle and dispatch follow `ableton-js`: a timer-drained queue on Live's main thread, as AbleSet runs here.
  - The WebSocket transport is vendored from upstream v5. It is the reference design, not something proven on this PC; K1 proves it here.
  - We diverge only where §2.2 says why.
- **P7 Never disturb Live.**
  - The Live script never blocks Live's main thread on I/O.
  - It never logs per message and binds only to localhost.
  - It coexists with AbleSet and with the AbletonOSC copies during the parallel run.
- **P8 MVP means limited scope, never lower quality.** Tests, CI, logging and the version display apply from the first commit.

---

## 2. Architecture

### 2.1 Processes

| Process | Where | Started by | Role | Restart effect |
|---|---|---|---|---|
| **FohMixer** remote script (Python, one copy per Live) | inside each Live, in a free control-surface slot | Live, when the set loads | LOM proxy on a localhost WebSocket | reloaded by Live on every set load; the hub reconnects and resubscribes |
| **fohmixer-hub** (Rust, one binary) | Ableton PC | scheduled task at logon (iemmixer pattern) | Live connections, fan-out, app, auth, the STAGE AUT rule; remote access (#17): HTTPS of the public name, its certificate, the Access check | clients reconnect and resync; Live is unaffected |
| **cloudflared** (the `fohmixer-tunnel` service, #17) | Ableton PC | Windows service, automatic | the Cloudflare Tunnel of the public name → the hub's plain HTTP port | internet clients reconnect; the LAN is unaffected |
| **fohmixer-ui** (Leptos/WASM PWA) | engineer iPad, other tablets, phones, PC | the user, from the Home Screen | touch surface | reload; state comes back from the hub |

Data flow: clients ⇄ (LAN, WebSocket) ⇄ hub ⇄ (127.0.0.1, WebSocket) ⇄ the FohMixer script in each Live. From the internet (#17): clients ⇄ Cloudflare (Access) ⇄ cloudflared on the PC ⇄ (127.0.0.1) hub.

### 2.2 FohMixer, the Live script

**Base.**
- It vendors the reference v5 transport (`Socket.py`, `WebSocket.py`) and the generic dispatch of `Namespaces/Interface.py`, with the MIT notice.
- The ~30 curated per-class namespace files are **replaced by one generic LOM layer**. The reference script returns children only through about 20 curated serializers and turns un-wrapped object properties into dead strings; that contradicts P1.

**Addressing.** Every command has a `target`:
- **LOM path** in the notation of the official LOM docs, from the roots `live_set` (the Song) and `live_app` (`Live.Application.get_application()`).
  - Example: `live_set tracks 3 mixer_device volume`.
  - Any list step may select by name instead of index: `live_set tracks[name=Klavir #] devices[name=EQ Eight] parameters[name=1 Gain A]`.
  - The script resolves the path at request time. A missing or ambiguous name is an error slot, never a guess.
- **Object id** `live_<ptr>`, returned for every object the script sends out. The script stores `(ptr, class name)` and checks the class on lookup, because pointers can be reused.

**Operations.** The reference names and dispatch are kept:
- `get_prop`, `set_prop`, `add_listener` and `remove_listener`.
- Any other `name` is called as a LOM function on the target, with positional or keyword `args` (e.g. `str_for_value`, `re_enable_automation`, `select_device`).
- `describe` returns the class plus its observable properties, properties and functions, from `dir()`. The reference calls these the introspection ops.

**Values out**
- Primitives are sent as-is.
- A Live object is sent as `{"$ref": "<id>", "path": "<LOM path>", "class": "<Live class>", "name": "<name, if it has one>"}`. A vector is sent as a list of those.
- An enum is sent as `{"$enum": "<member name>", "value": <int>}`.
- `add_listener` may ask for the display string. Each push then carries `{"value", "display": str(param)}`, so dB text comes from Live.

**Values in**
- A `{"$ref": id}` or `{"path": …}` in `set_prop` values or call args is resolved to the live object.
- A `{"$enum": name}` is converted to the enum type of the target: for `set_prop`, the type of the property's current value; for a call, a qualified name such as `Live.Clip.WarpMode.beats`.
- Without this rule, setters such as `selected_track` or `select_device(device)` would need curated wrappers (reference issue), which P1 forbids.

**Threading.**
- Socket accept, reads and writes run on Live's main thread too, non-blocking, in the same timer tick (#5: K1 showed the reference's daemon threads get Python only around that tick; the script runs no threads).
- All LOM access runs on Live's main thread, drained from `Live.Base.Timer(interval=T, repeat=True)`, with the ~100 ms `schedule_message` tick as a fallback.
- T is a tunable default: 10 ms, as AbleSet runs here. K1 sets the final value.

**Our changes to the reference script**, each with its reason:
1. **Non-blocking I/O.** The reference writes frames of 64 KB or less with a blocking `sendall` on the calling thread (Live's main thread, 3 s socket timeout), and its per-connection queue is unbounded.
   - In FohMixer, the main thread only updates a per-connection map of the latest value per key. Its size is bounded by the number of subscribed keys.
   - The tick writes that map to the socket without waiting, keeping what the socket does not take for the next tick (#5; first a sender thread).
   - The result queue is bounded; on overflow the connection is dropped and the hub resyncs.
2. **Listener coalescing.** A listener callback only marks `(object, property)` dirty; it does no LOM writes, because Live forbids changes from inside notifications. Once per timer call, the latest value of every dirty key goes out as one batched frame. The reference sends one frame per callback; on this PC meters fire at about 30 Hz per track.
3. **Meter rate cap.** `output_meter_left/right/level` are flushed at most every 33 ms (tunable).
4. **Work budget.** The drain stops at 5 ms per timer call and never uses more than 5 ms in any 20 ms window (tunable), checked between commands. A single LOM call cannot be interrupted; K1 logs the longest one.
5. `jsonReplace` calls `list()` only on Live vector types (reference issue #115).
6. The listener key includes the class (reference flaw: singleton keys collide).
7. On Windows, sockets use `SO_EXCLUSIVEADDRUSE` instead of `SO_REUSEADDR`. On Windows, `SO_REUSEADDR` lets a stale listener share the port without an error.

**Lifecycle** (as the reference script)
- A set load calls `disconnect()`, which broadcasts `disconnect`, clears ids and listeners and closes the socket. The new instance binds the same port again.
- The script sends `connect` with `{instance, set_name, script_version, live_version}` on each connection.

**Health**
- Every timer call writes a `last_main_tick` timestamp.
- The main thread makes `heartbeat{main_tick_age_ms}` in its tick every 100 ms (#5; first a background thread, which K1 showed ran only around the tick anyway). During a stall none goes out, so the hub shows the "Live busy" badge **during** the stall once a heartbeat is 300 ms overdue; the first heartbeat after the stall reports its length.

**Logging.** Rotating file at WARNING level, never per message. `connect`, `disconnect`, errors and stalls above 200 ms are logged.

**Per-instance config**
- Each Windows user's copy has its own `Config.py`: `INSTANCE` (`band` or `master`) and `PORT`.
  - Defaults: 39101 band, 39102 master.
  - Never 39031, the reference default that AbleSet may adopt.
- The script binds **127.0.0.1 only**.
- The folder is named `FohMixer`, so it never collides with `AbleSet` or the AbletonOSC folders in Live's shared Python interpreter (relative imports, own logger name).

**Size.** About 1,200 LoC with the vendored transport.

### 2.3 Wire protocol (hub ⇄ script)

This is the reference v5 envelope and dispatch; only `ns`/`nsid` is replaced by `target`:

- **Request:** `{"uuid","commands":[{"target","name","args"}]}`. There is one result slot per command; a failure does not abort the others.
- **Result:** `{"event":"result","uuid","data":[{"ok":true,"data"}|{"ok":false,"error","errorType"}],"ts"}`.
- **Push:**
  - `{"event":"values","data":[{"key","value","display?"}],"ts"}`, one frame per flush;
  - `connect`, `disconnect`, `heartbeat`.

Everything is JSON text over WebSocket on localhost, with no compression, no chunking and no auth. The socket never leaves 127.0.0.1.

### 2.4 fohmixer-hub

- **Instances.**
  - A config lists the Live instances by name and port.
  - The hub keeps one reconnecting client per instance: backoff 250 ms, doubling to 2 s.
  - It declares an instance "busy" when `main_tick_age_ms` > 150 or the heartbeat is overdue.
- **Pass-through.** Client commands carry `instance` plus the §2.3 command. The hub forwards them unchanged and routes the results back.
- **Subscription dedupe.**
  - Each `(instance, target, property)` gets one Live listener, reference-counted across clients.
  - The latest value is cached and sent at once to a new subscriber.
- **Fan-out.** Values go to subscribed clients. Per client, only the latest value per key is kept while a send is pending, so there is no unbounded queue.
- **Resync.** On a Live `connect`, every subscription is re-resolved and fresh values are pushed. On `disconnect`, the instance shows offline.
- **Auth.** The engineer PIN and JWT come from iemmixer (argon2id, DPAPI pepper, login guard), on every path.
- **Remote access (#17, D7).** One public name for the LAN and the internet (split-horizon DNS: the church router's static record → the PC, a CNAME to a local-only name so AAAA and HTTPS never go upstream, checked by `scripts/check_lan_dns.py` after every deploy (#26); public DNS → a Cloudflare Tunnel).
  - An HTTPS listener serves the name (`[tls]`, port 443) with a Let's Encrypt certificate the hub gets and renews itself (`[acme]`: DNS-01 through the Cloudflare DNS API, renewed under 30 days left, retried 1 min doubling to 6 h (every 5 min while no Cloudflare token is set), the old certificate served meanwhile; the Cloudflare API token is set with `fohmixer-hub cloudflare set-token` and sealed with DPAPI).
  - The plain-HTTP listener (8480) always stays: the emergency path by IP (no DNS, no certificate) and the tunnel's origin; a taken HTTPS port never stops it (the HTTPS bind is retried every minute and reported). Only a request for the public name is redirected to HTTPS, and only while HTTPS serves; one by IP or through the tunnel never is.
  - A request that came through a proxy (any forwarded header: the tunnel) or from a public address is an internet request and needs a valid Cloudflare Access JWT (RS256 against the team's keys, `exp`/`aud`/`iss` required; `[access]`). Without `[access]` every internet request is refused. LAN requests need no Access and the LAN path does no network I/O, so the mixer works with the internet down. A browser's cross-site POST or WebSocket upgrade is refused on every path (Origin guard), a foreign `Host` too (DNS rebinding).
  - The Ableton PC resolves the name itself (a hosts entry to 127.0.0.1) and has a desktop shortcut to it, so it works with the router down. An install without the public name keeps the remote access an earlier one set up.
  - `/api/status` reports the HTTPS listener, the certificate (names, days left, the last ACME error) and cloudflared's ready connections (`[tunnel]`).
- **Static app** (rust-embed), `/api/version` and `/api/client-error`, as in iemmixer.
- **Client reports (#26).** Every page reports its load, its hub socket, its visibility, its service worker and wake lock, its errors and (`perf`, #5 K4) its frame rate, longest frame and the most fingers at once to `POST /api/client-report` (public, 10 KiB, the known fields only, each cut and stripped of control characters, 60 per peer per 10 s). The hub logs each at INFO as `client report` with the peer (through the tunnel also the client address Cloudflare names) and `lan`/`internet`, and keeps the newest 50 in `/api/status` `client_reports`. A hub whose hello names another build than the page's reloads the page (at most once a minute), so a deploy reaches an open Home Screen app by itself.
- **Layout.** The hub serves the layout document (§2.5) and validates it on load. On a validation failure it keeps serving the last good version and reports why.
- **One rule: STAGE AUT (F15).** This is the single named exception to "the hub does not interpret the LOM".
  - The STAGE AUT flag is a persisted hub value, shared by all clients.
  - While it is on, the hub sets the stage-mic mute on every `is_playing` change: playing → muted, stopped → live.
  - It must run once and keep running while an iPad sleeps, so it cannot live in the clients.

Apart from that rule, the hub has no mixer model and does no dB maths.

**Client protocol**
- The same `target`/`name`/`args` plus `instance`.
- A `Hello{proto, build}` handshake and a bounded reload on a protocol mismatch, from iemmixer.
- The types live in a small WASM-safe crate, `fohmixer-proto`, shared by the hub and the UI.

### 2.5 fohmixer-ui

**Stack.** Leptos 0.7 (CSR) built with Trunk, from iemmixer.

**Input layer.** This part is new; iemmixer's fader is single-touch.
- Pointer Events per `pointerId` with `setPointerCapture`, and `touch-action: none`, so several faders move at once.
- No activation hold and no send throttle; sends are coalesced to one per animation frame.
- Device prerequisite: iPadOS multitasking gestures are off. They take over 4- and 5-finger gestures before the page sees them.

**Rendering.** One `requestAnimationFrame` loop draws faders and meters. Animations are time-based, so 60 Hz and 120 Hz behave the same.

**Components** are generic: `Strip`, `Fader`, `Pan`, `Mute`, `Meter`, `DbLabel`, `SoloButton`, `ParamToggle`, `ParamFader`, `AlertOverlay` (`RefreshButton` removed, #58). §3.1 gives their v1 behaviour. **The look is our own (D13, #21):** `docs/superpowers/specs/2026-09-28-ui-redesign-design.md` §4 (rows of sections, the strip with its dB scale and meter, the name button in the track's Live colour); since D15 (#63) the top bar and the rail live in one control column in the middle of the screen.

**General binding form**, for every component: `instance` + `anchor` + `path`.
- **anchor**: a track or return by exact name, `master`, or `song`.
- **path**: relative to the anchor, with name selectors allowed.

Examples:
- a strip: `band`, `tracks[name=Klavir #]`, empty path;
- a plugin parameter: `band`, `tracks[name=Klavir #]`, `devices[name=EQ Eight] parameters[name=1 Gain A]`.

How it resolves:
- The UI observes the anchor lists (`tracks`, `return_tracks`) and the names along the path, and re-resolves on any change.
- An unresolved or ambiguous binding disables the component and shows it red (I5).
- A strip is just this form with an empty path, so the same code serves v1 and every later control.

**Layout document** (schema 2 since #21; the redesign note §2)
- It describes pages, each with a rail of function controls and rows of sections; a section is a group of controls or the page's one nested pager, whose sub-pages hold groups; global controls (TechAlert; REFRESH ALL until #58) show on every page. A strip may be `pinned`: it never leaves the screen (D15, #63).
- Nothing is placed: no canvas, frames or z. The UI computes the placement (one strip width shared by all rows).
- It lives on the Ableton PC in the hub's data folder (§5.2).
- **Editing it later** (D4): Claude edits the file on the PC through the MCP tools. The hub validates it on reload: schema, known components, and a report of unresolved names. A failed validation keeps the last good layout live, so a bad edit never blanks the surface. Every accepted change is kept as a dated backup.

**Robustness**
- WebSocket reconnect under 2 s that never gives up.
- Per-instance connection badges and the "Live busy" badge.
- The version is shown on screen.

### 2.6 TouchOSC import tool

This is a **one-shot tool**, `tools/import-tosc`, outside the hub. It is re-run during the parallel run if the engineer changes TouchOSC, and it is deleted at S6 together with its fixture.

- It inflates the `.tosc` (zlib XML) and composes parent-relative frames into canvas coordinates. Pages sit at (0,59); sub-pages sit inside their pager.
- **It classifies by role, not by script alone.** A node is live only when it has the expected name under a strip group (`fader`, `pan`, `mute`, `meter`, `db`, `status`, …).
  - Nodes that carry scripts but do nothing are decoration, redrawn natively or dropped: backdrop buttons carrying the mute script, the second meter bar, tick TEXT lines, the dead Podklady label.
  - The script hash table is built from the scripts inside the `.tosc` itself, with trailing newlines normalised.
- **It emits** (layout schema 2 since #21; it groups what it used to place, the redesign note §3):
  - pages, the rail of each page with strips, rows of groups (containment by an area's frame, the title beside or above it, the area's fill as the group colour) and the nested pager as a section;
  - the global controls (the TechAlert strip and alert box as one `alert`; REFRESH ALL is dropped and reported, #58);
  - strips of kind `standard` or `return`, `wide` for the wider bus strips;
  - solo buttons, the stage-mic group and STAGE AUT;
  - MIDI controls, with type, channel, number, value/velocity scaling, press and release trigger flags, button type, send/receive flags, and fader response mode and grid;
  - the `Conf` text (`double_click_mute`; `unfold_*` is dropped and reported since #58).
- **It reports and skips** invisible and off-canvas nodes. It also reports stale config: a `double_click_mute` entry that matches no strip, and a binding whose track lost its `#`. It never fixes them silently.
- **For D10**, it also reads the band set's MIDI mappings (`KeyMidi` with their ranges), writes each MIDI control's target parameters into the layout, and triages each control as clean or dropped (§3.2). Nothing is retyped.

Its tests check the layout document it produces, on a synthetic `.tosc` fixture.

### 2.7 Invariants

- **I1** The Live script never blocks Live's main thread on I/O. Its work budget (§2.2 change 4) is checked between commands.
- **I2** No per-message logging inside Live.
- **I3** The Live script binds 127.0.0.1 only.
- **I4** A displayed value is the last value Live reported. A control the user is touching shows the finger's value until release; it then snaps to Live's value, after the TouchOSC delay when X3's switch is on.
- **I5** An unresolved or ambiguous binding disables the control. It never guesses.
- **I6** The hub never retries a set on its own. A failed slot is shown and logged.
- **I7** Nothing is force-killed on the Ableton PC. fohmixer never restarts Live.
- **I8** On app start, reconnect, Live reconnect and set load, a control accepts input only after it shows Live's current value. Until then it is visibly disabled.
- **I9** A marker problem is never dropped silently: the strip shows, marked (both strips of an equal label disabled), and the problem is listed in `/api/status`.
- **I10** An unchanged composition never bumps the layout revision: the tablets refetch only when the frame or a marker changed what they show.
- **I11** fohmixer only reads a marker's name: it never touches a Tuner, and a Tuner never alters the signal.

---

## 3. v1 parity with the TouchOSC surfaces

### 3.1 Features

Source: the parity inventory of the 2026-08-02 project, IDs PAR-01..26. The full inventory is kept privately with the layout (§5.2).

Proof codes:
- **U**: unit tests.
- **S**: hub tests against the real Live script running on SimLive (§5.3).
- **E**: Playwright E2E in Chromium and WebKit.
- **L**: live check on the Ableton PC.

| # | Feature | Parity detail (TouchOSC behaviour) | Proof |
|---|---|---|---|
| F1 | Pages and areas | Tabs: musician cue page, **FOH/WORSHIP** (default); no Conf tab (#58: TouchOSC's settings text is read by the import, not shown). FOH/WORSHIP has a nested pager STAGE / BAND B / OTHERS, a left sidebar (stage mics, STAGE AUT, three solos, MIDI toggles), the EFFECTS, MASTER A and HANDS areas, and the two top-right strips. TechAlert is on every page (PAR-01; REFRESH ALL removed, #58). The same pages, sections and order; the placement is computed (D13, #21) | E |
| F2 | Two instances | Strips route to band or master; a missing prefix means band (PAR-02, PAR-03) | S, E |
| F3 | Strip binding | Exact name match on tracks, then return tracks. Red status and disabled controls when missing. Re-resolved by the hub on every track-list or name change, also for a binding made while its name was missing (PAR-03; #58). A marker strip (D16) binds by the index of the track that holds its Tuner instead | S, E |
| F4 | Strip label | First word of the name, after dropping a leading `X-` return prefix. The instance is named once in the group's title when all its strips share it, else on each strip; a return in a named group shows RET on its name button (PAR-04; #63). A marker strip (D16) shows its Tuner's quoted label, never the track's name | U |
| F5 | Status pill | Red when unmapped. Yellow when a value arrived in the last 150 ms, fading to green by 500 ms (PAR-05). It sits with the dB readout on the line under the name button (#63) | E |
| F6 | Refresh | **Removed (#58, owner decision 2026-10-07).** No REFRESH ALL: the hub keeps every name binding current by itself (renames, track-list changes, a binding made while its name was missing: F3, #58). Was: automatic about 1 s after load, plus REFRESH ALL (0.5 s debounce, 300 ms yellow flash) (PAR-06) | E |
| F7 | Auto-unfold | **The hub** keeps every group a strip's track sits in (nested too) unfolded: `fold_state = false` when it reads it folded, at Live's connect or set load and whenever someone folds it. Live meters no track inside a folded group (verified on the PC, #58). No `unfold_<instance>` list any more (PAR-07; #58) | S, E |
| F8 | Fader law | Position *p* ↔ Live volume *v*: **Live's own law, *v* = *p*** (0 dB at 0.85, −20 dB at 0.36, −40 dB at 0.16: close to a console fader; #63, owner 2026-10-07), or TouchOSC's *v* = *p*^0.515 with `config.fader_law: "touchosc"`. The dB scale labels the law (Live's: +6 … −24, −30, −40, −60). Relative touch with no jump on grab (PAR-08) | U, E |
| F9 | Fader touch shaping | 0.1 dB minimum first step, reaction compensation, gradual 0.9→1.0 scaling, and bypass above 3 % per event (PAR-09). See X3 | U, E |
| F10 | Fader double-tap | Two taps 50–250 ms apart glide to 0 dB (*v* = 0.85) at about 0.3 position/s, time-based. A touch cancels (PAR-10) | U, E |
| F11 | Pan | *p* ↔ panning 2*p*−1, in the channel detail (F27, D17; it left the strip's foot, where the iPad's home bar waits for a swipe, and pans are rarely used). Double-tap within 300 ms glides to the centre at 0.4 position/s (iemmixer's; #63); the detail's STRED centres it too. Grey when centred, cyan otherwise; a bar from the centre (PAR-12, #21) | U, E |
| F12 | Mute + protection | Toggle, lit when audible: the name button at the top of the strip, where a hand holding the tablet does not cover it, 30 px high so the fader gets the height (#63). A muted strip steps back (darker, dimmed, a small red MUTE mark). Strips in `double_click_mute` need a second tap within 500 ms (PAR-13, PAR-14) | U, E |
| F13 | Volume dB text and meters | Volume dB text is Live's display string (X1). Meter bar: 300 ms rise, 200 ms fall, green/yellow/red. Scale per K2 (PAR-15, PAR-16, PAR-18). The meter dBFS label is dropped (D11) | U, E, L |
| F14 | Solo buttons | Three group-track solo toggles, independent (no exclusivity), lit when on (PAR-19). Owning instance only (X4). The SOLO ✕ pill turns off every solo of the page (#21) | S, E |
| F15 | Stage mics + STAGE AUT | Inverted mute button. The STAGE AUT flag lives in the hub; while on, the stage mics are muted while playing and live when stopped (PAR-20, §2.4, X9) | S, E |
| F16 | TechAlert | Full-screen red overlay blinking every 300 ms while the TechAlert track is unmuted (PAR-21) | E |
| F17 | Musician cue page (**after the core**, D12) | Six presence toggles (CC20, 22–26), which are mapped mutes: included if their mapping is clean (§3.2). Two instrument toggles (CC28, CC65) and a momentary ALERT LOOP (note 29) have **no mapping in either set**: dropped unless K3 finds a working consumer (PAR-22) | S, L |
| F18 | Main-page toggles and Podklady All (**after the core**, D12) | VOC MIC (double-tap latch), REVERB, AUTOTUNE, ZVUKAR (tap pulse, double-tap latch), REPRO, and the Podklady All fader: each included if its mapping is clean (§3.2) (PAR-23) | U, S, L |
| F19 | Visual layout | Our own design (D13, #21): the pages, sections and order from the import, the placement computed, the look of the redesign note §4 (a dB scale and a zone meter with peak hold and clip beside every fader, the name button in the track's Live colour) | E |
| F20 | Many clients | Any number of clients see the same state live. TouchOSC needed one script copy per tablet | S, E |
| F21 | Second tablet | If K5 finds a second TouchOSC client (band port 12000, the cue-page project), its project is imported the same way and served by the same hub | S, L |
| F22 | Current state on open | On app start, reconnect, Live reconnect or set load, every control shows Live's current value before it accepts input (I8). This is new: TouchOSC often failed to load the current state | S, E, L |
| F23 | Strips from markers | The hub reads both instances' tracks, return tracks, their device lists and the Tuners' names, follows every change, and composes the frame and the markers into the served layout: label, groups and places, pin, mute guard (D16) | S, E |
| F24 | Views | Every tag group the default view does not place is a view button in the control column: it shows that group's strips and the pins, a second tap returns (D16) | E |
| F25 | Marker problems | An equal label in two tracks disables both strips, marked; two Tuners in one track, two strips on one place, an unknown tag or a malformed value mark the strip; `/api/status` lists them (D16, I9) | S, E |
| F26 | Tag manual and tray | A `ZNAČKY` button in the column and an item in the tray's right-click menu open the tag manual the hub serves; the tray's left click opens fohmixer (D16) | E |
| F27 | Channel detail | A hold of 500 ms on a strip's ☰ (the right end of its readout line) opens that channel over the whole screen; a shorter touch only shows the hint. The detail is this channel only: its name, group and instance, `← SPÄŤ NA MIX`; the mute (red while muted, the strip's guard), Live's dB and a tall fader with its scale and meter on the left; the pan with `STRED` on the right; its Pro-Q 4 instances in the middle (F28). It stays open through a new layout while its strip is in it (D17). Status: the detail (#71 PR D) with its Pro-Q 4 cards (PR E) | U, E |
| F28 | Pro-Q 4 on the surface | Each Pro-Q 4 of the channel's track (on it or in a rack chain; `class_display_name`) is a card in the detail with its last picture; a tap opens it over the whole screen as a live picture of its editor, touched like the editor itself; `← SPÄŤ NA KANÁL` closes it and its window on the PC. One Pro-Q 4 editor is on the PC's screen at a time (one screen, one cursor; two would overlap), locked to the engineer who opened it until they leave or their connection closes: every other engineer's cards show it locked, and their faders, mute and pan stay free (the owner's call of 2026-10-09, #71). How touches reach the editor is decided on #71 (D17): injected touch, the editor on top; nothing is tried in a running Live until it is proven outside it. Status: the cards, the screen, the lock and the hub's window backend built and tested against the simulated backend (#71 PR E); the check against Pro-Q 4 in Carla's bridge on the PC (`eq-probe`) and the first open in the band Live are next | S, U, E |

### 3.2 The former MIDI toggles (D10, D12)

**Today.** The toggles in F17 and F18 send MIDI on channel 14 into band Live's MIDI map through a Bome virtual port. The song MIDI clips drive the same mappings through the loopback port. Master Live takes no MIDI. The owner reports that the MIDI/OSC mix was often half-working and did not show the current state on open.

**In fohmixer (D10), with no MIDI:**
- A toggle writes its target parameters directly through the LOM, with the ranges its MIDI mapping defines.
- The import tool reads the targets and ranges from the band set's MIDI mappings, so nothing is retyped.
- The shown state is the targets' real state from Live listeners, so song changes show too.
- The hub needs no MIDI and no Bome port.
- The MIDI map stays in the set for the songs.

**Priority and triage (D12).** These controls come after the OSC-equivalent core (F1–F16, F19–F22).
- The import tool triages each MIDI control and reports the result:
  - **clean**: the mapping resolves to one or more existing parameters with a clear range. It becomes a `ParamToggle` or `ParamFader`.
  - **dropped**: no mapping, ambiguous or missing targets, or behaviour that cannot be reproduced cleanly.
- CC28, CC65 and note 29 are dropped unless K3 finds a working consumer.
- A dropped control is listed on the ticket, not asked about one by one.

**Page-change CC0.** K3 checks what the TouchOSC page-change message (CC0, channel 1) hits today (X6).

### 3.3 Deviations (each has a reason)

- **X1** Volume dB text is Live's value (`str(param)`), instead of the TouchOSC `value2db` approximation (P2), shown in TouchOSC's form: one decimal, no unit, white exactly at 0 dB (#21).
- **X2** The meter source is `output_meter_left/right` if K2 shows the cost is acceptable, otherwise `output_meter_level`.
  - The bar's position on the fader scale uses a K2-measured scale. This is placement, not text.
  - The inert second meter bar is dropped.
- **X3** Fader touch shaping (F9) and the 1000 ms post-release snap (PAR-11) existed partly to hide AbletonOSC's 100 ms tick. v1 reproduces both behind one switch, for muscle memory. The engineer decides during the parallel run.
- **X4** Solo and stage-mic controls use only their owning instance. TouchOSC sent to all connections and could bind a name from the wrong instance.
- **X5** No battery gauge on the iPad. WebKit has no Battery Status API; the widget shows only where a browser provides it.
- **X6** The pager's page-change MIDI/OSC messages and the incoming OSC restyle receivers are not reproduced (no consumer or sender found). K3 checks whether the page-change CC0 hits mappings today; if it does, dropping it removes an accidental side effect.
- **X7** Stale config found by the import is reported, never silently fixed.
- **X8** A one-time PIN login per device (from iemmixer). The token persists, so there is no re-login during a service.
- **X9** STAGE AUT runs in the hub, not on the tablet, so it keeps working while the iPad sleeps and cannot conflict between clients.
- **X10** The Podklady All dB label, which never updated in TouchOSC, shows Live's display string of its first target.
- **X11** The former MIDI toggles write parameters directly, not MIDI (D10). Controls that triage drops (§3.2) are not reproduced (D12).

---

## 4. After v1: extending by prompt

### 4.1 How a new control is added

1. Find the target with `describe` on the live object.
2. If no existing component fits, write a new generic component. Example: `EqEightGraph`, which draws the EQ curve from the band parameters and edits them by drag.
3. Place it in the layout with the general binding form (§2.5).

The Live script and the protocol do not change (P4).

### 4.2 What the LOM reaches

- sends and returns;
- device and plugin parameters with their display strings (a large plugin may first need its parameters added in Live's Configure mode);
- rack chains (mute, solo, volume, chain selector) and macros;
- `fold_state`, `is_playing`, cue points and locator names;
- automation state and `re_enable_automation`;
- track colours (in v1 since #21: the name button);
- clip and scene launch.

### 4.3 What the LOM does not provide

LUFS, true-peak and pre-fader input meters are not in the LOM. Each needs a second source next to the Live instances, decided per feature when it is requested. Candidates:
- a small Max for Live meter device sending over localhost;
- an audio tap from the existing virtual audio routing.

### 4.4 More Live instances

The hub config takes any number of instances. A Live on another PC would need its script to listen beyond localhost, with a password (the reference supports one). That is a later decision.

---

## 5. Repo, CI, security

### 5.1 Repo layout

- `live-script/FohMixer/`: the Python script, with the vendored reference code and its LICENSE.
- `crates/fohmixer-proto`, `crates/fohmixer-hub`, `crates/fohmixer-ui`: the Rust workspace.
- `tools/import-tosc`: the one-shot import tool.
- `sim/`: SimLive.
- `e2e/`: Playwright.

The workspace has one version, shown in the UI and at `/api/version`.

### 5.2 What never enters this public repo

These stay out of the repo:
- host names, IPs and Windows user names;
- PINs, keys and certificates;
- the imported layouts (they hold real track names, which include people's first names);
- the parity inventory;
- the `.tosc` and `.als` files.

This follows the rule the owner set for the sibling public repo iemmixer. The layouts live on the Ableton PC with dated backups. The code, the import tool, and a synthetic `.tosc` fixture and layout are public. `.mcp.json` is git-ignored.

### 5.3 Testing without Live in CI

- **SimLive** is a fake of the external `Live` Python module: tracks, returns, master, mixer devices, parameters with display strings, devices and chains, listeners and the timer.
- **The real FohMixer script runs on top of SimLive.** Hub tests and Playwright E2E therefore exercise the real script, the real hub and the real UI; only Live itself, an external system, is simulated.
- SimLive's display strings are checked against strings captured from the real Live, so the fake does not drift.
- **Live checks (L)** run on the Ableton PC through the MCP tools and are recorded on the tickets.

### 5.4 CI

The iemmixer baseline, on GitHub-hosted runners:
- integrity;
- fmt and clippy (native and wasm32);
- tests with a coverage floor;
- Trunk build;
- Playwright E2E with the console-error guard, in Chromium **and WebKit**;
- cargo-deny, gitleaks, version check;
- diff-scoped mutation testing.

A Python job lints and tests the Live script on SimLive.

---

## 6. Sub-projects

Each one gets a short design note and a plan before code, as in iemmixer.

| # | Deliverable | Size (estimate) | Needs |
|---|---|---|---|
| S0 | Bootstrap: workspace, CI baseline, version display, SimLive skeleton, hygiene | 1–2k LoC | spec approval |
| S1 | **Checks on the real PC**, results recorded on tickets (see the list below) | small | S0; one Live restart per instance to put the script in a free slot |
| S2 | FohMixer Live script v1 (§2.2, §2.3) + SimLive complete | ~1.2k LoC | K1, K2 |
| S3 | Hub v1 (§2.4) + proto crate + import tool (§2.6) | 3–4k LoC | S2, K3 |
| S4 | UI v1: first the core (F1–F16, F19–F22), then the triaged former MIDI toggles (F17, F18) | 5–7k LoC | S3, K4 |
| S5 | Parallel run next to TouchOSC; engineer feedback loop | – | S4 |
| S6 | Cutover: engineer sign-off, every K5 client migrated, then remove the AbletonOSC copies, their logs and the import tool (with owner approval) | small | S5 |

**The S1 checks:**
- **K1** Timer interval and jitter; the vendored transport running next to AbleSet with meters subscribed; socket-to-drain and flush-to-wire delays; the longest single command. Idle and with the band set loaded.
- **K2** Meter source cost and the meter scale.
- **K3** The former MIDI toggles: the mapping triage on the real set, any consumer of CC28/CC65/note 29 outside Live, and the effect of the page-change CC0.
- **K4** iPad: Home Screen app over the chosen origin, multitasking gestures off, 4+ faders at once, frame rate, screen stays on.
- **K5** Which clients use the AbletonOSC ports today (source addresses during a service), and whether the cue-page project is current.

**Critical path:** S0 → S1 → S2 → S3 → S4 → S5 → S6. The UI components in S4 can start on SimLive as soon as S2 exists.

---

## 7. Risks

- **R1** Band Live main-thread hiccups (about 60 per service on 2026-09-27, mostly +100–200 ms) affect every LOM path. The UI stays responsive locally and shows "Live busy" during the stall. K1 measures them with FohMixer in place.
- **R2** `Live.Base.Timer` is undocumented. AbleSet runs it here at 10 ms. The threaded v5 transport is new upstream and unproven on this PC (K1). K1 showed its threads run only around the main thread's tick; since #5 the script's socket I/O runs in that tick, with no threads. The `schedule_message` fallback is kept.
- **R3** `output_meter_left/right` add GUI load, per Live's docs. K2 measures it; the fallbacks are `output_meter_level` or on-screen strips only.
- **R4** A mapping read from the saved set file can differ from the running set when the set has unsaved changes. The import is re-run after the set is saved, and the toggles show the live target state either way.
- **R5** Secure context on the iPad. **Resolved by #17:** the public name is served over HTTPS on the LAN and through the tunnel, so the iPad's Home Screen app has a secure context there: a network-only service worker (installable PWA, never a cached answer: the internet path sits behind Access) and a Screen Wake Lock taken while the page is visible, taken again when it comes back. The emergency plain-http path stays without either (iPad Auto-Lock set to Never remains the fallback).
  - v1 worked without a service worker or Wake Lock: the app was LAN-only and iPad Auto-Lock is set to Never.
  - K4 confirms that the Home Screen app works over the chosen origin.
  - HTTPS with a real certificate is added if needed.
- **R6** A Live update changes undocumented details. Pin the Live version per release; the script reports `live_version` on connect.
- **R7** The shared Python interpreter with AbleSet and the AbletonOSC copies. Mitigations: own folder name, relative imports, own port, own logger name, `SO_EXCLUSIVEADDRUSE`.
- **R8** Track or device renames break bindings. They show red and never mis-bind (I5); the hub heals them when the name is back (#58), and `/api/status` and the import report list unresolved names.
- **R9** Live may not report a change of `Track.devices` or `Device.name` through a listener (D16). Probed on the PC before the hub side is built; the fallback re-reads on every track-list push and on a slow timer.
- **R10** Quotes in a device name, and a renamed Tuner surviving a save and a copied track, are unproven. Probed on the PC first.
- **R11** A marker strip binds by its track's index, which moves when a track is inserted above it. The composed layout follows at once; the tablets then rebuild the page, as for any layout change (an edit of the set, not of a show).

---

## 8. Owner decisions

**Recorded on #3**

- **D1** Architecture: Rust hub + own thin Live script + PWA (2026-09-27).
- **D2** A thin layer over the native LOM, JSON, no OSC, no binary frames, no curated data model (2026-09-28).
- **D3** v1 = as close as possible to the TouchOSC surface; nothing re-entered (2026-09-28). Superseded for what is on the surface by D16 (2026-10-08).
- **D4** After v1, any control by prompt that the Live API allows (2026-09-28).
- **D5** AbleSet / `ableton-js` is the reference, not a dependency (2026-09-28).

- **D10** No MIDI: the former MIDI toggles write their target parameters directly through the LOM; the targets come from the set's MIDI mappings (2026-09-28).
- **D12** Priority: first the OSC-equivalent core; former MIDI controls only where they work cleanly, and Claude triages them (2026-09-28).
- **D13** TouchOSC's functionality, not its look: a modern surface of our own design (the approved mockup, `docs/mockups/redesign-stage-v1.html`); every TouchOSC behaviour kept (#21, 2026-09-28).
- **D14** Strips for a tablet held in the hands (#63, 2026-10-07; the owner delegated the choice): what is read sits at the top (the name button with Live's dB), the fader runs down towards the foot (the pan, at first under the name button, went under the fader on 2026-10-08: a pan touch hit the mute); the strip lights while its fader is held; a muted strip steps back; the fader follows Live's own law; the pan glides to the centre. Mockup: `docs/mockups/strips-ergonomics-v1.html`. The pan left the strip for the channel detail (D17, 2026-10-09).
- **D15** One control column (#63, the owner's design and approval of 2026-10-08; it replaces the phone's screen bar and overview bar of 0.1.0-dev.41–43): the fader's height is the base requirement, so nothing sits above the faders. One column in the middle of every screen, bottom to top, holds the status, the tabs, the arrows, the rail and TechAlert: the thumbs of two hands holding the tablet do not reach it. The page's rows keep the layout's own arrangement and are cut in two by it; each row is a line while every line keeps 340 px, else one line holds all rows. Strips the owner marks `pinned` never leave the screen: a pinned strip keeps its slot on every sub-page, and a line that does not fit shows its pinned strips and a window the column's arrows move. Which strips sit on the first view is the owner's choice, never an arrangement Claude invents. One layout for every screen. Mockup: `docs/mockups/surface-column-v3.html`.
- **D16** Strips from Tuner markers in the Live sets (#68; the owner's design and approval of 2026-10-08 on #3). TouchOSC is obsolete: no compatibility is kept and nothing is re-imported; the import tool goes once the sets carry markers. A **Tuner** in the device chain of a track or return track is a marker when its name has a quoted label or a ` +` tag, e.g. `"Vox 1" +G:VOCALS:2 +G:TALKSHOW:1 +PIN +MG`. Live's manual: the Tuner "does not alter the signal in any way"; switched off it costs no CPU. The quoted label is the strip's name, any words, never the track's name (Live allows equal track names; the `#` suffix ends). `+G:NAME[:N]` puts the strip in group NAME (uppercase letters, digits, `-`, `_` shown as a space) at place N, else in Live's track order, and repeats for several groups (one fader in several views); `+PIN` keeps it on screen in every view (D15); `+MG` is the mute guard (F12). The strip binds to the track that holds its Tuner, by index. The frame file stays on the PC and is the default view: its groups name the tag groups they show (a group may keep controls that are not strips, shown after its marker strips); every other tag group is a view button in the control column, which shows that group's strips and the pins, a second tap returns. Problems are shown, never dropped (I9): an equal label in two tracks disables both strips, marked. A `ZNAČKY` button in the column and the tray's right-click menu open the tag manual the hub serves; the tray's left click opens fohmixer. The surface parts are built after an approved mockup (D15). **Migration** (PR C): `fohmixer_proto::markers::migrate` turns each frame group of strips bound by name into a tags group of the same title, place and other controls, and plans each track's marker (its strip's label, `+G:` per group with its place, `+PIN`, `+MG`). The owner adds one plain Tuner to each planned track; the hub (`/api/markers/migration`, a token) lists them and names each ready track's Tuner with its marker; `fohmixer-hub markers frame` writes the converted frame for the PC; the surface looks the same.
- **D17** The channel detail and the real Pro-Q 4 (#71; the owner's design and approval of 2026-10-09, mockup `docs/mockups/channel-detail-v2.html` at 9f47355). A hold on a strip's ☰ opens one channel over the whole screen (F27): focused work on that channel, no other strip reachable; the pan moves there from the strip's foot. From the detail, a Pro-Q 4 of the channel opens full screen as the live picture of its own editor (F28), and exits go back step by step (EQ → channel → mix). One Pro-Q 4 editor is on the PC's screen at a time (one screen, one cursor; two would overlap), locked to the engineer who opened it; the other engineers' faders, mute and pan stay free. The hub opens and closes the editor through Live (`PluginDevice.is_editor_open`, Live 12.4.3+) and captures its window; posted mouse messages give hover, click and wheel but no drag (the 2026-10-09 test; a drag needs real input with the window on top), and that test crashed the band instance once (Pro-Q 4.02, `0xc0000409`, data 7: a known FabFilter close crash fixed in later versions). So nothing that touches a plug-in's window runs in a running Live before it is proven against Pro-Q in a separate host process.

**Defaults chosen in this spec** (the owner may override them at review)

- **D6** The v1 layout is imported from the TouchOSC projects and lives on the Ableton PC, not in the public repo (§2.5, §5.2). The strips' source is the Live sets' markers since D16; the frame stays on the PC.
- **D7** ~~v1 is LAN-only, with no internet exposure (R5).~~ Changed by the owner on #17 (2026-09-28): one name for the LAN and a Cloudflare Tunnel behind Cloudflare Access (e-mail), LAN first by the router's DNS; the mixer must stay openable on the Ableton PC and on any wired PC with the internet and the Wi-Fi down (hosts entry, the always-on emergency `http://<PC IP>:8480`); the engineer PIN stays everywhere (§2.4 Remote access).
- **D8** The fader touch shaping and post-release delay are kept in v1 behind a switch; the engineer decides during the parallel run (X3).
- **D9** The hub runs as a scheduled task at logon on the Ableton PC (iemmixer pattern).
- **D11** The meter dBFS number is dropped; the meter bar stays. The LOM has no display string for meters, and the TouchOSC number froze below −23 dB.
