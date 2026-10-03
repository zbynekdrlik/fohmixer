# Robust control link, link-quality indicator and audit trail (#43)

Status: direction approved by the owner on #43 (2026-10-03), with the added requirement of a link-quality indicator. This note turns the direction into a buildable design for phases 1–3; phase 0's evidence is on #43.

## 0. Zhrnutie pre vlastníka

- **Žiadny pohyb sa nestratí.** Každý ovládač si pamätá posledný zámer zvukára. Kým ho Live nepotvrdí, mixér ho drží a po výpadku spojenia ho pošle znova. Výnimkou je pustenie fadra staršie než 2 s: to sa nepošle naslepo, fader ukáže „neodoslané“ a čaká na dotyk.
- **Ovládanie nikdy nezamrzne.** Výpadok spojenia nevypne fadre. Nové dotyky fungujú ďalej a fader po pustení neskočí späť na starú hodnotu.
- **Hub neposiela do Live staré hodnoty.** Pre každý ovládač drží len najnovšiu hodnotu. Do Live ide vždy jedna dávka naraz, takže pri zaseknutom Live sa nehromadí fronta.
- **Ukazovateľ kvality spojenia** je stále viditeľný v hornej lište. Povie, či je všetko v poriadku, či fadre reagujú s oneskorením (a o koľko ms), alebo či je spojenie preč. Uvedie aj príčinu: sieť, alebo zaťažený Live. Ťuknutím ukáže čísla.
- **Čierna skrinka.** Hub zapisuje každý pohyb na celej ceste: odoslanie z iPadu, príchod do hubu a potvrdenie z Live. Zapisuje aj odozvu siete, zaseknutia Live a zmeny ukazovateľa. Záznamy sú v denných súboroch a uchovávajú sa 60 dní. Nástroj z nich vykreslí časovú os ľubovoľného úseku služby.
- **Rýchlejší prenos** (WebTransport, funguje ako UDP) príde v ďalšej etape. Najprv ho overím meraním na skutočnom iPade.

## 1. Evidence and goals

Phase 0 (#43, 2026-10-03):
- At the second service the FOH iPad's page ran at 57–60 fps with one socket for the whole service.
- The band Live's main thread stalled 150–470 ms many times early in the service, and 5.7 s twice before it.
- The first service's log was lost to the per-start rotation.
- Nothing recorded per-move latency or the Wi-Fi link.

The code review on #43 found further defects:
- **Moves made while the socket is not ready are dropped.**
- A release during an outage never reaches Live, and the fader snaps back to Live's old value.
- New touches are refused until values return.
- Every move is a separate command with its own result.

**Invariants added by this design** (numbered after the program spec's I1–I8):

- **L1 — no lost intent.** The latest value the engineer set on a control reaches Live while the page lives. The exceptions are a newer intent on that control (from this or another client) and the staleness bound of L4.
- **L2 — never refused.** A control that shows a value takes touches, connected or not.
- **L3 — no snap-back.** After a release, a fader shows its own value until the hub confirms that value or a newer one, or another writer's newer value arrives. A fader whose release is unconfirmed past 1 s is drawn as *unconfirmed* (§4.3); it never jumps to a stale value.
- **L4 — bounded staleness.** After a reconnect, the page resends:
  - a held control's current value;
  - a released control's value only if the release is younger than `RESEND_MAX_AGE_MS` = 2000.
  
  An older unconfirmed release is shown as *not sent* (§4.3) and is dropped when the engineer touches that control again.
- **L5 — latest-wins, bounded.** Every queue between the finger and Live holds at most one value per control.
- **L6 — every hop is recorded** in the event log (§5) with the clock of the hop.

## 2. Protocol 2 (hub ⇄ page)

`UI_PROTO = MIN_CLIENT_PROTO = 2`. A page of protocol 1 reloads through the existing handshake.

**Client → hub (new or changed)**

| Message | Shape |
|---|---|
| Set | `{"type":"set","instance","target","prop","value","seq":<u64>,"t":<page ms>,"final":<bool>}` |
| Ping | `{"type":"ping","n":<u32>,"t":<page ms>,"rtt":<ms of the previous pong>?}` |
| Trace | `{"type":"trace","events":[…]}`: a batch of the flight recorder (§5.2) |

- `seq` is per page session and strictly increasing over all sets of that page.
- `t` is `performance.timeOrigin + performance.now()`, in ms.
- `final` marks a release, a toggle or a tap. Faders, pans, parameter faders and toggles all use `set`.
- `cmd` stays for reads and multi-command batches (REFRESH ALL, ranges).

**Hub → client (new or changed)**

| Message | Shape |
|---|---|
| Ack | `{"type":"ack","items":[{"key","seq","value"?,"error"?,"superseded"?}]}`, coalesced per client, latest per key |
| Pong | `{"type":"pong","n","t","h":<hub ms>}`: echoes the ping |
| Link | `{"type":"link","instance","tick_age_ms","busy"}`: on every busy change and at most 4/s while busy |

- The key is `instance|target|prop` (the hub key without `display`).
- An ack means one of:
  - Live ran that set (`value` = the value Live reported, when the result has one);
  - Live refused it (`error`);
  - another client's newer set replaced it before it was written (`superseded`).

## 3. Hub

### 3.1 The setter (one per instance)

- **State.** `pending: BTreeMap<key, Want>`, where `Want = {value, client, seq, t_page, t_hub, final}`, plus `in_flight: Option<Batch>`.
- **On `set`.**
  - The hub drops it when `seq` is not above the last seq it took from that client for that key.
  - Otherwise it replaces the key's `Want`. When the slot held another client's want, the newer `t_hub` wins and the replaced client gets an ack with `superseded`.
- **Writing.** When nothing is in flight, the hub sends every pending want as ONE `set_prop` batch (`LiveHandle::call`) and moves them to `in_flight`. When the result returns:
  - the hub acks every item to the client that sent it;
  - it records the batch's round trip;
  - it writes the next batch if anything is pending.
  
  Live therefore receives at most one batch per result, which is naturally one per Live tick when Live is healthy. During a stall nothing piles up, only the newest want per key (L5).
- **Errors.** A timeout (`REQUEST_TIMEOUT`, 3 s) or offline acks `error` to the senders and leaves newer wants pending.
- **Disconnect.** An instance disconnect clears `in_flight` without acks (the clients resend per L4).
- **Pure core.** The setter is a pure state machine (`crates/fohmixer-hub/src/setter.rs`), driven by the router task; it is tested natively without Live.

### 3.2 Ping, link

- The hub answers a ping with `pong` at once (through the outbox's ordered replies, as today).
- The hub forwards each instance's `busy` and `main_tick_age_ms` as `link` to every client.

### 3.3 Logs

- Retention of `hub.out.log`: `Start-FohmixerHub.ps1` keeps the last 20 starts (`hub.out.<UTC stamp>.log`) instead of one `.prev`.
- The event log (§5) is separate and dated.

## 4. Page

### 4.1 The intent store (`store/intent.rs`, pure)

Per key it holds `{value, seq, t, final, released_at, acked_seq, state}`. The state is one of:

- `sending`
- `confirmed`
- `unconfirmed` (released, no ack for 1 s)
- `not_sent` (L4 expired)

Rules:

- **Sending.** A control's send goes to the store, never straight to the socket. The store sends it when the socket is ready and keeps it until an ack with `seq ≥` its seq.
- **On hello.** The store resends per L4. Sends are rate-capped per key at one per animation frame (as today).
- **Errors.** An ack with `error` marks the key failed: the red flash as today, and the fader then shows Live's value.

### 4.2 Values and touches

- **Keeping the value.** A socket loss no longer turns a known slot into `Pending`: the slot keeps its last value, marked stale. I8's "enabled only after the first value" holds for the first load, and after an instance reports offline.
- **Taking touches.** `on_down` takes the touch when the slot has a value, stale or not (L2).
- **What a fader shows** (`FaderCtl::frame`):
  - while the key has an open intent (`sending`, `unconfirmed`, `not_sent`), the cap shows the intent and Live's value only moves the ghost (L3);
  - the intent closes on an ack whose seq is at least the intent's, or on a `superseded` ack;
  - after that the cap shows Live's value again, after today's post-release hold.

### 4.3 How states look

| State | Look |
|---|---|
| `sending` / `confirmed` | as today |
| `unconfirmed` | the cap outlined in amber; a thin ghost line at Live's value |
| `not_sent` | the cap outlined in red, a ghost at Live's value, the label `neodoslané`; the next touch starts from the cap |

### 4.4 The link-quality indicator

**Where.** The `hub-dot` in the top bar's status cluster becomes the link badge: a dot + a short word, always visible. A tap opens a small panel with the numbers.

**Inputs** (rolling 10 s window, recomputed 4×/s):

- **Network.**
  - RTT of pings, sent every 250 ms while visible: p50 and max.
  - The current stall: time since the last message from the hub while a pong is due.
  - Socket state.
- **Live.** `busy` and `tick_age_ms` from `link`.
- **End to end.**
  - The confirmation latency of sets: send → ack at the page, p95 over the window.
  - The age of the oldest unconfirmed intent.

**Levels.** First match wins; each level names its cause.

| Level | When | Word | Panel text (Slovak, what it means for mixing) |
|---|---|---|---|
| `offline` | socket down | `OFFLINE` | `Spojenie s mixérom je preč {s} s. Pohyby sa ukladajú a pošlú sa po obnovení (do 2 s po pustení).` |
| `bad` | stall ≥ 1000 ms, or confirmation p95 ≥ 250 ms, or an unconfirmed intent ≥ 1 s | `ZLÉ` | `Fadre reagujú o ~{p95} ms neskôr a môžu skákať. Príčina: {sieť / Live je zaťažený}.` |
| `slow` | stall ≥ 250 ms, or p95 ≥ 80 ms, or RTT max ≥ 150 ms, or Live busy | `POMALÉ` | `Fadre reagujú o ~{p95} ms neskôr. Príčina: {sieť / Live je zaťažený}.` |
| `ok` | otherwise | `OK` | `Fadre reagujú do ~{p95} ms.` |

- **Cause.** The cause is Live when Live is busy or the confirmation latency minus RTT p50 exceeds 60 ms; otherwise it is the network.
- **Hysteresis.** A level goes up at once and goes down only after 3 s at the lower level, so the badge does not flicker.
- **Colours.** green / amber / red / red with a dark background.
- **No sound or blinking.** Live mixing must not be disturbed.
- **Event log.** Every level change is sent to the event log (§5).

The thresholds are constants in `behave/link.rs`, pinned by tests. They are adjusted from real data once the event log has a service, decided on #43.

## 5. Audit trail

### 5.1 Hub event log

- **Files.** `logs\events-YYYY-MM-DD.jsonl` (UTC date), one JSON object per line, written by one task through a bounded channel. When the channel is full, the event is counted, never blocking: a `dropped` count is logged at the next write.
- **Retention.** Files older than 60 days are deleted at start and daily. A day file is capped at 256 MB (past it: `cap` once, then only `warn`-class events).
- **Records** (`ev` field). Every record carries `ts` (hub UTC ms) and, where it applies, `client`, `peer`, `instance`, `key`, `seq`, `t` (page ms).

| `ev` | Fields |
|---|---|
| `sock` | `open` / `close`, reason |
| `set` | `value`, `final`, the gap since this client's previous set of that key, `dropped_old` (seq not newer) |
| `batch` | `instance`, `n`, `sent` |
| `applied` | `instance`, `n`, `rtt_ms`, errors |
| `ack` | per item |
| `ping` | `n`, `t`, the page's `rtt` of the previous pong, `offset_ms` |
| `trace` | the page's flight-recorder events, as sent |
| `link` | busy changes with `tick_age_ms`, heartbeat gaps |
| `level` | the page's indicator level changes, with the inputs |

- **Clocks.** Page times (`t`) map to hub time through the ping exchange: `offset = hub arrival − (t + rtt/2)`, from the lowest-RTT ping of the last minute (Cristian). Each `ping` record carries it as `offset_ms`.

### 5.2 Page flight recorder (`diag/trace.rs`)

- **Contents.** A ring of the page's own events:
  - touch down / up / cancel with the key;
  - each send (`seq`) and ack;
  - pings and pongs with RTT;
  - socket transitions with reasons;
  - frames longer than 50 ms;
  - visibility;
  - indicator level changes.
- **Size.** At most 20 000 events and 2 MB.
- **Upload.** Sent as `trace` batches every 2 s while connected. After a reconnect the backlog goes first, so an outage is recorded from the page's side.
- **Reload.** A reload loses an unsent backlog. The page keeps nothing in browser storage, because L1–L4 live in memory too.

### 5.3 Forensics timeline (`tools/forensics/timeline.py`)

- **Input and output.** Stdlib Python, run on the PC or on a copied log: `timeline.py --events <dir> --from <local time> --to <local time> [--key …] --out report.html`.
- **What the report shows.**
  - Per control: three lines — the page's sends, the hub's arrivals and Live's applied values — with gaps over 100 ms marked.
  - The page's RTT and stalls, Live's busy episodes, the indicator's levels and the socket transitions.
  - A summary table: worst confirmation latency, longest stall, jumps over 3 dB between two applied values with their cause.
- **Hygiene.** It never writes names or addresses into the report header beyond what the log holds. The report stays on the PC or goes to the owner through `share`, never into the repo.

## 6. Proof

### 6.1 The impair proxy

The E2E harness gets a TCP proxy between the browser and the hub (`e2e/harness/impair.py`, asyncio, stdlib) with control routes:

- `POST /link/stall {"ms"}`: hold both directions;
- `POST /link/drop`: close every proxied socket with a reset;
- `POST /link/block {"on"}`: refuse and hold new connections.

The Playwright projects open the surface through it. Deterministic stalls make the tests reproducible; random `tc netem` stays out of CI.

### 6.2 Tests

RED first, against today's code:

1. A fader released while the link is dropped reaches SimLive after the link returns (within 2 s): **fails today** (L1).
2. A fader touched while the socket reconnects moves: **fails today** (L2).
3. During a 1.5 s stall with a release inside it, the fader never shows SimLive's pre-stall value after the release: **fails today** (L3).
4. A release dropped for 5 s is not applied after the link returns; the fader shows `neodoslané`.
5. The hub setter (native tests):
   - a stalled instance (SimLive `stall 1000`) receiving 60 sets for one key gets ≤ 2 batches, and its final value is the last set;
   - two clients: the newer `t_hub` wins;
   - an old seq is dropped.
6. The indicator (pure tests on `behave/link.rs`), every level boundary, hysteresis and cause. In E2E: a 400 ms stall shows `POMALÉ`, a dropped link `OFFLINE`, both back to `OK`.
7. The event log:
   - a test drags a fader through a stall;
   - it reads the day file: `set` → `batch` → `applied` → `ack` for the last seq, a `trace` with the touch, the `level` changes;
   - `timeline.py` renders that window with the stall marked.
8. Mutation (the existing gate) covers the setter, the intent store and the level function.

All Playwright tests keep the zero-console-error assertion, on Chromium and WebKit.

## 7. Phases and PRs

1. **PR A — protocol 2 and hub.**
   - `set` / `ack` / `pong` / `link`, the setter, the hub event log with retention, the log rotation in the launcher;
   - the page sends sets through a minimal intent store (L5, L6);
   - tests 5, 7 (hub part).
2. **PR B — the page's resilience.**
   - L1–L4, the stale slots, the touch rules, the fader states, the impair proxy;
   - tests 1–4.
3. **PR C — indicator and audit.**
   - the link badge and panel, the flight recorder, `timeline.py`;
   - tests 6, 7.
4. **Phase 2 (own design note) — datagram transport.** A spike measured on the FOH iPad (Safari 26.6.1: WebTransport datagrams vs a WebRTC data channel), then the datagram path for `set`, with the WebSocket as fallback and the same seq rules.
5. **Phase 3 — verification.** A degraded-link check on the PC with the real iPad, then a service, read through `timeline.py`.

Each PR is deployed and verified on the PC per `deploy-pc.md` before the next starts.

## 8. Not in scope

- A second physical path (wired iPad, an access point at FOH). Recommended to the owner on #43; it is an installation step, not code.
- Changing Live's main-thread load. Phase 1 makes it visible; if the logs show the FohMixer script's own share, that is its own ticket.
