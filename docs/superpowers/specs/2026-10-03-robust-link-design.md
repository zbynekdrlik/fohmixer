# Robust control link, dropout counter and audit trail (#43)

Status: direction approved by the owner on #43 (2026-10-03), with the added requirement of a link-quality indicator, which the owner then replaced by a dropout counter (ROZHODNUTÉ on #43, 2026-10-03). This note turns the direction into a buildable design for phases 1–3; phase 0's evidence is on #43.

## 0. Zhrnutie pre vlastníka

- **Žiadny pohyb sa nestratí.** Každý ovládač si pamätá posledný zámer zvukára. Kým ho Live nepotvrdí, mixér ho drží a po výpadku spojenia ho pošle znova. Výnimkou je pustenie fadra staršie než 2 s: to sa nepošle naslepo, fader to ukáže červeným obrysom (bez textu, miesto na tablete je vzácne) a čaká na dotyk.
- **Ovládanie nikdy nezamrzne.** Výpadok spojenia nevypne fadre. Nové dotyky fungujú ďalej a fader po pustení neskočí späť na starú hodnotu.
- **Hub neposiela do Live staré hodnoty.** Pre každý ovládač drží len najnovšiu hodnotu. Do Live ide vždy jedna dávka naraz, takže pri zaseknutom Live sa nehromadí fronta.
- **Počítadlo výpadkov** namiesto slov o kvalite: malé číslo v hornej lište, ktoré pri každom výpadku spojenia stúpne o jedna. Zvukár tak pod rukami vidí, že výpadky pribúdajú. Kým výpadok trvá, číslo je červené. Ťuknutím sa vynuluje. Bez zvuku a bez blikania.
- **Výpadok** je chvíľa, keď iPad aspoň 300 ms nič nepočuje od hubu, hoci čaká na odpoveď (iPad sa hubu ozýva každých 100 ms), alebo keď je spojenie prerušené. Zaseknutý Live sa zapíše, ale nezapočíta: počítadlo meria spojenie.
- **Čierna skrinka.** Hub zapisuje každý pohyb na celej ceste: odoslanie z iPadu, príchod do hubu a potvrdenie z Live. Zapisuje aj každú odozvu siete (10-krát za sekundu), každý výpadok s jeho dĺžkou a vynulovania počítadla. Záznamy sú v denných súboroch a uchovávajú sa 60 dní. Nástroj z nich vykreslí časovú os ľubovoľného úseku služby.
- **Ani dlhý ťah prstom sa nestratí (PR E).** iPad si drží každý pohyb prsta aj počas 30 s ťahu dvoma fadrami naraz a pošle ho hubu, až keď prsty pustia (počas ťahu nič nepridáva pred pohyby fadrov). Ak by sa záznam predsa len musel skrátiť, zapíše, od najstaršieho po najnovší vynechaný záznam, a nástroj ten úsek ukáže ako „bez údajov“, nikdy ako zaseknutie fadra.
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
| Trace | `{"type":"trace","events":[…]}`: a batch of the flight recorder (§5.2), each event with `ev` and the page's `t`; a dropout is `{"ev":"dropout","t":<start, page ms>,"ms":<length>,"socket_lost":<bool>,"rtts":[the last ≤ 5 round trips before it]}` |

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

- **Keeping the value.** A socket loss no longer turns a known slot into `Pending`: the slot keeps its last value, marked stale, and so does a page switch while the hub is not connected (a page left and re-entered during one outage still takes touches; a page left while connected, or before a reconnect, comes back waiting for Live's value). I8's "enabled only after the first value" holds for the first load, after an instance reports offline, and for a key subscribed again while the hub is connected (its fresh value is on its way).
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

The pan and the toggles (mute, solo, stage, the former MIDI toggles) keep showing Live's value (P2) and get the same outlines on their dot or button, no ghost (PR C); a toggle with several targets shows its most urgent state (`not_sent`, then `unconfirmed`, then `sending`).

### 4.4 The dropout counter

The owner ruled out status words on the surface (ROZHODNUTÉ on #43, 2026-10-03): space on the tablet is scarce. Instead the surface shows how often the link drops out.

**What a dropout is.**

- The page pings the hub every 100 ms while visible (every second while hidden).
- A **dropout** is one continuous interval in which the page hears nothing from the hub for ≥ 300 ms although a pong is due, or the socket is down. The silence counts from the later of the last message heard and the oldest unanswered ping, so a hidden page that pinged rarely raises no false dropout.
- Each interval counts once, however long it lasts, and a silence that turns into a lost socket stays one dropout until the next hello.
- A silence counts only while the dropout watch's own page tick keeps coming (every 100 ms, independent of any socket, so it goes on between a close and the next hello): a page that was frozen or hidden (300 ms or more since the last tick) cannot tell the link's silence from its own, so the silence starts over at its next tick; a socket lost meanwhile is a dropout from the page's next on-time tick.
- A Live-side delay (Live busy, the network fine) is logged (`link`) but not counted: the counter measures the link.

**What the surface shows (PR C).**

- A small number in the top bar's status cluster (first in it) that grows by one with every dropout, so the engineer sees it rising under the hands: the dropouts since the last tap (`DropoutWatch::counter`).
- While a dropout lasts the number is red; otherwise it is neutral.
- A tap (`pointerdown`) resets it to 0. A dropout that lasts through the tap was counted when it began: the counter shows 0, red until it ends.
- No words, no sound, no blinking: live mixing must not be disturbed.

**Event log.** Each dropout, when it ends, goes to the hub's event log as a `trace` event (start, length, whether the socket was lost, the last ≤ 5 round trips before it), and so does every reset of the counter (`{"ev":"reset","t","count":<what it showed>,"active":<a dropout lasted>}`), both through the flight recorder (§5.2).

The detection is a pure state machine (`behave/link.rs`, `DropoutWatch`, since PR A); its constants (100 ms, 300 ms) are pinned by tests and adjusted from real data once the event log has a service, decided on #43.

## 5. Audit trail

### 5.1 Hub event log

- **Files.** `logs\events-YYYY-MM-DD.jsonl` (UTC date), one JSON object per line, written by one task through a bounded channel. When the channel is full, the event is counted, never blocking: a `dropped` count is logged at the next write.
- **Retention.** Files older than 60 days are deleted at start and daily. A day file is capped at 256 MB (past it: `cap` once, then only `warn`-class events: `sock`, `link`, the notes, anything with an error, and a `trace` that holds a dropout or a counter reset; the flight recorder's other batches stop at the cap like `ping` and `set`, PR C).
- **Records** (`ev` field). Every record carries `ts` (hub UTC ms) and, where it applies, `client`, `peer`, `instance`, `key`, `seq`, `t` (page ms).

| `ev` | Fields |
|---|---|
| `sock` | `open` / `close`, reason |
| `set` | `value`, `final`, `t`, `hub_ms` (its arrival), `offset_ms` and `delay_ms` (`hub_ms − (t + offset)`, its one-way delay), the gap since this client's previous set of that key, `dropped_old` (seq not newer); on the first set of a client's touch of a key (no set of the key from that client before, the previous one `final`, or its page `t` more than 500 ms after the previous one's: the page's clock, so a stalled link does not cut a drag in two) `live_before`: Live's value of the key as the hub knew it then (its subscription cache, what it pushes to pages; null when it held none, PR D). It is the cache, not Live: a re-grab right after a release whose value Live has not pushed back yet (Live stalled) reads the old value, and the timeline then calls the touch `local` |
| `batch` | `instance`, the batch number, `n`, `sent` (key, client, seq, value of each) |
| `applied` | `instance`, the batch number, `n`, `rtt_ms` (Live's round trip), errors, `sent` (key, client, seq) |
| `ack` | per item; an ack of a batch with its number and `rtt_ms` |
| `ping` | every ping (10 a second while the page is visible): `n`, `t`, `hub_ms` (its arrival), the page's latest `rtt` and `rtt_n`, `offset_ms` |
| `trace` | `client`, `peer`, `events`: a batch of the page's flight recorder as it sent it (§5.2: its touches with their starts, the moves of each frame, its writes' turns to unconfirmed and not_sent, round-trip summaries, socket transitions, long frames, visibility, dropouts, the counter's resets and what its full backlog dropped, with the span; before PR E also the writes its socket did not take and ack arrivals) |
| `link` | busy changes with `tick_age_ms`, heartbeat gaps |

- **Clocks.** Page times (`t`) map to hub time through the ping exchange (Cristian): a ping carries the latest pong's round trip and the number `m` of the ping it measured, and `offset = arrival of ping m − (its t + rtt/2)` from the hub's ring of the last 64 pings, so a round trip is paired with the exchange it measured (ping n − 2 or older on a slow link), never with the carrying ping that may itself have been held up; the lowest-RTT exchange of the last minute wins. Each `ping` record carries it as `offset_ms`, and each `set` record the offset of its socket's last ping with the one-way delay it gives: a stall on the way shows as a gap and a delay spike on the moves after it.

### 5.2 Page flight recorder (`diag/trace.rs`)

- **Contents.** A ring of the page's own events, each with `ev` and the page's `t` (no event field is named `ts`: the timeline reads a record's hub `ts` as the first `"ts":` of its line, and a `trace` line's `events` come first):
  - `touch` (`what` down / up / cancel, or `tap` for a mute, solo or stage button, the control's `keys`, the `pointer`); a fader's or pan's taken `down` also says where the touch started (PR D, `diag/trace/moves.rs`): `dt` (the pointer event's own time minus `t`), `c` (its `clientY`; a pan's `clientX`), `travel` (px), `pos` (the position the control showed), `live` (Live's value as the position the control used), `local` (it showed its own position: a finger, a glide, an open write or the post-release hold) and `from` (where the touch starts: `pos` when local, else `live`); positions are 0..1 of the travel (a volume's Live value is p^0.515, a pan's 2p − 1);
  - `mv` (PR D), one per frame that sends from a finger on a fader or pan: `key` (the shown key), `p` (the pointer), `e` (`[dt, c]` of each pointer move merged into the frame, `dt` = the move's own `timeStamp` minus `t`), `r` (the 1:1 finger position: `from` plus the finger's travel since the down, before the touch shaping), `s` (the position the frame sends) and `q` (its set's `seq`); a release that sends the last move adds a last one;
  - `intent` (PR E): a write's turn to `unconfirmed` (`t` = its release + 1 000 ms, when the page drew the amber outline) or `not_sent` (`t` = the resend that kept it back, red), with its `key`, its latest `seq` and the `state`; once per state, a new touch starts over (`store/intent.rs` `Intents::changes`, read on the link's tick). Only the page knows when it drew them;
  - no `send` and no `ack` (PR E; PR D still recorded a write the socket did not take and each ack's arrival): the hub's own `set` record holds each write with the page's `t` and `seq` and the hub writes each `ack`, a frame's `mv` names its set (`q`), and a write the socket did not take stays open in the page and is sent again (a new `set`) or turns `not_sent`;
  - `rtt` (PR D): one summary a second of the pongs' round trips (`t` its first pong, `n`, `min`, `med` the upper middle, `max`); no event per pong (204 812 of them at the service of 2026-10-04, most of the recorder's volume), and none per ping: the hub logs every ping that reaches it with the page's latest round trip;
  - socket transitions, `sock` (`what` open / hello / close / drop / fail, the socket number, the close code or the reason);
  - `frame`: frames longer than 50 ms (not the gap after a visibility change, which is the time hidden);
  - `visibility` (`hidden`);
  - dropouts and the counter's resets; `overflow` (PR E: one per kind dropped since the last batch: `n`, `kinds` (that kind and `n`), `from` and `to`, the page time of the oldest and the newest it dropped; before PR E one note with `n` and every kind's count).
- **Upload** (PR D; the service of 2026-10-04 showed the PR C recorder up to 30 minutes behind the link: its uploads waited for `bufferedAmount == 0`, and the iPad's WebKit reported a ping's bytes there at every 100 ms tick for up to 9 minutes while the page pinged on and sent no set). From the link's 100 ms tick, one `trace` batch of at most 1 000 bytes, its envelope included (one small WebSocket frame: a set sent after it waits for it), goes only when:
  - no set went onto the socket since the previous tick: while a finger moves a fader the recorder waits, and its events go when the fingers rest, so a batch never sits in front of the next frame's set;
  - the socket holds at most 1 KB unsent (a ping or two still leaving never holds it, a backed-up socket does);
  - the rate allows it: each batch waits for the previous one's bytes at 10 KB/s (a full batch 97.7 ms: the next tick, with a margin for a coarse clock), a hard cap far below a slow Wi-Fi link;
  - something waits and it is due: a full batch (events that fill a message with their commas and the envelope) as soon as the cap lets it, smaller amounts every 2 s, and the first batch after a hello, a dropout or a reset as soon as the cap lets it.
  
  Oldest first: after a reconnect the backlog goes first, so an outage is recorded from the page's side.
- **Bounds.** Past 768 KB of unsent events (PR E; PR D's 48 KB lost the moves of any drag longer than about 4 s, since no batch goes while a finger moves a fader) unsent events go by rank, oldest first within a rank: first the `rtt` summaries, long `frame`s and any other kind, then the `mv` records. Essential, never dropped by that bound: touches, dropouts, resets, socket transitions, visibility, the `overflow` notes, the `intent` records, and a touch's first 8 `mv` (what the first-touch diagnosis needs). A 30 s drag of two faders at 60 Hz (3 600 `mv` records, ~610 KB with real-length keys) fits whole: the sizing test runs it through the real recorder, cap and ticks and drops nothing. The price is the lag once the fingers rest: the backlog drains at the cap, about 5 move records a tick, so such a drag reaches the event log in ~75 s, a 2 s drag in ~2 s. Every drop leaves its marker (above), so the forensics read the span as no data. The ring's hard bounds stay as the last guard: 20 000 events and 2 MB of their JSON; past either the oldest unsent event goes, whatever it is.
  - Why not upload during a drag: the batches would sit in front of the sets on exactly the weak link the cap protects, and two faders' moves (~19.5 KB/s) exceed the cap anyway. Streaming the moves during a drag belongs with phase 2, once the sets leave the WebSocket.
- **Delivery.** A batch stays in the ring until the pong of a ping sent after it: the hub reads a socket's messages in order, so that pong proves the hub logged the batch. A socket lost first sends it again after the next hello; the log may then hold an exact duplicate, which the timeline drops.
- **Reload.** A reload loses what was not proved. The page keeps nothing in browser storage, because L1–L4 live in memory too.

### 5.3 Forensics timeline (`tools/forensics/timeline.py`)

- **Input and output.** Stdlib Python, run on the PC or on a copied log: `timeline.py --events <logs dir> --from <local time> --to <local time> [--key …] --out report.html`. It reads the day files that can hold the window and the `hub.out*.log` files beside them (Live busy changes, late heartbeats); an `--out` inside a git checkout is refused. Page times map to the hub's clock with the uploading socket's ping offset.
- **What the report shows** (one self-contained HTML file, inline SVG).
  - Per control: three lines — the page's sends, the hub's arrivals and Live's applied values — with gaps over 100 ms marked inside one touch of a continuous control (a control with a non-final send; a toggle's row has none), each touch its own span (`.claude/rules/forensics.md` has the exact rule; without the page's touches, points up to 10 s apart count as one).
  - The page's RTT and the hub's ping RTT, Live's busy episodes and late heartbeats, the dropouts, the counter's resets, the socket transitions, long frames and visibility.
  - A summary table: the confirmation latency (Live's `applied` minus the page's send on the hub clock: count, p50, p90, p99, worst), the longest dropout, the longest gaps, and jumps over 3 dB between two applied volume values (TouchOSC's `value2db`) with their measured cause: `link` (the arrivals paused while the page kept sending, or a dropout), `live` (the batch's Live round trip or its wait over 100 ms, or Live busy), `page` (the page sent nothing for over 100 ms, or a long frame), `no data` (PR E: none of these, but the recorder dropped the long frames of that time), else `move`.
  - The touches of single volume faders (PR D; a touch naming several keys, the imported MIDI-mapped faders, is left out): where each started (`from`, the page's `live`, `local`) against its first set's `live_before`, its first applied value, the time from the down's own pointer event to the first move and to the first set on the page's clock, and:
    - a **first-touch jump**: the touch's first applied value (Live's result of the first of its own sets Live applied) is more than 1 dB from `live_before`, and more than 1 dB from where the finger alone would have taken Live (`live_before`'s position moved by the finger's travel up to that set's frame, `r` − `from`: a fader that went up while the finger went down counts too); its why: `local` (it started from the fader's own position: a hold, an open write), `stale` (the page's value of Live, `live`, was more than 1 dB from `live_before`), else `other`;
    - **stutter**: a move gap (two consecutive pointer moves over 50 ms apart whose coordinate changed more than 3 px across it: the finger went on while no move came) and a held run (3 or more frames in a row each sending the previous frame's value while the finger's position changed, not at the travel's ends).
  - A touch's records are matched on the page's clock from its down to its lift (else to the key's next down, else the window's end): its pointer's `mv` records of the key, and the key's sets of the socket those records name (a set's `seq` equal to a record's `q`, their `t` within 50 ms).
  - A control's send row is its `send` events (old pages; PR D's only writes the socket did not take, drawn hollow; PR E records none) and the hub's sets at their page `t` (+ offset).
  - **No data (PR E).** Each `overflow` marker with a span is a span of no data of its kind (a grey band in the link lane, a note). A span of `mv` records inside a touch is a hole in the record, not in the drag: a move gap or two frames of a held run across it do not count, and the touch's row shows the span's length and the touch's own hub sets inside it (sets no kept frame names, within 50 ms of the span: they reached the hub; only the finger's moves are missing). A span cannot be pinned to one tablet (the page events carry no tablet id, and a socket number changes at a reconnect), so it applies to every touch it overlaps.
  - A write's `intent` records are marks on its control's send row (amber `unconfirmed`, red `not_sent`).
  - stdout repeats the summary as `name=value` lines, a control only as a hash of its key (the numbers can go on a public ticket).
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
   - PR C: pure tests of `DropoutWatch::counter` / `reset`; in E2E (`counter.spec.ts`, through the impair proxy) an 800 ms stall makes it 1 and red during the stall (a `requestAnimationFrame` sampler; a 400 ms stall is a dropout too, but whether a 100 ms tick sees it lasting depends on the ping's phase), a cut link 2 and red until the reconnect, a tap 0, and the event log then holds the page's records: both dropouts, the reset, and the socket's close the page saw while its link was down, sent up after the reconnect.
7. The event log:
   - a test drags a fader through a stall;
   - it reads the day file: `set` → `batch` → `applied` → `ack` for the last seq, a `trace` with the touch and the dropouts;
   - `timeline.py` renders that window with the stall marked (`forensics.spec.ts`, the harness's `POST /forensics/timeline`); its own unit tests run on synthetic logs.
8. Mutation (the existing gate) covers the setter, the intent store and the dropout watch.
9. PR D, the touch diagnosis and the recorder's load:
   - pure tests: the recorder's events, its gate (a ping in the buffer never holds a batch; a set since the last tick does), 1 000-byte batches, the 10 KB/s cap, the 48 KB backlog dropping the oldest non-essential events per kind, the round-trip summaries; a touch's start and a frame's moves (`moves.rs`); `FaderCtl::press` / `PanCtl::press`;
   - hub: the setter's touch starts (`starts_touch`, 500 ms of the page's clock), `Subs::live_value`, `set_fields`' `live_before`, and on SimLive a touch's first set recording Live's value before it (`tests/setter.rs`);
   - E2E, `recorder.spec.ts`: through the impair proxy's rate limit (24 KB/s, a slow link) two drags back to back with the recorder's frames dropped and then kept: the sets' one-way delay with the recorder is that of without it (median + 20 ms, p90 + 30 ms), at most 2 `trace` records reach the hub between a drag's first and last set (the page events recorded between them fill at least 4 000 bytes, 4 batches of 1 000, so without the gate at least 3 would go inside it), and the second drag's lift reaches the event log within 10 s (about 5 s by design, twice that for a loaded runner's late timers) while the hub logs a trace record at least every 500 ms from the last set to the lift;
   - E2E, `forensics.spec.ts`: the touch start, the moves and `live_before` in the event log, and a touch made while the link stalled and Live moved meanwhile is a `stale` first-touch jump in the timeline;
   - the timeline's unit tests on synthetic logs (`test_timeline_touch.py`).

10. PR E, no lost moves in the recorder:
   - pure tests: a 30 s drag of two faders at 60 Hz (real `Trail` records with p90-length keys, a set every frame, the link's ticks and pongs) keeps all 3 600 `mv` records and drains within 80 s of the lift; the drop order (rank 0, then `mv`, essential never) and the markers with their spans; `Intents::changes` (`unconfirmed` once at the release + 1 s, `not_sent` at its resend, a touch starts over);
   - the timeline's unit tests: a dropped `mv` span is no data, never a move gap or a held run, a gap outside it still counts, the touch's sets inside it are counted, an older note without a span stays a note; the intent marks;
   - E2E, `recorder.spec.ts`: a 24 s drag (1 200 moves) through 24 KB/s, the recorder off then on: every set of the drag has its `mv` in the event log, no `overflow`, no `send`/`ack` record, the drag's page events over the old 48 KB, and the sets' delay with the recorder within that without it (median + 20 ms, p90 + 30 ms); `intent.spec.ts` reads the pan's and the mute's `unconfirmed` and `not_sent` records.

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
4. **PR D — the touch diagnosis and the recorder's load** (after the service of 2026-10-04: "jerky movement at the first touch", and a recorder up to 30 minutes behind).
   - the touch start and the per-frame moves on the page, `live_before` on the hub, the recorder's round-trip summaries, compact sends and acks, its gate, cap and backlog bound (§5.2), the timeline's touches (§5.3), the impair proxy's rate limit;
   - test 9. The faders' behaviour is unchanged: the owner placed the jumps on ordinary single-track faders (#43, 2026-10-04), which these records now diagnose; the imported MIDI-mapped faders writing several tracks are outside PR D (the owner's call on #43).
5. **PR E — no lost moves in the recorder** (PR D's worker found a drag over ~4 s losing its oldest moves, read by the timeline as a false stutter; design comment on #43, 2026-10-04).
   - no `send`/`ack` records, `intent` records, the ranked drop order, a 768 KB backlog and the drop markers with spans (§5.2), the timeline's no-data spans (§5.3);
   - test 10.
6. **Phase 2 (own design note) — datagram transport.** A spike measured on the FOH iPad (Safari 26.6.1: WebTransport datagrams vs a WebRTC data channel), then the datagram path for `set`, with the WebSocket as fallback and the same seq rules.
7. **Phase 3 — verification.** A degraded-link check on the PC with the real iPad, then a service, read through `timeline.py`.

Each PR is deployed and verified on the PC per `deploy-pc.md` before the next starts.

## 8. Not in scope

- A second physical path (wired iPad, an access point at FOH). Recommended to the owner on #43; it is an installation step, not code.
- Changing Live's main-thread load. Phase 1 makes it visible; if the logs show the FohMixer script's own share, that is its own ticket.
