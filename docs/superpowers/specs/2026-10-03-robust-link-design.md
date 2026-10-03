# Robust control link, dropout counter and audit trail (#43)

Status: direction approved by the owner on #43 (2026-10-03), with the added requirement of a link-quality indicator, which the owner then replaced by a dropout counter (ROZHODNUTÉ on #43, 2026-10-03). This note turns the direction into a buildable design for phases 1–3; phase 0's evidence is on #43.

## 0. Zhrnutie pre vlastníka

- **Žiadny pohyb sa nestratí.** Každý ovládač si pamätá posledný zámer zvukára. Kým ho Live nepotvrdí, mixér ho drží a po výpadku spojenia ho pošle znova. Výnimkou je pustenie fadra staršie než 2 s: to sa nepošle naslepo, fader to ukáže červeným obrysom (bez textu, miesto na tablete je vzácne) a čaká na dotyk.
- **Ovládanie nikdy nezamrzne.** Výpadok spojenia nevypne fadre. Nové dotyky fungujú ďalej a fader po pustení neskočí späť na starú hodnotu.
- **Hub neposiela do Live staré hodnoty.** Pre každý ovládač drží len najnovšiu hodnotu. Do Live ide vždy jedna dávka naraz, takže pri zaseknutom Live sa nehromadí fronta.
- **Počítadlo výpadkov** namiesto slov o kvalite: malé číslo v hornej lište, ktoré pri každom výpadku spojenia stúpne o jedna. Zvukár tak pod rukami vidí, že výpadky pribúdajú. Kým výpadok trvá, číslo je červené. Ťuknutím sa vynuluje. Bez zvuku a bez blikania.
- **Výpadok** je chvíľa, keď iPad aspoň 300 ms nič nepočuje od hubu, hoci čaká na odpoveď (iPad sa hubu ozýva každých 100 ms), alebo keď je spojenie prerušené. Zaseknutý Live sa zapíše, ale nezapočíta: počítadlo meria spojenie.
- **Čierna skrinka.** Hub zapisuje každý pohyb na celej ceste: odoslanie z iPadu, príchod do hubu a potvrdenie z Live. Zapisuje aj každú odozvu siete (10-krát za sekundu), každý výpadok s jeho dĺžkou a vynulovania počítadla. Záznamy sú v denných súboroch a uchovávajú sa 60 dní. Nástroj z nich vykreslí časovú os ľubovoľného úseku služby.
- **Rýchlejší prenos** (WebTransport, funguje ako UDP) príde v ďalšej etape. Najprv ho overím meraním na skutočnom iPade.

## 1. Evidence and goals

Phase 0 (#43, 2026-10-03; corrected on #43 the same day):
- At the second service the FOH iPad's page kept one socket for the whole service. That does **not** show a stable link: the page tears a socket down only after 3 s of silence, so today's logs cannot see a stall shorter than that. The network question is open.
- The owner's observation stands: on the same Wi-Fi TouchOSC (UDP) stayed usable and fohmixer did not. That points at how fohmixer carries moves over a lossy link (TCP stalls delivering moves late and in a burst, every move a request with its own result, a release during a stall snapping back to Live's stale value), and the next version's logs must prove or refute it.
- The page's `perf` report covers only the last 10 s window of each minute, so whether the page itself stuttered is not established either.
- The band Live's main-thread stalls in that log cluster around opening the set: known Live behaviour when a set opens or saves, outside our control, and not the cause the owner reported.
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
- **L6 — every hop is recorded** in the event log (§5) with the clock of the hop. The next deploy logs enough to analyse a stutter report (owner, #43): per move the page's send time, the hub's arrival (so per-move delay and the gaps between moves show a stall) and Live's confirmation; the page's ping round trips every 100 ms; every dropout with its length.

## 2. Protocol 2 (hub ⇄ page)

`UI_PROTO = MIN_CLIENT_PROTO = 2`. A page of protocol 1 reloads through the existing handshake.

**Client → hub (new or changed)**

| Message | Shape |
|---|---|
| Set | `{"type":"set","instance","target","prop","value","seq":<u64>,"t":<page ms>,"final":<bool>}` |
| Ping | `{"type":"ping","n":<u32>,"t":<page ms>,"rtt":<latest pong's round trip>?,"rtt_n":<the ping it measured>?}`, every 100 ms while the page is visible (every second while hidden); no `rtt` before a socket's first pong |
| Trace | `{"type":"trace","events":[…]}`: a batch of the flight recorder (§5.2); since PR A each finished dropout, `{"ev":"dropout","t":<start, page ms>,"ms":<length>,"socket_lost":<bool>,"rtts":[the last ≤ 5 round trips before it]}` |

- `seq` is per page session and strictly increasing over all sets of that page.
- `t` is `performance.timeOrigin + performance.now()`, in ms.
- `final` marks a release, a toggle or a tap. Faders, pans, parameter faders and toggles all use `set`. In PR A a fader's or pan's release sends a `set` only when a move is still unsent (the last frame's send carried the rest); the release time itself is the intent store's in PR B (L4).
- `cmd` stays for reads and multi-command batches (REFRESH ALL, ranges).

**Hub → client (new or changed)**

| Message | Shape |
|---|---|
| Ack | `{"type":"ack","items":[{"key","seq","value"?,"error"?,"superseded"?}]}`, coalesced per client, per key the highest `seq` |
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
- **Disconnect.** An instance disconnect clears `in_flight` and the pending wants without acks: they belong to the old Live session (the clients resend per L4).
- **Pure core.** The setter is a pure state machine (`crates/fohmixer-hub/src/setter.rs`), driven by the router task; it is tested natively without Live.

### 3.2 Ping, link

- The hub answers a ping with `pong` at once (through the outbox's ordered replies, as today) and logs every ping (§5.1).
- The hub forwards each instance's `busy` and the age of Live's last main-thread tick as `link` to every client (the last heartbeat's `main_tick_age_ms` plus the time since that heartbeat, so it grows during a stall); a client's outbox keeps the latest `link` per instance.

### 3.3 Logs

- Retention of `hub.out.log`: `Start-FohmixerHub.ps1` keeps the last 20 starts (`hub.out.<UTC stamp>.log`) instead of one `.prev`.
- The event log (§5) is separate and dated.

## 4. Page

### 4.1 The intent store (`store/intent.rs`, pure)

Per key it holds the open intent `{value, seq, t, final, released_at, not_sent}` (a closed one is dropped). The state is one of:

- `sending`
- `confirmed` (no open intent)
- `unconfirmed` (released, no ack for 1 s)
- `not_sent` (L4 expired at a resend)

Rules:

- **Sending.** A control's send goes to the store, never straight to the socket. The store sends it when the socket is ready and keeps it until an ack with `seq ≥` its seq.
- **When an instance is back.** The store resends that instance's intents per L4: right after each hello (the hub reports every instance then, and a socket loss marked them offline on the page) and when an instance comes back online on the same socket (the hub's setter dropped its pending wants). Each goes as a new `set` (the page's next seq, `t` = the time it goes). Sends are rate-capped per key at one per animation frame (as today).
- **Release and touch.** A release with no final write (the frames already sent the last move, or a glide ended) records its time; a control taken away under a finger counts as released then. A touch on a control with a write still on its way holds that write again (no release time until the touch's own release); a touch on a `not_sent` one drops it.
- **Live already holds it.** A `not_sent` write whose value Live's fresh value equals (float32) is closed: the hub applied it before the link went, its ack was lost. Any other value leaves it (another writer's included): the red outline and the ghost show the difference until the next touch.
- **Toggles.** A toggle keeps showing Live's value (P2) and a tap inverts what it shows: nothing on a toggle shows a write still on its way, so a second tap during an outage means "it did not take", not "undo".
- **Errors.** An ack with `error` marks the key failed: the red flash as today, and the fader then shows Live's value.

### 4.2 Values and touches

- **Keeping the value.** A socket loss no longer turns a known slot into `Pending`: the slot keeps its last value, marked stale, and so does a page switch while the hub is not connected (a page re-entered during an outage still takes touches). I8's "enabled only after the first value" holds for the first load, after an instance reports offline, and for a key subscribed again while the hub is connected (its fresh value is on its way).
- **Taking touches.** `on_down` takes the touch when the slot has a value, stale or not (L2).
- **What a fader shows** (`FaderCtl::frame`):
  - while the key has an open intent (`sending`, `unconfirmed`, `not_sent`), the cap shows the intent and Live's value only moves the ghost (L3); a fader built while its write is open (a page switch) shows the write, and the next touch starts from it;
  - the intent closes on an ack whose seq is at least the intent's, or on a `superseded` ack;
  - after that the cap shows Live's value again, after today's post-release hold, which runs again from the close.

### 4.3 How states look

| State | Look |
|---|---|
| `sending` / `confirmed` | as today |
| `unconfirmed` | the cap outlined in amber; a thin ghost line at Live's value |
| `not_sent` | the cap outlined in red and a ghost at Live's value, no text (owner, #43: space on the tablet is scarce); the next touch starts from the cap |

### 4.4 The dropout counter

The owner ruled out status words on the surface (ROZHODNUTÉ on #43, 2026-10-03): space on the tablet is scarce. Instead the surface shows how often the link drops out.

**What a dropout is.**

- The page pings the hub every 100 ms while visible (every second while hidden).
- A **dropout** is one continuous interval in which the page hears nothing from the hub for ≥ 300 ms although a pong is due, or the socket is down. The silence counts from the later of the last message heard and the oldest unanswered ping, so a hidden page that pinged rarely raises no false dropout.
- Each interval counts once, however long it lasts, and a silence that turns into a lost socket stays one dropout until the next hello.
- A silence counts only while the dropout watch's own page tick keeps coming (every 100 ms, independent of any socket, so it goes on between a close and the next hello): a page that was frozen or hidden (300 ms or more since the last tick) cannot tell the link's silence from its own, so the silence starts over at its next tick; a socket lost meanwhile is a dropout from the page's next on-time tick.
- A Live-side delay (Live busy, the network fine) is logged (`link`) but not counted: the counter measures the link.

**What the surface shows (PR C).**

- A small number in the top bar's status cluster that grows by one with every dropout, so the engineer sees it rising under the hands.
- While a dropout lasts the number is red; otherwise it is neutral.
- A tap resets it to 0.
- No words, no sound, no blinking: live mixing must not be disturbed.

**Event log.** Each dropout, when it ends, goes to the hub's event log as a `trace` event (start, length, whether the socket was lost, the last ≤ 5 round trips before it), and so does every reset of the counter (PR C).

The detection is a pure state machine (`behave/link.rs`, `DropoutWatch`, since PR A); its constants (100 ms, 300 ms) are pinned by tests and adjusted from real data once the event log has a service, decided on #43.

## 5. Audit trail

### 5.1 Hub event log

- **Files.** `logs\events-YYYY-MM-DD.jsonl` (UTC date), one JSON object per line, written by one task through a bounded channel. When the channel is full, the event is counted, never blocking: a `dropped` count is logged at the next write.
- **Retention.** Files older than 60 days are deleted at start and daily. A day file is capped at 256 MB (past it: `cap` once, then only `warn`-class events).
- **Records** (`ev` field). Every record carries `ts` (hub UTC ms) and, where it applies, `client`, `peer`, `instance`, `key`, `seq`, `t` (page ms).

| `ev` | Fields |
|---|---|
| `sock` | `open` / `close`, reason |
| `set` | `value`, `final`, `t`, `hub_ms` (its arrival), `offset_ms` and `delay_ms` (`hub_ms − (t + offset)`, its one-way delay), the gap since this client's previous set of that key, `dropped_old` (seq not newer) |
| `batch` | `instance`, the batch number, `n`, `sent` (key, client, seq, value of each) |
| `applied` | `instance`, the batch number, `n`, `rtt_ms` (Live's round trip), errors, `sent` (key, client, seq) |
| `ack` | per item; an ack of a batch with its number and `rtt_ms` |
| `ping` | every ping (10 a second while the page is visible): `n`, `t`, `hub_ms` (its arrival), the page's latest `rtt` and `rtt_n`, `offset_ms` |
| `trace` | the page's events, as sent: since PR A its dropouts (§4.4); the flight recorder (§5.2) and the counter's resets in PR C |
| `link` | busy changes with `tick_age_ms`, heartbeat gaps |

- **Clocks.** Page times (`t`) map to hub time through the ping exchange (Cristian): a ping carries the latest pong's round trip and the number `m` of the ping it measured, and `offset = arrival of ping m − (its t + rtt/2)` from the hub's ring of the last 64 pings, so a round trip is paired with the exchange it measured (ping n − 2 or older on a slow link), never with the carrying ping that may itself have been held up; the lowest-RTT exchange of the last minute wins. Each `ping` record carries it as `offset_ms`, and each `set` record the offset of its socket's last ping with the one-way delay it gives: a stall on the way shows as a gap and a delay spike on the moves after it.

### 5.2 Page flight recorder (`diag/trace.rs`)

- **Contents.** A ring of the page's own events:
  - touch down / up / cancel with the key;
  - each send (`seq`) and ack;
  - pings and pongs with RTT;
  - socket transitions with reasons;
  - frames longer than 50 ms;
  - visibility;
  - dropouts and the counter's resets.
- **Size.** At most 20 000 events and 2 MB.
- **Upload.** Sent as `trace` batches every 2 s while connected. After a reconnect the backlog goes first, so an outage is recorded from the page's side.
- **Reload.** A reload loses an unsent backlog. The page keeps nothing in browser storage, because L1–L4 live in memory too.

### 5.3 Forensics timeline (`tools/forensics/timeline.py`)

- **Input and output.** Stdlib Python, run on the PC or on a copied log: `timeline.py --events <dir> --from <local time> --to <local time> [--key …] --out report.html`.
- **What the report shows.**
  - Per control: three lines — the page's sends, the hub's arrivals and Live's applied values — with gaps over 100 ms marked.
  - The page's RTT and stalls, Live's busy episodes, the dropouts and the socket transitions.
  - A summary table: worst confirmation latency, longest stall, jumps over 3 dB between two applied values with their cause.
- **Hygiene.** It never writes names or addresses into the report header beyond what the log holds. The report stays on the PC or goes to the owner through `share`, never into the repo.

## 6. Proof

### 6.1 The impair proxy

The E2E harness gets a TCP proxy between the browser and the hub (`e2e/harness/impair.py`, asyncio, stdlib) with control routes:

- `POST /link/stall {"ms"}`: hold both directions;
- `POST /link/drop`: close every proxied socket with a reset;
- `POST /link/block {"on"}`: hold new connections (accepted, not passed on to the hub, until the block is lifted; a refused connection would be a console error in the browser, a held one is not).

The Playwright projects open the surface through it (`E2E_LINK_URL`); the tests' own client of the hub stays on the hub's port. Deterministic stalls make the tests reproducible; random `tc netem` stays out of CI.

### 6.2 Tests

RED first, against today's code:

1. A fader released while the link is dropped reaches SimLive after the link returns (within 2 s): **fails today** (L1).
2. A fader touched while the socket reconnects moves: **fails today** (L2).
3. During a 1.5 s stall with a release inside it, the fader never shows SimLive's pre-stall value after the release: **fails today** (L3).
4. A release dropped for 5 s is not applied after the link returns; the fader is drawn `not_sent` (red outline + ghost).
5. The hub setter (native tests):
   - a stalled instance (SimLive `stall 1000`) receiving 60 sets for one key gets ≤ 2 batches, and its final value is the last set;
   - two clients: the newer `t_hub` wins;
   - an old seq is dropped.
6. The dropout counter:
   - pure tests on `behave/link.rs` (PR A): a silence of 299 ms is no dropout, 300 ms is one; a long silence counts once; a silence that becomes a lost socket is one dropout until the next hello; a lost socket counts even when short; nothing counts before the first hello; the round trips a report carries;
   - in E2E (PR A): the hub's messages held 800 ms give one reported dropout (socket kept), a dropped socket another (socket lost), both in the event log;
   - PR C: the number rises by one per dropout, is red while one lasts, a tap resets it to 0, each reset in the event log.
7. The event log:
   - a test drags a fader through a stall;
   - it reads the day file: `set` → `batch` → `applied` → `ack` for the last seq, a `trace` with the touch and the dropouts;
   - `timeline.py` renders that window with the stall marked.
8. Mutation (the existing gate) covers the setter, the intent store and the dropout watch.

All Playwright tests keep the zero-console-error assertion, on Chromium and WebKit.

## 7. Phases and PRs

1. **PR A — protocol 2 and hub.**
   - `set` / `ack` / `pong` / `link`, the setter, the hub event log with retention, the log rotation in the launcher;
   - the page sends sets through a minimal intent store (L5, L6), pings every 100 ms while visible, and reports each dropout (§4.4) to the event log;
   - tests 5, 6 (the detection), 7 (hub part).
2. **PR B — the page's resilience.**
   - L1–L4, the stale slots, the touch rules, the fader states, the impair proxy;
   - tests 1–4.
3. **PR C — counter and audit.**
   - the dropout counter on the surface, the flight recorder, `timeline.py`;
   - tests 6 (the counter), 7.
4. **Phase 2 (own design note) — datagram transport.** A spike measured on the FOH iPad (Safari 26.6.1: WebTransport datagrams vs a WebRTC data channel), then the datagram path for `set`, with the WebSocket as fallback and the same seq rules.
5. **Phase 3 — verification.** A degraded-link check on the PC with the real iPad, then a service, read through `timeline.py`.

Each PR is deployed and verified on the PC per `deploy-pc.md` before the next starts.

## 8. Not in scope

- A second physical path (wired iPad, an access point at FOH). Recommended to the owner on #43; it is an installation step, not code.
- Changing Live's main-thread load. Phase 1 makes it visible; if the logs show the FohMixer script's own share, that is its own ticket.
