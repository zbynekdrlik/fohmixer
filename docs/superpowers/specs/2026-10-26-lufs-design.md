# Loudness (LUFS) on the mics and on the PA output (#151)

Status: draft for the owner's approval, 2026-10-26. The rules in §2 are the owner's, recorded on #151 on 2026-10-26 (brainstorm answers and two DECIDED comments). The main session's calls are marked *(call)*. Nothing is built before the owner approves this note and mockup `docs/mockups/lufs-v1.html` (one of its two variants).

## 0. Zhrnutie pre vlastníka

**Čo uvidíš na iPade**

- Na každom speváckom mikrofóne v PA ceste: krátkodobú hlasitosť (short-term LUFS) hneď za Trim IN, ako celé číslo, napríklad `−25`.
- Farba podľa cieľa −25 LUFS: zelená do ±3 LU (−28 až −22), žltá do ±6 LU, červená za tým (pod −31 alebo nad −19).
- Keď spevák nespieva, číslo zmizne a ukáže sa sivá pomlčka `—`. Ticho a pauzy nikdy nerozsvietia žltú ani červenú.
- Na výstupe do reprákov (master return): len živé short-term číslo, bez farby a bez priemeru za pieseň.
- Fadre ostávajú presne také vysoké ako dnes. Nad fadre nepribudne nič.

**Odkiaľ to číslo príde**

- Live samotný LUFS nemeria (LOM ho nemá).
- Projekt dodá malé zariadenie Max for Live `FohMixer LUFS`. Počíta hlasitosť podľa normy ITU-R BS.1770, rovnako ako Sonible trueLevel.
- Vloží sa do paralelného chainu v `CORE Effects` vedľa trueLevel. Meria teda presne v tom istom bode a zvuk ani latenciu nemení (do chainu posiela ticho).
- Hub ho číta cez LOM ako bežný parameter zariadenia. Ktoré stopy sa merajú a aký je cieľ, povie značka v Tuneri: `+LUFS:-25` na mikrofóne, `+LUFS` na výstupe.

**Čo sa najprv overí na PC (pred stavbou)**

- Či Live berie zmenu parametra zo zariadenia bez toho, aby plnil Undo a označoval set ako zmenený.
- Ak nie, to isté zariadenie pošle čísla hubu priamo po lokálnej sieti (záložná cesta, §4.4). Značky aj povrch ostávajú rovnaké.
- Číslo nášho zariadenia sa porovná s trueLevel na tej istej piesni a mikrofóne. Musia sedieť do 0,5 LU.

## 1. Goals and non-goals

**Goals**

- G1. Each PA mic track shows its singer's short-term loudness right after the Trim IN, judged against the team's −25 LUFS, so a wrong trim (analog gain moved, mic swapped, weak singer that day) shows on the surface during a normal passage.
- G2. The PA output (the master Live's return to the speakers) shows its live short-term loudness as a plain number, to compare songs' backing tracks while they play.
- G3. Silence and pauses never light a warning.
- G4. The faders keep their height; nothing is added above them (memory rule "fader height first").
- G5. A thin layer over the native LOM: the measurement lives in Live, the hub follows a device parameter as it already follows macros and plug-in parameters, and the tracks are chosen by Tuner tags, as every per-track feature since #68.

**Non-goals**

- Integrated (per-song) loudness, loudness range, true peak, momentary display. The owner ruled the per-song average out for the output ("už hrá ďalšia", #151, 26.10).
- A target or colour on the output (#151, 26.10: the number only).
- Any automatic trim change. fohmixer shows; the engineer turns the Trim IN.
- Replacing Sonible trueLevel. It stays where it is; removing it is the owner's later call.
- LUFS on the stream tracks, the band's instruments, or the master Live's hand mics. A `+LUFS` tag on any track measures it (the mechanism is general), but this note sets up only the PA singers and the PA output.
- Logging loudness as a time series (the event log keeps state changes only, §8).

**Anti-goals** (a failure even if it "works")

- a change of a singer's sound or timing (the meter is never in the signal path, §3.2);
- a yellow or red light while nobody sings;
- a number that disagrees with trueLevel on the same signal by more than 0.5 LU;
- a set that Live marks as changed, or an Undo history filled, by the meter alone;
- any element above the faders, or a change in the faders' travel.

## 2. Rules (the owner's, binding)

- **What is measured on a mic:** short-term loudness (ITU-R BS.1770-4 / EBU R 128 short-term: K-weighted, 3 s window) right after the Trim IN utility on each PA mic track. (#151, FINDING 26.10.)
- **The mic target:** −25 LUFS in a normal passage of a normal song. (FINDING 26.10.)
- **The mic tolerance:** within ±3 LU (−28 to −22) fine; beyond that yellow; beyond ±6 LU (below −31 or above −19) red. (DECIDED 26.10, owner's answer "1".)
- **The PA output:** the live short-term LUFS number only, no per-song average, no target colour. (DECIDED 26.10.)

## 3. What is measured, where

### 3.1 The points

| Strip | Live set | Track | Point | Shown |
|---|---|---|---|---|
| each PA singer (`Vox 1`…) | band | the PA mic track | after Trim IN: a parallel chain of its `CORE Effects` rack, beside trueLevel | `Voice` (§3.3), coloured against `+LUFS:-25` |
| PA output | master | the return to the speakers | the return's device chain, before its mixer | `Short-term`, plain |

**Seen on the PC earlier (to confirm at rollout, #151 comment of 26.10):** each PA mic track has a `CORE Effects` rack with a Sonible trueLevel in a parallel chain after the Trim IN. The team reads −25 there today. Placing our meter in a sibling chain of the same rack measures exactly the signal trueLevel measures *(call)*.

### 3.2 Never in the signal path

- The device's audio output is silence (the `plugout~` is fed zeros), so its chain adds nothing to the rack's sum. *(call)*
- The chain stays active (never muted by its activator): a deactivated chain may stop processing its devices. UNVERIFIED for Live 12; the silent output makes the question moot.
- The device reports no latency (a Max for Live device that does not set `latency` reports 0, UNVERIFIED for this exact patch: checked at rollout by reading `latency_in_samples` of the rack before and after the insertion, which must not change).
- Memory rule "latency claims need measurement": the rollout check above is a read of Live's reported latency, not a timing claim. The path's audio is untouched by construction (the device adds silence to a sum).

### 3.3 The two readings the device makes

Every 100 ms (EBU Tech 3341's 10 Hz minimum), on the stereo input (L and R, each weight 1.0, as BS.1770 sums a stereo track; a centred mono mic on a stereo track therefore reads as trueLevel reads it on that track):

1. **K-weighting:** BS.1770's two biquads (the high-shelf pre-filter and the RLB high-pass), coefficients derived for the running sample rate from the standard's analog prototypes (the published 48 kHz values as the test case).
2. **Sub-blocks:** the mean square of each 100 ms sub-block, summed over channels.
3. **Short-term** `S`: −0.691 + 10·log10 of the mean of the last 30 sub-blocks (3 s). No gating (BS.1770 short-term has none). Floor −70 (shown `—`).
4. **Voice** `V` *(call; the gate rule G3 asks for)*:
   - A sub-block is **voiced** when the momentary loudness of the 400 ms ending with it (its 4 sub-blocks) is at least **−40 LUFS**, that is 15 LU under the target and 9 LU under the red line.
   - `V` is the loudness of the mean of the **voiced** sub-blocks among the last 30 (3 s).
   - `V` is valid only when at least **10** of those 30 are voiced (1 s of voice in 3 s). Otherwise `V` = −70 (silent).

   Why this rule:
   - A pause shorter than the window does not drag the reading down: unvoiced sub-blocks are left out, so a phrase end never turns yellow on its way to silence (plain `S` falls through −28, −31 for 3 s after every phrase).
   - A voice that really is too quiet (−34) still reads red: it is above the −40 gate.
   - 1 s of voice in 3 s keeps a single breath or a cough from lighting a colour.
   - Stage bleed louder than −40 LUFS after the trim would read as a voice. Whether it does is open question Q1.
5. **Parameters written** only when the value rounded to 0.1 changed (fewer LOM events): `Short-term` and `Voice`, each a float −70.0…0.0 LUFS.

The master strip shows `S`, a mic strip shows `V` *(call)*: the output's number is the standard short-term the owner asked for, silence there between songs reads low (−50) or `—`, which is the truth for the room.

## 4. The data source

### 4.1 The options checked

**(a) A Max for Live device the project ships, its readings as device parameters followed through the LOM.** *Chosen, gated by probe P1 (§4.3).*

- Evidence it fits:
  - Max for Live is in use on both Lives' PC: the old autotune chain ran four Max for Live devices (autotune note §1).
  - The hub already finds devices inside racks by a bounded walk and follows their parameters' `value` by `$ref` with its own subscriber (`router/autotune.rs`: the `Latencies` rack's macro and the key devices' Key/Scale; `rules::autotune::find`). LUFS is the same pattern: a find, a follow, a state to the pages.
  - A device parameter is a native LOM object: the script, its protocol and its listener machinery need no change (spec P4). The script's subscription flush already coalesces per tick (~30 Hz; SimLive load: 60 metered strips at 30 Hz, `live-script.md`); about ten parameters at 10 Hz are far below that.
  - The measurement is ours: reviewable source, BS.1770 test vectors, the same code on every track.
- Risks (UNVERIFIED until P1):
  - A parameter changed from inside the device may add Undo steps or mark the set changed. The Live set is saved by us after approved changes (memory "save the Live set yourself"); a set always dirty would hide real changes.
  - A parameter with visibility "Automated and Stored" could be recorded as automation; "Stored Only" might avoid it, "Hidden" leaves the LOM. Which one Live 12 exposes in `Device.parameters` with listeners firing is part of P1.

**(b) Reading Sonible trueLevel's meter.** *Rejected.*

- Plug-ins expose their controls as parameters; their meters are outputs. VST3 has read-only output parameters, but whether trueLevel declares its loudness readings as such, and whether Live lists them in `Device.parameters`, is UNVERIFIED: nothing in the repo or its probes touches trueLevel, and this note had no PC access.
- Even if one existed, its value range, scaling and the mode the team set on each instance (momentary, short-term, its own gate) are unknown, and differ per instance.
- Configure mode (`program spec §4.2`) adds automatable controls, not meters.
- It ties the feature to a third-party binary's update policy.
- Worth one look during P1 (two minutes in Configure mode on one trueLevel). If it lists a short-term reading, this note is revised before building; otherwise (a) stands.

**(c) An audio tap from the virtual audio routing, measured by the hub.** *Rejected.*

- It needs one routed channel per mic plus the output: changes to the routing the band depends on.
- The tap point would be wherever the routing can reach, not "right after the Trim IN".
- DSP in the hub is a second audio path on the PC, against the thin layer.

**(d) Live's own meters (`output_meter_level`).** *Rejected:* post-fader peak-ish levels, not K-weighted loudness; already on every strip.

### 4.2 The device: `FohMixer LUFS`

- An audio effect device, file `FohMixer LUFS.amxd`, name `FohMixer LUFS` (the hub finds it by this name, §5.2).
- DSP in one `gen~` (the BS.1770 filters, the sub-block sums, the window ring buffers); the rest is plain Max: two `live.numbox` parameters `Short-term` and `Voice` (float, −70…0, unit "LUFS", the visibility P1 settles), driven 10×/s.
- No controls. Its face shows the two numbers, so it can be read in Live's device view too.
- **Source in the repo, binary built on the PC** *(call)*: `live-device/FohMixer-LUFS/` holds the patch (`.maxpat` JSON) and the `gen~` code (`.genexpr`) as text. The `.amxd` is built (frozen) on the PC and never committed: a frozen device can embed the author's file paths, which hold the Windows user name (public-repo hygiene), and the denylist scan skips binary files.
- `live-device/FohMixer-LUFS/version.txt` follows the workspace version; the device shows it on its face, and the hub logs it from the device's name or a third parameter `Version` *(call: a parameter, read once)*.

### 4.3 Probe P1 on the PC (before PR 2)

On the PC, with the owner's yes for the session, the band Live stopped, on a copy of the set (never the event set):

1. Insert a draft device (the patch from this note) in one PA mic's `CORE Effects` sibling chain.
2. Read through the LOM (`tools/live-probe`): the device's `name`, `class_name`, its parameters' `name`, `value`, `min`, `max`; subscribe `value` and count events for 30 s of a played song: expect about 10/s each.
3. Check Live's Undo menu and the set's changed state after 60 s of playing: **pass** = no Undo entries from the meter, the set not marked changed.
4. Try both visibilities ("Automated and Stored", "Stored Only"); keep the first that passes 2 and 3.
5. Read the rack's `latency_in_samples` before and after the insertion: equal.
6. Look at one trueLevel in Configure mode (option (b)).

Results go on #151 without host names, IPs or user paths.

### 4.4 Fallback if P1 fails: the same device sends over localhost

- The device sends `Short-term` and `Voice` 10×/s by UDP to the hub on `127.0.0.1` (`udpsend`), with its own LOM `id` and its track's `id` (read once with `live.path this_device` / `canonical_parent`).
- The hub matches the track id to the `+LUFS` markers it already knows. Tags, protocol to the pages and the surface stay as in §5–§6.
- This is the program spec's first candidate (§4.3), but it adds a listener to the hub; it is only built if P1 fails.

## 5. How the hub gets it

### 5.1 The tag

- `+LUFS:<target>` on a track's Tuner marker: measured, judged against `<target>` LUFS, an integer −40…−10 (`+LUFS:-25`).
- `+LUFS` alone: measured, the number only (the PA output).
- Problems (`markers::parse`, the existing problem list): a second `+LUFS` (`DuplicateLufs`), a target that is not an integer in range (`BadLufs`).
- A Tuner of only `+LUFS` makes a strip like any marker (it is a strip feature, not a link like `+MUTE`).
- A strip from the frame file (not a marker) may carry the same field in `layout.json` (`"lufs": {"target": -25}` or `"lufs": {}`), so the PA output works whether its strip is a marker or a frame strip.
- The tolerances (±3, ±6 LU) are the owner's rule and constants in code, not tag arguments *(call)*.

### 5.2 The router's half (`router/lufs.rs`, `LUFS_CLIENT`)

The AUTOTUNE pattern, smaller:

- For each served strip with `lufs`, a bounded walk of its track: `devices`, and inside each rack its `chains` → `devices`, two rack levels deep. Each device's `name` is compared with `FohMixer LUFS` (and `class_name` = `MxDeviceAudioEffect`, the Max audio effect class: UNVERIFIED name, read in P1).
- Found once: its `Voice` (mic) or `Short-term` (output) parameter's `value` is followed by `$ref` with the hub's own subscriber `LUFS_CLIENT` (the next free id after `SONGMAP_CLIENT` = `ClientId::MAX - 4`; the builder confirms).
- Not found: the strip's problem `NoLufsMeter`. Found twice: `TwoLufsMeters`, nothing followed (which one would be a guess).
- Walked again on: connect, a track-list change (the marker keeper's lists), a `devices` value of that track (a device inserted or removed), a failed follow (a stale ref). Retries: the connect-time bound (`retries.rs`), only transient failures.
- A disconnect drops the instance's refs and values: the strips show `—` with the offline look they already have.
- Never writes anything to Live.

### 5.3 To the pages

- `ServerMsg::Lufs { items: [{instance, label, value}] }`, `value` in tenths of LU as an integer, or none (silent / no meter / offline) *(call: tenths kept for the log and a later detail view; the strip shows the integer)*.
- Paced with the meters (`meter_pace`, #51): at most once per the page's meter period, the latest value kept; an idle page with `meter_ms` 0 gets none until it wakes, then the current values at once.
- `/api/status` `lufs`: per strip the device found (ref, version), the last value, its state, the problem.

## 6. What the surface shows

### 6.1 The states (pure, `fohmixer_proto::lufs::judge`, shared by the UI and the hub's log)

The value shown is the integer `round(v)` (half away from zero). The colour is judged on that same integer, so the number and its colour never disagree *(call)*:

| State | Mic (`+LUFS:-25`) | Output (`+LUFS`) |
|---|---|---|
| `fine` (green) | −28…−22 (|d| ≤ 3) | — |
| `warn` (yellow) | −31…−29, −21…−19 (3 < |d| ≤ 6) | — |
| `far` (red) | ≤ −32, ≥ −18 (|d| > 6) | — |
| `plain` (neutral) | — | any value > −70 |
| `silent` (`—`, grey) | `V` = −70: under 1 s of voice in 3 s | `S` ≤ −70 |
| `none` (`—`, dim) | offline, no meter, or no value yet | same |

A strip in `none` because the device is missing also shows the existing problem `!` in its corner, with `no LUFS meter` in the problem list.

### 6.2 Where on the strip (mockup v1, two variants for the owner)

Both keep the faders' travel exactly as today: neither adds a row, both use space the strip already has. The mockup measures the fader track of a strip with LUFS and one without and prints both numbers.

- **Variant A — in the readout line.** On a `+LUFS` strip the readout under the name button shows the loudness instead of Live's dB, in a pill of the state's colour. While a finger (or a glide) holds the fader, the readout shows Live's dB in accent as it does today. Live's dB stays in the channel detail.
- **Variant B — a pill at the top of the meter.** Live's dB stays in the readout. The loudness is a pill over the top of the scale and meter columns, under the clip light, left of the fader's groove. It takes no touch (the fader and the clip light work through and around it). It covers the meter's top 16 px and the scale's `+6` mark.

On the phones (strips down to 64 px) both variants keep their place. Its font comes from the strip width, as the readout's does, and nothing is measured at runtime (#21). In A the number has three characters, not dB's five, so it stays 12 px from a 64 px strip up. In B it is 12 px on the iPad and 10 px at 64 px. The mockup checks that every reading fits its box on all six screens.

### 6.3 Texts (English, #141)

- On the strip: the number only (`−25`, `−18`), `—` when silent or missing. No unit on the strip (no room at 64 px); the legend says LUFS.
- In the problem list: `no LUFS meter`, `two LUFS meters`, `bad LUFS tag`.
- The tag manual (`znacky.html`): `+LUFS:<target>` and `+LUFS`, with the states table.

## 7. Protocol and data additions (summary)

- `fohmixer_proto::markers`: `+LUFS`, `+LUFS:<n>`; problems `DuplicateLufs`, `BadLufs`.
- `fohmixer_proto::layout::Strip`: `lufs: Option<LufsSpec { target: Option<i8> }>` (`deny_unknown_fields` stays).
- `fohmixer_proto::lufs`: `judge(value_tenths, target) -> LufsState` and the display integer; the constants `FINE_LU` 3, `FAR_LU` 6, `SILENT` −70.
- `StripMark` (or the problem list): `NoLufsMeter`, `TwoLufsMeters`.
- `ServerMsg::Lufs`, paced as meters.
- No change to the Live script or its protocol (§4.1 (a)); in the fallback (§4.4) a hub UDP listener on localhost.

## 8. Logging

- `lufs` records (info): a device found or lost (strip, instance, ref, version), a problem, a walk retried or given up.
- A mic strip's state change (`fine`/`warn`/`far`/`silent`) as one `lufs` record with the value, at most one per strip per 3 s (the window's length) *(call)*, so the forensics timeline (#43) shows "Vox 2 far −33 from 19:04:12" next to the transport. No time series of values.

## 9. Tests

- **The reference DSP (Rust, CI):** a small pure module with the BS.1770 filters, sub-blocks, `S` and `V`, tested against EBU Tech 3341's cases that apply to short-term (a stereo 1 kHz sine at −23 dBFS reads −23.0 ±0.1; the 20 dB step cases; the sample-rate variants 44.1/48/96 kHz), and the gate: a 2 s phrase, a 2 s pause (V stays at the phrase's value, S falls), 0.5 s of voice in 3 s (silent), a voice at −34 (red, not silent), a 1 s breath at −38 (silent).
- **The device against the reference (PC, an L check):** the test signals rendered as WAVs, played through the device in Live; its parameters agree with the reference within 0.1 LU. The `.genexpr` text and the Rust module share one written-out coefficient table, kept equal by a CI check *(call)*.
- **proto:** the tag (`+LUFS`, `+LUFS:-25`, `+LUFS:-5` bad, `+LUFS:x` bad, two tags), the composition into `Strip.lufs`, `judge` on every boundary (−22 fine, −21 warn, −19 warn, −18 far, −28 fine, −29 warn, −31 warn, −32 far, −70 silent, rounding of −21.5 and −22.4), the frame file's `lufs` field.
- **hub (SimLive):** SimLive gains a Max device with `Short-term`/`Voice` parameters it can drive. A walk finds it two racks deep; a device inserted before it re-walks and follows the new ref; a missing and a doubled device give their problems; a disconnect clears; pacing and an idle page; `/api/status`. The mutation gate covers `judge`, the walk's matching and the pacing.
- **E2E (Playwright, Chromium and WebKit iPad, the phone sizes):** a SimLive `Voice` of −25, −21, −33, −70 shows `−25` green, `−21` yellow, `−33` red, `—`; the output shows its plain number; (variant A) holding the fader shows dB and lifting shows LUFS again; the fader track's height on a `+LUFS` strip equals a plain strip's; zero console errors.

## 10. Rollout on the PC

1. **P1** (§4.3) on a set copy; results on #151. Decide (a) or the fallback.
2. **PR 1: proto + hub + SimLive + the reference DSP**, nothing on the surface yet (`/api/status` shows the readings).
3. **The device:** the patch built and frozen on the PC; `.amxd` installed into both Live users' User Libraries by `Install-Fohmixer.ps1` (the installer already checks Live's User Library for the script).
4. **The set work (ours, memory "restart Live and do set work yourself", "save the Live set yourself"):** in the band set, a new chain `LUFS` in each PA singer's `CORE Effects` beside trueLevel, the device in it; `+LUFS:-25` on each PA singer's Tuner. In the master set, the device on the speakers' return, `+LUFS` on its Tuner (or the frame strip's `lufs`). Each set saved with Ctrl+S. Not during an owner-declared event.
5. **Check:** each rack's `latency_in_samples` unchanged; `/api/status` lists every strip's device; a played song: our `V` vs trueLevel's short-term on two singers, within 0.5 LU (recorded on #151); Live's CPU meter before and after (all devices together).
6. **PR 2: the surface**, the variant the owner approved; deployed and verified on the iPad on real Wi-Fi (memory "real-device acceptance").
7. **Acceptance with the owner:** a song with a mic at the right trim (green), a trim moved 5 dB (yellow) and 8 dB (red), the singer silent (`—`), the output's number while a song plays.

## 11. Open questions for the owner

- **Q1. Bleed on the vocal mics.** With the band playing and the singer silent, how loud is the stage in a vocal mic after its trim? If it can reach −40 LUFS, the gate (−40) should go lower than the bleed would read as a voice, e.g. −36. A one-song check during P1 answers it; the owner only confirms which level counts as "not singing".
- **Q2. The variant:** A (the number replaces Live's dB in the readout; dB while the fader is held) or B (a pill over the meter's top; dB stays).
- **Q3. Which tracks:** only the PA singers and the speakers' return, or also the master Live's hand mics (speech) with the same −25 target?
