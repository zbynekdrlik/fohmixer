# Loudness (LUFS) on the mics and on the PA output (#151)

Status: draft for the owner's approval, 2026-10-26. The rules in §2 are the owner's, recorded on #151 on 2026-10-26 (brainstorm answers and two DECIDED comments). The main session's calls are marked *(call)*. Nothing is built before the owner approves this note and mockup `docs/mockups/lufs-v7.html`. Mockup v7 is v6 with the owner's ruling on it (#151, 26.10): the muted strip recedes and its name button carries today's small red text mark, its word `MUTED`; no RET mark on any strip (§6.4). Mockup v6 is v5 with the owner's ruling on its muted look (#151, 26.10): a muted strip recedes as today's surface does, never lights up, and one colour per strip, the Live track's own (§6.4). Mockup v5 is v4 with the owner's ruling on it (#151, 26.10): the SEL key approved (variant B dropped), and the LUFS as quiet text, never a shape, nothing while the singer is silent (§6.2); and the name button calm, loud only while muted (§6.4). Mockup v4 is v3 (the owner: "better", #151, 26.10) with the channel detail's ☰ moved out of the readout line under MUTE to a key at the strip's bottom left, in two pictogram candidates (§6.3). Mockup v3 is v2 (`lufs-v2.html`) without the PEAK number and without any clip light or clip state, on the owner's feedback to v2 (#151, 26.10): Live mixes in 32-bit float, so a track cannot clip inside Live. Mockup v1 (`lufs-v1.html`, two variants of a pill beside today's meter) was rejected by the owner on 2026-10-26 (#151, "OWNER FEEDBACK on mockup v1"): the strip is redrawn the Waves LV1 way, a stereo meter inside the fader's track and the scale on its right (§6.2, §6.3).

## 0. Zhrnutie pre vlastníka

**Čo uvidíš na iPade**

- Na každom speváckom mikrofóne v PA ceste: krátkodobú hlasitosť (short-term LUFS) hneď za Trim IN, ako celé číslo, napríklad `−25`.
- Číslo je tichý text: malé tlmené sivé číslice, bez rámčeka, okraja či žiary. V cieli −25 ±3 LU (−28 až −22) len sivé číslo; za ±3 LU sa tlmene jantárovo zafarbí iba text, za ±6 LU (pod −31 alebo nad −19) tlmene červeno, nikdy nie tvar.
- Keď spevák nespieva, nie je tam nič (ani pomlčka, ani rámček). Ticho a pauzy nikdy nič nerozsvietia.
- Meno pásu (tlačidlo MUTE) je pokojné: svetlé veľké meno na tmavom podklade, farba stopy len ako 4 px pásik vľavo; stlmený pás ustúpi do pozadia ako dnes (celý stmavne, meno sivé a málo kontrastné, meter a gombík vyblednú) a v rohu mena je malé červené slovo MUTED (len text, ako dnešná značka MUTE), bez výplne a bez prečiarknutia. Značka RET sa na pásoch nezobrazuje.
- Jedna farba na pás: farba stopy z Ableton Live (tenký pásik vľavo pri mene); farebná horná hrana skupiny zmizne, skupinu pomenúva len tlmený sivý text na boku pása.
- Poradie výraznosti: gombík fadra a stĺpiky metra; meno čitateľné; stlmený pás ustúpi, nikdy nesvieti; potom dB z Live; LUFS najtichší.
- Na výstupe do reprákov (master return): len živé short-term číslo, bez farby a bez priemeru za pieseň.
- Pás ako vo Waves LV1: v dráhe fadra pod gombíkom dva tenké stĺpiky L a R (stereo, panoráma a nevyvážený obraz sú vidno), stupnica dB vpravo od dráhy, dnešný tyrkysový pás vo fadri a tmavý meter vľavo zmiznú.
- Vľavo od gombíka stĺpik: LUFS (na pásoch so značkou), pod ním meno skupiny a dole kláves SEL detailu kanála (namiesto ☰ pod MUTE; na iPade 36 × 36 px). Žiadne číslo špičky ani svetielko clipu (Live mieša v 32-bitovom floate, stopa v ňom neclipne).
- Fadre ostávajú presne také vysoké ako dnes (322 px na iPade, zmerané v mockupe). Nad fadre nepribudne nič.

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
- any element above the faders, or a fader travel shorter than today's (322 px on the iPad's FOH page);
- an output meter that does not follow the fader on a track Live meters at its input (§3.4).

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

### 3.4 The stereo output meter in the fader track (mockup v2)

The v2 strip draws a track's output as two bars, L and R, inside its fader's track (§6.3). Where the two levels come from:

- **Already in the code:** the layout's `meter_source: lr` subscribes Live's `output_meter_left` / `output_meter_right` (`binding::meter_props`; `MeterView` draws one bar per level slot, one clip light for all). The site runs `level` (one bar, the max of L/R) since #5 K2 (2026-09-29): with `lr`, Live's CPU did not rise, but the master Live's script of then (0.1.0-dev.9) fell behind on its sends. The script's send path was rebuilt since (#9). **A PC re-test of `lr` with the current script and Live playing is the main session's job and gates the surface PR.**
- **Armed tracks (#151, latest comments):** the PA singers' mic tracks stay armed for recording (the owner: they stay armed), and Live meters an armed track's INPUT, not its post-fader output; that is why today's strip meter does not follow the fader on those strips. For them the L/R levels come from a device the project ships, placed LAST in the track's device chain (the same Max for Live build as `FohMixer LUFS`, §4.2, or a sibling device `FohMixer Meter` *(call at build time)*), which writes its L and R peak levels as parameters, followed like the LUFS readings (§5.2).
- **A device at the chain's end is still before Live's mixer** (the track's volume and pan come after every device). The hub turns its levels into the post-fader output: `out_L = dev_L + fader_dB + panL_dB`, `out_R = dev_R + fader_dB + panR_dB`, with the fader's dB from the volume value it already follows (`fohmixer_proto::volume::value2db`) and the pan's gains from Live's pan law (UNVERIFIED for Live 12: the law, constant power or linear, is measured on the PC before the hub uses it; the strip then subscribes the track's panning, which today only the channel detail does). A muted track's output reads −∞. The result is sent to the pages as a meter level, so the surface does not know which source a strip uses.
- **Non-armed tracks** (the band's instruments, returns, the master Live's tracks, the PA output) keep Live's own `output_meter_left` / `output_meter_right`.
- Which strips use the device: a strip whose track carries the device (found by the same walk as `FohMixer LUFS`, §5.2); a missing device on an armed `+LUFS` track is the problem `no meter device` *(call: the hub could also read `arm` and flag every armed strip without one)*.

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

The value shown is the integer `round(v)` (half away from zero). The tint is judged on that same integer, so the number and its tint never disagree *(call)*. The tints colour the TEXT only (§6.2, v5):

| State | Mic (`+LUFS:-25`) | Output (`+LUFS`) |
|---|---|---|
| `fine` (dim grey text) | −28…−22 (|d| ≤ 3) | — |
| `warn` (muted amber text) | −31…−29, −21…−19 (3 < |d| ≤ 6) | — |
| `far` (muted red text) | ≤ −32, ≥ −18 (|d| > 6) | — |
| `plain` (dim grey text) | — | any value > −70 |
| `silent` (nothing shown) | `V` = −70: under 1 s of voice in 3 s | `S` ≤ −70 |
| `none` (nothing shown) | offline, no meter, or no value yet | same |

A strip in `none` because the device is missing also shows the existing problem `!` in its corner, with `no LUFS meter` in the problem list.

### 6.2 Where on the strip (mockup v2)

v1's two variants (a pill in the readout line, or over the top of today's meter) are gone with the owner's rejection of v1. In v2 the readings sit in the information column the new fader zone frees (§6.3): the LUFS at the column's top, under a small `LUFS` caption, on `+LUFS` strips only; the row stays empty on every other strip, so the eye finds each number at the same place.

**The look (v5, the owner's ruling on v4, #151, 26.10):** v4's filled pill was the most glaring thing on the strip, and a lit box while the singer was silent was rejected outright. The LUFS is quiet text: JetBrains Mono 500 at 10.5 px (9.5 px upright on a narrow strip), dim grey (`#6e6e88` on the strip's `#1a1a28`), no background, no box, no border, no glow, never a shape. In range (`fine`) and on the output (`plain`) it is that dim grey; out of range only the text is tinted, muted: amber (`#a8894a`) for `warn`, red (`#b0585f`) for `far`. `silent` and `none` show nothing at all: no number, no dash, no box, and the caption goes too (the row keeps its height, so nothing moves). The hierarchy rule: the fader cap and the meter bars are the loudest; the strip's name reads well, and a muted strip recedes (§6.4); then Live's dB readout (12 px, 600, `#9be7a8`); the LUFS is clearly quieter than the dB: smaller, lighter and of lower contrast (mockup v5 checks all three on every strip; its contrast is at most 44 % of the dB's). Live's dB stays in the readout line under the name, as today (held: accent, as today); since v4 the line holds the status light and the dB only, no ☰ (§6.3).

On a narrow strip (under 78 px: the phones) the column is 13–24 px wide: the LUFS stands upright, digit under digit (`writing-mode: vertical-rl; text-orientation: upright`), and the caption leaves. Which layout a strip takes is decided from the strip width the arrangement already computes (`arrange.rs`), never by measuring text (#21). The mockup checks every reading inside its box on all screens and in a sweep of one strip from 64 to 120 px.

### 6.3 The strip's fader zone (mockup v2, the LV1 way)

The owner's reference is Waves LV1's new generation: a thin stereo meter pair inside the fader's own track, under the cap, and the dB scale on the track's right. The zone keeps today's height (the head, 30 + 2 + 22 px, and the zone's padding, 8 px top and 10 px bottom, unchanged): **322 px of travel on the iPad's FOH page (1194 × 834, two lines), 292 px on a phone on its side and 327 px upright**, measured in the mockup, the same with and without LUFS. Right to left, in the zone:

- **The scale** (17 px, the strip's right edge): today's labels (+6 … −60), left-aligned beside the track. The cap (32 px on the iPad) overhangs the labels' first ~8 px as it passes (the price of a usable information column; the label under the cap is read from the cap's own position).
- **The track** (12 px, 2 px from the scale): two bars, L and R, 4 px each, 2 px apart, on the fader's dB scale (today's zones: green, yellow from −12 dB, red from −3 dB; the TouchOSC-calibrated position, as today), each with its peak line (the highest level of the last 1.5 s; never called or shown as a clip). The 0 dB mark is 4 px either side of the track.
- **No clip light, no clip state (v3, the owner's feedback to v2, #151, 26.10):** Live mixes in 32-bit float, so a track cannot clip inside Live; the strip drops today's clip light (`MeterView`'s `clip` signal, `.meter-clip`, `behave::peak::clips` / `CLIP_LEVEL` lose their strip use), and v2's per-channel lights and red `CLIP` are gone. The zone keeps its 8 px top padding (today's room for the light), so the travel stays today's.
- **The cap**, centred on the track, opaque, with a 1 px dark ring (no blur): the bars run under it, its white line stays readable over them. Today's turquoise fill in the groove is gone.
- **The information column**, from the zone's left edge to 3 px left of the cap's widest reach (29 px on an 83 px iPad strip, 37 px on a wide one): LUFS (§6.2), then the group's name (#127, bottom to top; it moves here from the scale column), and at its foot the channel detail's key (below). Nothing else: v2's PEAK number was removed in v3 (the owner's feedback, #151, 26.10).
- **The channel detail's key (v4, the owner's feedback to v3, #151, 26.10):** today's ☰ (`StripMenu`, `.strip-menu` in `.strip-info`) leaves the readout line: right under MUTE it was a mis-tap risk, and a burger icon does not belong on a mixer strip. The readout line keeps the status light and Live's dB, centred (`--menu-w` 0). The key sits at the bottom left of the fader zone: its touch box runs from the strip's inner edge (over the strip's 4 px padding) to the cap's widest reach, `min(36px, 100% − 21px − cap-w / 2)` of the zone, and up from the zone's foot (into its 10 px bottom padding), 36 px high; on a strip under 78 px 44 px high. So it never meets the cap, even at −∞, and the travel is unchanged. Measured in mockups v4 and v5: **36 × 36 px on the iPad's 83 px strips** (from 83 px up), about 32 × 36 px on a 79 px strip (a phone upright as the app stretches it), **20 × 44 px at the narrowest 64 px** (a phone on its side; 26.4 × 44 on a wide strip there). The drawn key is 2 px inside its box. It takes its own touches (`use:owns_touches`, no keys); the rest of the zone stays the fader's. Its press is ☰'s, unchanged (`behave/hold.rs`: a hold of 500 ms opens the detail, a tap shows the hint, a slide past 10 px ends it), and its flight-recorder `detail` steps keep their names. The pictogram is the word `SEL` in the condensed face (a console's select key, as on Yamaha and LV1 desks): the owner approved it on v4 (#151, 26.10); v4's variant B (three mini faders) is dropped.
- Considered and left out of the column: a peak number and a clip mark (v3, above), the pan position (it would need the pan subscribed on every strip; the L/R bars show its effect, and it is in the channel detail), gain reduction (not in the LOM), automation state (shown by the REVERB/AUTO marks).

The whole zone stays the fader's touch target (TouchOSC's 1:1); the column, the scale and the bars take no touch. The meter moves as today: a cover's `scaleY` and a peak layer's `translateY` per bar, no layout per frame (#21).

### 6.4 The name button (mockup v5, the owner's feedback, #151, 26.10)

Today the strip's name button (it is MUTE, #63) is filled with the Live track's colour (R4) and lights while the track plays. The owner: extremely loud and garish, its colours look random, mute is used rarely, and the point is that the fader's NAME reads well. So:

- **Unmuted:** the strip's dark ground (`#15151f` with the 1 px `--line` ring), the name in light, large text (`--text`, the condensed face at today's size, up to 17 px by `--n`), no colour fill. The Live track's colour stays only as a 4 px bar on the button's left edge (`--tc`), so neighbouring strips stay tellable apart.
- **No RET mark (v7, the owner, #151, 26.10):** the return's `RET` mark (`.strip-ret`, #68, drawn by `MuteView`'s `Corner.ret` in `.strip-corner`) goes from every strip; the corner line keeps `!`, the linked mute's chain, the fader link's mark and the mute word. (Its sweep rows in `strip-head.spec.ts` go with it.)
- **One colour per strip (v6, the owner's question on #151, 26.10: why do the header and the name show different colours?):** today a strip carries two: the 3 px top edge in the layout GROUP's colour (`--gc`, `.strip::before`, #127) and the name in the LIVE TRACK's colour (`--tc`, R4). The decision: one colour, the Live track's own as set in Ableton (so the engineer changes a strip's colour in Live, nowhere else), drawn only as the name's 4 px edge. The group's top edge is not drawn on a strip (`.strip::before` gone there; a block's top edge, `group.css` `.block::before`, is not a strip and is outside this note), and the group's name on the strip's side (`.group-side`, now in the information column, §6.3) is calm grey text (`--faint`), no longer teal or violet by its instance; it still names the group. The run title over a group whose run starts with no strip (#127, `.group-caption`) is outside the strip and unchanged. Mockup v6 checks every strip for no top edge and a grey group name.
- **Muted (v6, the owner's ruling on v5, #151, 26.10):** the strip RECEDES; nothing on it is brighter than on an unmuted strip. v5's loud muted look (a dark red fill, a struck-through name, `MUTED`) made the switched-off channel the most striking thing on the mix and is gone. As today's surface does it (`strip.css`: `--panel-mute`, the top edge at 0.3, the cap, the meter and the readout at 0.4; the SEL key at 0.6), and the name button goes darker than an unmuted one (`#101018`, the `--line-soft` ring), its name grey and of low contrast (`#55556e`), its colour edge at 0.3. **The mute word (v7, the owner's ruling on v6, #151, 26.10):** the corner line carries today's red text mark (`strip.css` `.mute-mark`: 800 7.5 px `--ui`, `--mute`, 0.6 px spacing, `margin: 0.5px 0 0 auto`, the corner line's right end), its word changed from `MUTE` to `MUTED`, so the state is read for sure; red text only: no fill colour, no badge box, no strike. An unmuted strip shows no mute word. The `.strip-corner` rules are unchanged (the mark yields first when the corner is full); mockup v7 checks that `MUTED` is seen whole in the corner on every muted strip, at 64 px too. Mockup v6 checks on every screen that each muted name button is no lighter and its name of lower contrast than every unmuted one, with no text decoration. A linked mute's `mixed` state keeps today's chain mark and the button's unmuted look (not every track is muted).
- **Unchanged:** the button's size (30 px high, the strip's width), its tap (MUTE on the down, the mute guard, the linked mute, #127), its corner marks and its intent outline (#43). Only `.mute.lit`'s colours change; `--tt` (the text colour on the track colour) is no longer needed on a strip.

### 6.5 Texts (English, #141)

- The name button: the strip's name; the red `MUTED` in its corner while muted (today: `MUTE`), nothing while unmuted; no `RET`.
- On the strip: the number only (`−25`, `−18`), nothing at all when silent or missing (no `—`); on a strip of 78 px or more the small caption `LUFS` over the number while it shows. No unit beside the number. No PEAK and no CLIP text (v3).
- In the problem list: `no LUFS meter`, `two LUFS meters`, `bad LUFS tag`, `no meter device` (§3.4).
- The tag manual (`znacky.html`): `+LUFS:<target>` and `+LUFS`, with the states table.

## 7. Protocol and data additions (summary)

- `fohmixer_proto::markers`: `+LUFS`, `+LUFS:<n>`; problems `DuplicateLufs`, `BadLufs`.
- `fohmixer_proto::layout::Strip`: `lufs: Option<LufsSpec { target: Option<i8> }>` (`deny_unknown_fields` stays).
- `fohmixer_proto::lufs`: `judge(value_tenths, target) -> LufsState` and the display integer; the constants `FINE_LU` 3, `FAR_LU` 6, `SILENT` −70.
- `StripMark` (or the problem list): `NoLufsMeter`, `TwoLufsMeters`.
- `ServerMsg::Lufs`, paced as meters.
- The output meter of an armed track (§3.4): the meter device's L/R parameters followed by the hub, combined with the fader's dB and the pan law, sent as the strip's meter levels; the strip's pan subscribed for it. `meter_source: lr` on the site once the PC re-test passes.
- No change to the Live script or its protocol (§4.1 (a)); in the fallback (§4.4) a hub UDP listener on localhost.

## 8. Logging

- `lufs` records (info): a device found or lost (strip, instance, ref, version), a problem, a walk retried or given up.
- A mic strip's state change (`fine`/`warn`/`far`/`silent`) as one `lufs` record with the value, at most one per strip per 3 s (the window's length) *(call)*, so the forensics timeline (#43) shows "Vox 2 far −33 from 19:04:12" next to the transport. No time series of values.

## 9. Tests

- **The reference DSP (Rust, CI):** a small pure module with the BS.1770 filters, sub-blocks, `S` and `V`, tested against EBU Tech 3341's cases that apply to short-term (a stereo 1 kHz sine at −23 dBFS reads −23.0 ±0.1; the 20 dB step cases; the sample-rate variants 44.1/48/96 kHz), and the gate: a 2 s phrase, a 2 s pause (V stays at the phrase's value, S falls), 0.5 s of voice in 3 s (silent), a voice at −34 (red, not silent), a 1 s breath at −38 (silent).
- **The device against the reference (PC, an L check):** the test signals rendered as WAVs, played through the device in Live; its parameters agree with the reference within 0.1 LU. The `.genexpr` text and the Rust module share one written-out coefficient table, kept equal by a CI check *(call)*.
- **proto:** the tag (`+LUFS`, `+LUFS:-25`, `+LUFS:-5` bad, `+LUFS:x` bad, two tags), the composition into `Strip.lufs`, `judge` on every boundary (−22 fine, −21 warn, −19 warn, −18 far, −28 fine, −29 warn, −31 warn, −32 far, −70 silent, rounding of −21.5 and −22.4), the frame file's `lufs` field.
- **hub (SimLive):** SimLive gains a Max device with `Short-term`/`Voice` parameters it can drive. A walk finds it two racks deep; a device inserted before it re-walks and follows the new ref; a missing and a doubled device give their problems; a disconnect clears; pacing and an idle page; `/api/status`. The mutation gate covers `judge`, the walk's matching and the pacing.
- **E2E (Playwright, Chromium and WebKit iPad, the phone sizes):** a SimLive `Voice` of −25, −21, −17, −30, −33, −70 shows `−25` in the quiet grey, `−21` and `−30` in the muted amber, `−17` and `−33` in the muted red (the text's colour only: the reading has no background, border or box shadow), and for −70 nothing (no text, no caption, no box, the row's height kept); the output shows its plain number in the quiet grey; on every `+LUFS` strip the LUFS text is smaller, lighter and of lower contrast than the strip's dB readout; two bars in the fader track with SimLive's different L and R levels at different heights; a level at Live's top (1.0) lights no clip mark and shows no `CLIP` or `PEAK` text anywhere on the strip; the fader travel on the FOH fixture at 1194 × 834 is at least 322 px and a `+LUFS` strip's equals a plain strip's; every reading inside its box in the 64–120 px sweep; no `strip-menu` in the readout line; the detail key (`SEL`) with its box at least 36 × 36 px on the iPad strip, clear of the cap at −∞ (a down on its spot at −∞ grabs the cap, never the key, and a down on the key never moves the fader), its 500 ms hold opening the detail and a tap only the hint (the existing ☰ tests moved to the key); the name button unmuted has no fill of the track's colour (its background is the dark ground on strips of different track colours) and a 4 px bar of the track's colour at its left edge, and muted darker than every unmuted button with a lower-contrast name, no fill colour and no strike, the red text `MUTED` seen whole in its corner line at every width from 64 px and no mute word on an unmuted strip, no `RET` on any strip, and the strip's cap, meter and readout at 0.4 as today; its size and its tap as today; no strip draws the group's top edge (`.strip::before`) and the group's side name is grey, so the track's colour is the strip's only colour; zero console errors.
- **hub (armed tracks, §3.4):** SimLive's meter device's L/R plus the fader's dB and the pan law give the strip's output level; a muted track reads −∞; a non-armed track keeps Live's meters.

## 10. Rollout on the PC

1. **P1** (§4.3) on a set copy; results on #151. Decide (a) or the fallback.
2. **PR 1: proto + hub + SimLive + the reference DSP**, nothing on the surface yet (`/api/status` shows the readings).
3. **The device:** the patch built and frozen on the PC; `.amxd` installed into both Live users' User Libraries by `Install-Fohmixer.ps1` (the installer already checks Live's User Library for the script).
4. **The set work (ours, memory "restart Live and do set work yourself", "save the Live set yourself"):** in the band set, a new chain `LUFS` in each PA singer's `CORE Effects` beside trueLevel, the device in it; `+LUFS:-25` on each PA singer's Tuner. In the master set, the device on the speakers' return, `+LUFS` on its Tuner (or the frame strip's `lufs`). Each set saved with Ctrl+S. Not during an owner-declared event.
5. **Check:** each rack's `latency_in_samples` unchanged; `/api/status` lists every strip's device; a played song: our `V` vs trueLevel's short-term on two singers, within 0.5 LU (recorded on #151); Live's CPU meter before and after (all devices together).
6. **PR 2: the surface**, the v2 strip the owner approved (after the `lr` re-test and the meter device on the armed tracks, §3.4); deployed and verified on the iPad on real Wi-Fi (memory "real-device acceptance").
7. **Acceptance with the owner:** a song with a mic at the right trim (quiet grey number), a trim moved 5 dB (amber text) and 8 dB (red text), the singer silent (nothing shown), the output's number while a song plays.

## 11. Open questions for the owner

- **Q1. Bleed on the vocal mics.** With the band playing and the singer silent, how loud is the stage in a vocal mic after its trim? If it can reach −40 LUFS, the gate (−40) should go lower than the bleed would read as a voice, e.g. −36. A one-song check during P1 answers it; the owner only confirms which level counts as "not singing".
- **Q4 (answered, 26.10):** the detail key is `SEL` (variant A); B dropped. The LUFS is quiet text (v5).
- **Q2 (answered, 26.10):** no PEAK number and no clip light or clip state on the strips (Live mixes in 32-bit float); mockup v3.
- **Q3. Which tracks:** only the PA singers and the speakers' return, or also the master Live's hand mics (speech) with the same −25 target?
