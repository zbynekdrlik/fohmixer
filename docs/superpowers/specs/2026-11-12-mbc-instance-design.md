# The MBC Ableton in fohmixer (#184)

Status: draft for the owner's approval, 2026-11-12. Mockup: `docs/mockups/mbc-layout-v1.html`.
Public-repo hygiene: this text holds no host names, addresses, Windows user names, people's
or real track names. The strip names below (`Hand 1`, `Moderator 1`, `Ambience`) are
invented.

## 0. Zhrnutie pre vlastníka

- MBC Ableton (samostatný mix do streamu) sa ovláda z toho istého fohmixera (rovnaký program,
  rovnaký vzhľad). Fohmixer pobeží aj priamo na MBC PC, takže MBC funguje aj pri vypnutom FOH PC.
- MBC operátor má vlastný tablet a vlastný PIN. Po prihlásení vidí len MBC rozloženie: pásy MBC,
  veľké číslo LUFS streamu a pri každom zdroji zarovnanie oneskorenia. Do FOH vecí sa nedostane,
  a to ani obídením stránky, lebo to stráži hub.
- FOH zvukár so svojím PIN-om vidí všetko ako doteraz. Medzi rozloženiami má navyše MBC.
- MBC tablet sa pripája na fohmixer na MBC PC. Fohmixer na FOH PC sa k nemu pripája cez sieť
  s heslom, aby FOH zvukár videl MBC na svojom tablete.
- TouchOSC a jeho skript v MBC Abletone zrušíme až vtedy, keď fohmixer na MBC preukázateľne
  funguje počas bohoslužby. Stream nikdy neostane bez ovládania.
- Schvaľuješ: pridanie značiek (Tuner) a zariadenia FohMixer LUFS do MBC setu; vzhľad MBC
  stránky (mockup); zmazanie TouchOSC skriptu a jeho 10 GB logov na MBC PC.

## 1. Intent and why

The MBC Ableton is the stream mix: levels and balance of the main groups tuned
independently of the PA, latency alignment of different sources (ambience mics, presenters,
the band feed), and the stream master at -14 LUFS. Today it is run on its own tablet through
TouchOSC and a modified AbletonOSC script; the loudness is read on the MBC PC's screen and the
alignment is edited there too. The owner wants one fohmixer that does all of it (#184
ROZHODNUTÉ, 2026-11-12):

- the MBC Ableton becomes another instance of the same fohmixer, with its own layout;
- the stream operator or the MBC operator, whoever is available, uses their own tablet;
- a PIN login with roles: an MBC login reaches only MBC things, the FOH engineer's login on
  the FOH tablet reaches FOH and MBC;
- TouchOSC is retired: no longer used, and no longer present on the MBC PC.

## 2. Goals

- G1. A third Live instance `mbc`, on the MBC PC, served by a hub on the MBC PC (works with the
  FOH PC off, owner 2026-11-12) and reachable from the FOH hub.
- G2. Parity with today's MBC TouchOSC page: 12 strips with volume (fohmixer's volume law,
  double tap to 0 dB), pan, mute, meter, dB readout. Today's battery widget, REFRESH ALL and
  the empty second page are not reproduced (the iPad shows its own battery; fohmixer needs
  no refresh, #58).
- G3. More than parity, as the owner asked: the stream's loudness on the tablet (the
  FohMixer LUFS device, target -14 LUFS shown), and the per-source latency alignment on the
  tablet (in the channel detail, not on the main page).
- G4. Roles enforced by the hub: an `mbc` token can read and write only instance `mbc`, only
  see the MBC layout, and reach none of the FOH-only endpoints.
- G5. A rollout in which the stream never loses control, then TouchOSC removed.

## 3. Non-goals

- A separate app or a different look for MBC (the owner: "nechcem samostatný nový fohmixer").
  The MBC PC runs a second copy of the same hub binary and app, not a second product.
- Sends, EQ screens, plug-in windows (#71) or the mastering chain's chain choice on the MBC
  page in this round. The generic LOM reaches them; they come by prompt later (D4).
- Reading Sonible true:level's numbers (not Live parameters; §6.2).
- Touching the MBC set's audio path beyond inserting markers and a metering chain (§7).

## 4. Anti-goals (a failure even if it technically works)

- An MBC operator can change anything of the PA (band, master, rules, AbleSet, IEM mixer,
  Presenter, Stream Deck) — by the UI or by calling the API directly.
- A fohmixer test steals the TouchOSC tablet's updates during the parallel phase (AbletonOSCMBC
  pushes to whoever sent the last packet: fohmixer must never send OSC to it).
- The stream has no working control at any moment of the rollout.
- An alignment change makes an audible jump in the stream while it is live without the
  operator knowing it will (§6.3).
- The MBC page shows the FOH page's rail (STAGE, REVERB, AUTOTUNE, TALKBACK, TECHALERT) or
  the FOH Lives' status in red (#178's lesson).

## 5. Constraints

- C1. The FohMixer script binds 127.0.0.1 only and has no auth (program spec §2.3, I-rules);
  this stays true on the MBC PC.
- C2. The FohMixer LUFS device sends to `127.0.0.1:39110` (fixed in the patch, #151 §4.4); it
  stays fixed.
- C3. The two PCs share one LAN segment; the MBC PC's firewall is off. LAN traffic is not
  encrypted today (the hub's plain HTTP port 8480 is LAN-trusted); the new link adds a shared
  secret, not TLS.
- C4. MBC's Live 12.2 has one control surface today (slot 1, AbletonOSCMBC); slots 2–7 are
  free.
- C5. MBC's Live cannot write its User Library (it points at another Windows user's folder);
  today's script is therefore in Live's built-in `MIDI Remote Scripts` folder.
- C6. Strips come from Tuner markers in the set (D16); a strip needs a Tuner on its track.
  Measuring devices output silence and sit only in a parallel chain of a rack (#151).
- C7. Never save a Live set the session did not change on purpose; a set change is approved
  by the owner first, saved with a dated backup of the `.als` beside it.
- C8. The FOH hub keeps working with the MBC PC off: instance `mbc` shows `not running` on MBC
  strips only (the #178 words), nothing else changes.
- C9. MBC control keeps working with the FOH PC off (owner, #184, 2026-11-12: "Asi by som bol
  radsej keby vedel mbc bezat aj bez fohabl pc").

## 6. Design

### 6.1 Transport — Architektúra

C9 rules out any design where the MBC tablet depends on the FOH PC. Candidates, read in the
code:

- **(a) The script listens on the LAN with a password.** Rejected: it breaks C1 (an
  unauthenticated-by-default LOM proxy on the LAN, auth in Live's Python on Live's thread),
  the LUFS device (C2) would still need a path, and the MBC tablet would still need a hub.
- **(c) A thin relay on the MBC PC, the hub only on the FOH PC.** Rejected by C9: with the
  FOH PC off the MBC tablet has nothing to talk to.
- **(b) A hub on the MBC PC too, the FOH hub reaching MBC through it.** **Chosen.** The MBC
  PC runs the same `fohmixer-hub` binary and PWA with one instance (`mbc`), its own PIN store,
  the MBC frame and its own LUFS listener on 127.0.0.1:39110. The MBC tablet opens that hub
  (LAN; remote access is out of this round). The FOH hub's instance `mbc` is a remote
  instance: it dials the MBC hub's **instance link**, an authenticated WebSocket that passes
  the MBC Live's script frames and LUFS readings through. Why: C1, C2 and C9 hold; one binary,
  one app (the owner's "one fohmixer"); the hub's existing stack (tokio, axum,
  tokio-tungstenite, `live/client.rs` framing) and install pattern (`scripts/fohmixer-pc/`).

The MBC hub owns the MBC Live's script connection. Both hubs' writes reach the one Live; the
MBC hub forwards the FOH hub's commands and fans the Live's replies and listener updates out
to its own clients and the link, so a fader moved on either tablet shows on both.

The instance link's contract:

- MBC hub config: `[link] listen = "<LAN address>:39201"`, `instance = "mbc"`. It binds the
  LAN address it is given, never 0.0.0.0.
- Auth: the FOH hub opens the WebSocket with `Authorization: Bearer <HS256 JWT>` signed by a
  link secret (32 random bytes, generated once, DPAPI-sealed on both PCs like the other
  secrets). `sub: "hub"`, `exp` 60 s, a new token per connection. Anything else: 401, logged.
- Frames: the script's JSON frames pass through, both ways, with the MBC hub's request-id
  mapping so the FOH hub's ids never collide with its own. LUFS readings are sent as
  `{"event":"lufs","osc":"<base64>"}` and fed to the FOH hub's existing LUFS parser, tagged
  instance `mbc`.
- One FOH hub connection at a time; a second is refused.
- Liveness: the link reports the MBC Live offline exactly as a local Live would; with the MBC
  PC off the FOH hub shows `mbc` not running (C8). The hub's backoff (250 ms → 2 s) applies.
- Logs: every connect, refusal and drop, peer address in the PCs' logs only.

FOH hub config: `[[instances]] name = "mbc", link = "ws://<MBC LAN address>:39201"`. `port`
and `link` are exclusive.

Roles on two hubs: the MBC hub has an `mbc` PIN and an `engineer` PIN (the same engineer PIN as
FOH, set on both by the install); the FOH hub has both too, so an MBC operator may also log in
on the FOH hub and still sees only MBC (§6.4).

### 6.2 The MBC set: markers and loudness (a set change, owner approval)

- **Strips:** a Tuner marker on each of the 11 tracks of today's page (12 strips: one
  track has two strips today, `Hand 1` and `Hand 1 sync`), label = today's TouchOSC strip
  name, `+G:MBC` in today's left-to-right order. Group tracks get the same. No `+PIN`.
- **Loudness:** one `FohMixer LUFS` device in a new parallel chain of the stream group's
  existing metering rack (the rack that already holds the loudness plug-in in parallel to a
  dry chain). The track's Tuner gets the tag that shows the LUFS reading on its strip
  (`+LUFS`, as on the FOH sets, #151). It reads the stream master's loudness at the point
  the operator reads it today.
- The insert is done by the session through the LOM (`insert_device`, as the Tuners were on
  FOH), then the set is saved with a dated `.als` backup beside it. Before and after: the
  rack's `latency_in_samples` is read; it must not change (the device reports 0, #151).

### 6.3 The MBC page

See `mbc-layout-v1.html`. In fohmixer's look (dark, Barlow, the control column in the middle):

- One page of 12 strips, 6 on each side of the column: name button (Live's colour), dB
  readout, fader with dB scale and meter, mute, pan.
- The stream's loudness is a large readout in the control column (`STREAM LUFS`, short-term,
  `cieľ −14`), not on a strip: the stream group is not one of today's 12 strips. Its source is
  the `FohMixer LUFS` device of §6.2, matched by a column binding in the MBC frame (builder:
  a frame entry naming the instance and the device's track, no strip made).
- Optional 13th strip `STREAM` (the stream group's own fader) to set the overall stream level
  against -14: an owner question (§9), not in today's TouchOSC.
- The column foot says only `MBC on` / `MBC not running` (no FOH Lives).
- The column for an `mbc` login: the `MBC` page tab, the version, logout. No STREAM DECK,
  ABLESET, IEM MIXER, PRESENTER, no FOH layouts, no rail buttons.
- **Alignment** in the channel detail (hold ☰, F27): a section `ZAROVNANIE` with
  - `Delay` — the Delay device's time in ms (shown in ms, written as Live's seconds), ±1 ms
    and ±10 ms steppers and a drag;
  - `Jemne` — Align Delay's ms, ±0.1 ms steppers and a drag;
  - for a source with two chains: a two-way switch with the chain names (e.g. `Ethernet` /
    `Wifi`), the rack's chain selector;
  - the total shown (`spolu 470.0 ms`).
  Found per strip by device class in the strip track's chain: the first rack holding a Delay
  device; in it the Delay and the Align Delay device; the rack's chain selector when it has
  more than one chain. None found: the section is not shown.
- The FOH engineer: `MBC` appears among LAYOUTS in the column; it opens the same page.

### 6.4 Roles

- PIN store: `engineer` and `mbc` entries (argon2id each). `fohmixer-hub pin set-mbc` (as
  the band user, as set-engineer). The two PINs must differ (set refuses an equal one).
- Login: one PIN field; the hub checks it against both hashes (the login guard counts every
  failure once) and issues a token with `sub: "engineer"` or `sub: "mbc"`.
- Enforcement in the hub, on every path (HTTP and WebSocket):
  - `mbc`: commands, subscriptions and setter writes only for instance `mbc`; layouts: only
    the MBC frame; `/api/status` trimmed to the `mbc` instance and the version; every other
    endpoint (rules, AbleSet, IEM, Presenter, Stream Deck, deck, plug-in windows, admin)
    403, logged.
  - `engineer`: everything, as today, plus `mbc`.
- Existing engineer tokens stay valid (`sub: "engineer"`).

### 6.5 The FohMixer script on the MBC PC

- Copied into Live's built-in `MIDI Remote Scripts` folder (C5), as the working AbletonOSCMBC
  is; `Config.py`: `INSTANCE = "mbc"`, `PORT = 39103`. Selected in a free control-surface
  slot (slot 2) while AbletonOSCMBC keeps slot 1.
- A Live update may replace that folder: the MBC hub reports `script missing` when it cannot
  reach the script for 60 s with Live running, and the install script re-copies it.

### 6.6 Rollout (the stream never without control)

1. Build and CI on SimLive (two hubs and the instance link, roles, alignment section, MBC page). Merge, deploy the hub.
2. Install the MBC hub and the script on the MBC PC; select slot 2. TouchOSC keeps slot 1 and
   keeps working (fohmixer sends nothing to AbletonOSCMBC).
3. Owner approves §6.2; markers and the LUFS chain inserted, set saved with a backup.
4. A service with both: the operator on fohmixer, TouchOSC still there as fallback. The
   event log is read afterwards (touches, drops, LUFS readings).
5. Owner confirms; then retire: AbletonOSCMBC out of slot 1 and its folders removed, its
   logs (about 6.5 GB, and 4 GB of old copies) deleted — each deletion with the owner's yes
   (gated). TouchOSC on the operators' tablet is removed by whoever holds that tablet.

## 7. What is left to the builder

- Exact ports (39103, 39201 proposed), names of config keys, the MBC hub's install script.
- The alignment section's visual details within the mockup.
- How the LUFS frame is fed into `lufs_udp.rs` (a shared parse function, no copy).
- Tests: instance-link auth and passthrough, a fader moved on either hub shown on both (unit + a two-hub test), roles on every endpoint
  (a table-driven test over the router), the MBC page and the alignment section in
  Playwright on SimLive (Chromium + WebKit iPad), console clean.

## 8. Calls made by the main session (check while reading)

- A hub on the MBC PC with an instance link to the FOH hub (b), after the owner chose C9 (the
  earlier thin relay (c) is dropped).
- The script in Live's built-in folder rather than fixing MBC's User Library path (fixing it
  changes where MBC's presets and packs are looked up).
- Alignment in the channel detail, not on the main page.
- No `+PIN` strips, no rail on the MBC page.

## 9. Builder check (2026-11-12, done by the main session reading as the builder)

Answered in the document now: where the LUFS reading shows (the column, §6.3); the FOH rules
(STAGE/REVERB/AUTOTUNE AUT) act only on tagged tracks, and the MBC set gets none of their
tags, so they never touch MBC.

Minor, settled (my call):
- The `mbc` role works on the LAN and through the internet path like the engineer's; who may
  pass Cloudflare Access from outside stays the owner's existing list.
- One login page for both roles; the role follows the PIN.
- The alignment writes go through the pages' setter (one batch in flight), like every write.

The owner's:
1. Is the FOH PC always on while the stream runs? The hub lives there; with it off, the MBC
   tablet has no control (today's TouchOSC talks to the MBC PC directly).
2. A `STREAM` fader (the whole stream's level) on the MBC page, yes or no.
3. May the operator change the alignment during a live stream (an audible skip), or only
   before it (fohmixer could lock it while the stream group plays)?
4. Approval of the set change (§6.2): Tuner markers on the 11 tracks and the LUFS device in
   the stream group's metering rack, saved with a backup.
5. Approval of the look (`mbc-layout-v1.html`).
