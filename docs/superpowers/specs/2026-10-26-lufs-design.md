# Loudness (LUFS) on the mics and on the PA output (#151)

Status: draft for the owner's approval, 2026-10-26. The rules in §2 are the owner's, recorded on #151 on 2026-10-26 (brainstorm answers and two DECIDED comments). The main session's calls are marked *(call)*. Nothing is built before the owner approves this note and mockup `docs/mockups/lufs-v8.html`. Mockup v8 is v7 with the strip's head redrawn on the owner's ruling (#151, 26.10): the name button is the strip's top edge, the name first and one line of marks under it, and a quieter red `MUTED` (§6.4). Mockup v7 is v6 with the owner's ruling on it (#151, 26.10): the muted strip recedes and its name button carries today's small red text mark, its word `MUTED`; no RET mark on any strip (§6.4). Mockup v6 is v5 with the owner's ruling on its muted look (#151, 26.10): a muted strip recedes as today's surface does, never lights up, and one colour per strip, the Live track's own (§6.4). Mockup v5 is v4 with the owner's ruling on it (#151, 26.10): the SEL key approved (variant B dropped), and the LUFS as quiet text, never a shape, nothing while the singer is silent (§6.2); and the name button calm, loud only while muted (§6.4). Mockup v4 is v3 (the owner: "better", #151, 26.10) with the channel detail's ☰ moved out of the readout line under MUTE to a key at the strip's bottom left, in two pictogram candidates (§6.3). Mockup v3 is v2 (`lufs-v2.html`) without the PEAK number and without any clip light or clip state, on the owner's feedback to v2 (#151, 26.10): Live mixes in 32-bit float, so a track cannot clip inside Live. Mockup v1 (`lufs-v1.html`, two variants of a pill beside today's meter) was rejected by the owner on 2026-10-26 (#151, "OWNER FEEDBACK on mockup v1"): the strip is redrawn the Waves LV1 way, a stereo meter inside the fader's track and the scale on its right (§6.2, §6.3).

**Revised 2026-10-27 (#151, after probe P1 and the owner's rulings of 27.10).** PR 1 (the v8 strip) shipped as v1.17.0. Probe P1 failed option (a): a parameter the device writes itself is an Undo step in Live 12, and "Stored Only" takes the parameters out of the LOM (§4.3). So the localhost path is the design: the devices have no parameters and send their readings to the hub by UDP (§4.4). The owner dropped the Rust reference DSP: the device is checked against Sonible trueLevel on the PA singers (§9, §10). The surface says which loudness a number is, `LUFS IN` or `LUFS OUT` (§3.1, §6.2). `meter_source: lr` is live on the site (§3.4). PR 2's plan: `docs/superpowers/plans/2026-10-27-lufs-pr2.md`.

## 0. Zhrnutie pre vlastníka

**Čo uvidíš na iPade**

- Na každom speváckom mikrofóne v PA ceste: krátkodobú hlasitosť (short-term LUFS) hneď za Trim IN, ako celé číslo, napríklad `−25`.
- Číslo je tichý text: malé tlmené sivé číslice, bez rámčeka, okraja či žiary. V cieli −25 ±3 LU (−28 až −22) len sivé číslo; za ±3 LU sa tlmene jantárovo zafarbí iba text, za ±6 LU (pod −31 alebo nad −19) tlmene červeno, nikdy nie tvar.
- Keď spevák nespieva, nie je tam nič (ani pomlčka, ani rámček). Ticho a pauzy nikdy nič nerozsvietia.
- Nad číslom drobný tlmený popis, ktoré číslo to je: `LUFS IN` (za Trim IN) alebo `LUFS OUT` (výstup), v dvoch riadkoch (`LUFS` a pod ním `IN` / `OUT`). Na úzkom páse (telefón) len číslo.
- Tlačidlo s menom je horný okraj pása cez celú šírku (37 px): najprv meno hore, pod ním jeden riadok drobných značiek (!, reťaz spojeného MUTE s počtom, spojenie fadrov, červené MUTED). Miesto sa vzalo z horného okraja pása a riadku dB, nie z fadra.
- Meno pásu (tlačidlo MUTE) je pokojné: svetlé veľké meno na tmavom podklade, farba stopy len ako 3 px pásik vľavo; stlmený pás ustúpi do pozadia ako dnes (celý stmavne, meno sivé a málo kontrastné, meter a gombík vyblednú) a pod menom je malé tlmené červené slovo MUTED (len text, ako dnešná značka MUTE), bez výplne a bez prečiarknutia. Značka RET sa na pásoch nezobrazuje.
- Jedna farba na pás: farba stopy z Ableton Live (tenký pásik vľavo pri mene); farebná horná hrana skupiny zmizne, skupinu pomenúva len tlmený sivý text na boku pása.
- Poradie výraznosti: gombík fadra a stĺpiky metra; meno čitateľné; stlmený pás ustúpi, nikdy nesvieti; potom dB z Live; LUFS najtichší.
- Na výstupe do reprákov (master return) a na výstupných zberniciach (Vocals, Backing): len živé short-term číslo s popisom `LUFS OUT`, bez farby a bez priemeru za pieseň.
- Pás ako vo Waves LV1: v dráhe fadra pod gombíkom dva tenké stĺpiky L a R (stereo, panoráma a nevyvážený obraz sú vidno), stupnica dB vpravo od dráhy, dnešný tyrkysový pás vo fadri a tmavý meter vľavo zmiznú.
- Vľavo od gombíka stĺpik: LUFS (na pásoch so značkou), pod ním meno skupiny a dole kláves SEL detailu kanála (namiesto ☰ pod MUTE; na iPade 36 × 36 px). Žiadne číslo špičky ani svetielko clipu (Live mieša v 32-bitovom floate, stopa v ňom neclipne).
- Fadre ostávajú presne také vysoké ako dnes (322 px na iPade, zmerané v mockupe). Nad fadre nepribudne nič.

**Odkiaľ to číslo príde**

- Live samotný LUFS nemeria (LOM ho nemá).
- Projekt dodá malé zariadenie Max for Live `FohMixer LUFS`. Počíta hlasitosť podľa normy ITU-R BS.1770, rovnako ako Sonible trueLevel.
- Vloží sa do paralelného chainu v `CORE Effects` vedľa trueLevel. Meria teda presne v tom istom bode a zvuk ani latenciu nemení (do chainu posiela ticho).
- Zariadenie nemá žiadne parametre: skúška na PC 27.10 ukázala, že parameter, ktorý zariadenie samo mení, Live zapisuje do Undo. Čísla preto posiela 10× za sekundu priamo hubu na tom istom PC (localhost) spolu s tým, na ktorej stope sedí.
- Ktoré stopy sa merajú a čo číslo znamená, povie značka v Tuneri: `+LUFS:-25` = hlasitosť hneď za Trim IN, súdená voči cieľu (popis `LUFS IN`); `+LUFS` = výstupný bod, len číslo (PA return, zbernice Vocals / Backing; popis `LUFS OUT`).
- Nahrávané (armed) PA mikrofóny dostanú druhé malé zariadenie `FohMixer Meter` na koniec reťazca. Live na armed stope meria vstup; s ním meter na páse ukáže výstup za fadrom.

**Ako sa to overí na PC**

- Číslo nášho zariadenia sa porovná so short-term čítaním trueLevel na dvoch PA spevákoch počas hranej piesne. Musia sedieť do 0,5 LU (výsledok na #151).
- Latencia racku sa vložením nezmení a Undo ostane bez krokov od merania.
- Pred meterom armed stôp sa na kópii setu zmeria stupnica metra v Live a zákon panorámy (sonda P2, §4.5).

## 1. Goals and non-goals

**Goals**

- G1. Each PA mic track shows its singer's short-term loudness right after the Trim IN, judged against the team's −25 LUFS, so a wrong trim (analog gain moved, mic swapped, weak singer that day) shows on the surface during a normal passage.
- G2. The PA output (the master Live's return to the speakers), and any output bus that carries the tag (the Vocals / Backing buses), shows its live short-term loudness as a plain number, to compare songs' backing tracks while they play.
- G2b. The surface says which loudness a number is: after the trim (`LUFS IN`) or at an output (`LUFS OUT`) (the owner, 27.10).
- G3. Silence and pauses never light a warning.
- G4. The faders keep their height; nothing is added above them (memory rule "fader height first").
- G5. A thin layer: the measurement lives in Live (a device in the track's chain), the hub only receives its readings over localhost and matches them to the strips, and the tracks are chosen by Tuner tags, as every per-track feature since #68. *History: until P1 the hub was to follow the readings as device parameters through the LOM (§4.1 (a)).*

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
- an Undo step, or an automation lane, made by the meter (the devices have no parameters, §4.2); the set's changed mark is no criterion, playback alone sets it (P1, §4.3);
- any element above the faders, or a fader travel shorter than today's (322 px on the iPad's FOH page);
- an output meter that does not follow the fader on a track Live meters at its input (§3.4).

## 2. Rules (the owner's, binding)

- **What is measured on a mic:** short-term loudness (ITU-R BS.1770-4 / EBU R 128 short-term: K-weighted, 3 s window) right after the Trim IN utility on each PA mic track. (#151, FINDING 26.10.)
- **The mic target:** −25 LUFS in a normal passage of a normal song. (FINDING 26.10.)
- **The mic tolerance:** within ±3 LU (−28 to −22) fine; beyond that yellow; beyond ±6 LU (below −31 or above −19) red. (DECIDED 26.10, owner's answer "1".)
- **The PA output:** the live short-term LUFS number only, no per-song average, no target colour. (DECIDED 26.10.)

## 3. What is measured, where

### 3.1 The points

| Strip | Live set | Track | Point | Shown | Caption |
|---|---|---|---|---|---|
| each PA singer (`Vox 1`…), tag `+LUFS:-25` | band | the PA mic track | after Trim IN: a parallel chain of its `CORE Effects` rack, beside trueLevel | `Voice` (§3.3), tinted against the target | `LUFS IN` |
| PA output, tag `+LUFS` | master | the return to the speakers | the return's device chain, before its mixer | `Short-term`, plain | `LUFS OUT` |
| any output bus, tag `+LUFS` (the Vocals / Backing buses, the owner 27.10) | either | the bus | its device chain, before its mixer | `Short-term`, plain | `LUFS OUT` |

**The kind of a number (the owner, 27.10: the surface must say WHICH loudness a number is):** a target in the tag (`+LUFS:<target>`) makes the point an INPUT point, read after the trim (the mic's gain staging) and judged; a bare `+LUFS` makes it an OUTPUT point, the plain short-term. The kind comes from the tag alone, never from the track's place.

**Seen on the PC earlier (to confirm at rollout, #151 comment of 26.10):** each PA mic track has a `CORE Effects` rack with a Sonible trueLevel in a parallel chain after the Trim IN. The team reads −25 there today. Placing our meter in a sibling chain of the same rack measures exactly the signal trueLevel measures *(call)*.

### 3.2 Never in the signal path

- **The devices' output is silence** (`plugout~` fed by a `sig~ 0.` and nothing else; `scripts/check_lufs_device.py` refuses any other feed), and **each device is ALWAYS placed in a parallel chain of an Audio Effect Rack, never in series** (the owner's ruling of 27.10: a plug-in or licence failure must never cut the sound, and a doubled signal is worse than none). *(Briefly on 27.10 the devices passed their input through, after the UDP probe found the PR #154 device silencing what followed it in series; the owner's ruling replaced that.)*
- **Where:** a mic's LUFS device in its own chain of the `CORE Effects` rack beside trueLevel (§3.1); an output point (the PA return, a Vocals / Backing bus) and an armed track's meter device (§3.4) get a rack of their own with an empty dry chain plus a chain holding the device. The dry chain carries the sound; the device's chain adds silence to the rack's sum.
- The chain stays active (never muted by its activator): a deactivated chain may stop processing its devices. UNVERIFIED for Live 12; checked at the rollout (the readings keep coming).
- The device reports no latency (a Max for Live device that does not set `latency` reports 0, UNVERIFIED for this exact patch: checked at rollout by reading `latency_in_samples` of the rack before and after the insertion, which must not change).
- Memory rule "latency claims need measurement": the rollout check above is a read of Live's reported latency, not a timing claim. The path's audio is untouched by construction (the device adds silence to a sum).

### 3.3 The two readings the device makes

Every 100 ms (EBU Tech 3341's 10 Hz minimum), on the stereo input (L and R, each weight 1.0, as BS.1770 sums a stereo track; a centred mono mic on a stereo track therefore reads as trueLevel reads it on that track):

1. **K-weighting:** BS.1770's two biquads (the high-shelf pre-filter and the RLB high-pass), coefficients derived for the running sample rate from the standard's analog prototypes (at 48 kHz they give the standard's published values; the device as a whole is checked against trueLevel, §10).
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
5. **Sent** every 100 ms, whether the values changed or not (the hub takes a device silent for 1 s as gone, §4.4): `Short-term` and `Voice` in tenths of LU, integers −700…0. *History: until P1 they were two device parameters, floats −70.0…0.0 written only when the value rounded to 0.1 changed (§4.3).*

An output point shows `S`, an input point shows `V` *(call)*: the output's number is the standard short-term the owner asked for, silence there between songs reads low (−50) or nothing, which is the truth for the room. Which of the two a strip shows follows its tag's kind (§3.1): `V` for `+LUFS:<target>`, `S` for `+LUFS`.

### 3.4 The stereo output meter in the fader track (mockup v2)

The v2 strip draws a track's output as two bars, L and R, inside its fader's track (§6.3). Where the two levels come from:

- **Live's own L/R:** the layout's `meter_source: lr` subscribes Live's `output_meter_left` / `output_meter_right` (`binding::meter_props`; one bar per level slot). **Live on the site since 27.10 (#151):** the PC re-test with the current script passed. The band and master Lives' CPU stayed within the run-to-run noise, the round trip p95 under 19 ms, no read lost, no heartbeat gap of 150 ms; the 2026-09-29 failure of the old send path is gone. The deployed layout and the import's source layout both carry `"meter_source": "lr"`. Still open: the hub log's busy lines with the transport playing, read in PR 2's verification (§10).
- **Armed tracks (#151):** the PA singers' mic tracks stay armed for recording (the owner: they stay armed), and Live meters an armed track's INPUT, not its post-fader output; that is why the strip meter does not follow the fader on those strips. For them the L/R levels come from a sibling device the project ships, **`FohMixer Meter`** *(call, 27.10: a sibling device, not a mode of `FohMixer LUFS`. The two sit at different points: the LUFS device in a parallel chain right after the Trim IN, the meter device LAST in the track's chain. A mode would need a switch the device can read, i.e. a parameter, the very thing P1 ruled out)*. It has no parameters and sends its L and R peak levels over the same localhost path as the LUFS device (§4.4, address `/fohmixer/meter`), never as device parameters.
- **A device at the chain's end is still before Live's mixer** (the track's volume and pan come after every device). The hub turns its levels into the post-fader output: `out_L = dev_L + fader_dB + panL_dB`, `out_R = dev_R + fader_dB + panR_dB`, with the fader's dB from the volume value (`fohmixer_proto::volume::value2db`) and the pan's gains from Live's pan law, then into Live's meter scale (the 0…1 value the strip already draws). Both the pan law and Live's meter scale are UNVERIFIED for Live 12; probe P2 (§4.5) measures them on the PC before the hub uses them. A muted track's output reads the meter's floor (0). The result goes to the pages on the strip's own meter keys (`output_meter_left`, `output_meter_right`, and `output_meter_level` as the larger of the two) in place of Live's values while the device is fresh, so the surface does not know which source a strip uses. A device silent for 500 ms gives the keys back to Live's values *(call)*.
- **Non-armed tracks** (the band's instruments, returns, the master Live's tracks, the PA output) keep Live's own `output_meter_left` / `output_meter_right`.
- Which strips use the device: a strip whose track carries a confirmed `FohMixer Meter` (§4.4's match; no tag needed). An armed `+LUFS:<target>` track with no fresh meter device is the problem `no meter device` *(call: the hub follows `arm` on those strips only)*.

## 4. The data source

### 4.1 The options checked

**(a) A Max for Live device the project ships, its readings as device parameters followed through the LOM.** *Chosen first, failed probe P1 on 27.10 (§4.3).*

- Why it was first: the hub already follows device parameters inside racks (`router/autotune.rs`), so no new transport. The measurement is ours and reviewable.
- Why it failed: a parameter the device writes is an Undo step with visibility "Automated and Stored", and with "Stored Only" the parameters leave the LOM (§4.3). "Hidden" leaves the LOM too.

**(b) Reading Sonible trueLevel's meter.** *Rejected.*

- Plug-ins expose their controls as parameters; their meters are outputs. Whether trueLevel declares its loudness readings as read-only VST3 output parameters, and whether Live lists them in `Device.parameters`, is UNVERIFIED (P1 did not reach its Configure look: step 3 had already decided).
- Even if one existed, its value range, scaling and the mode the team set on each instance are unknown and differ per instance; and it ties the feature to a third-party binary's update policy.
- trueLevel stays the acceptance reference instead (§10 step 7).

**(c) An audio tap from the virtual audio routing, measured by the hub.** *Rejected.*

- It needs one routed channel per mic plus the output: changes to the routing the band depends on.
- The tap point would be wherever the routing can reach, not "right after the Trim IN".
- DSP in the hub is a second audio path on the PC, against the thin layer.

**(d) Live's own meters (`output_meter_level`).** *Rejected:* post-fader peak-ish levels, not K-weighted loudness; already on every strip.

**(e) The same device, sending its readings to the hub over localhost.** *Chosen (27.10, the main session's DECIDED on #151 after P1; the program spec's first candidate, its §4.3).* The device keeps no parameters, so nothing reaches Undo or automation; the hub gets a localhost UDP listener (§4.4). Tags, the protocol to the pages and the surface are as in §5–§6.

### 4.2 The devices: `FohMixer LUFS` and `FohMixer Meter`

- **`FohMixer LUFS`:** an audio effect device, file `FohMixer LUFS.amxd`, name `FohMixer LUFS`. DSP in one `gen~` (the BS.1770 filters, the sub-block sums, the window ring buffers, §3.3); every 200 ms it sends `S` and `V` (§4.4; the owner, 27.10: 5×/s is enough for a 3 s window shown as an integer, was every 100 ms). Its audio output is silence, and it sits in a rack's parallel chain (§3.2).
- **`FohMixer Meter`** (§3.4): an audio effect device, file `FohMixer Meter.amxd`, name `FohMixer Meter`. Its L and R peak level over each 33 ms in tenths of dBFS (floor −700 = −70 dB, top 120 = +12 dB *(call)*), sent 30×/s, Live's own meter rate (the script's ~30 Hz flush). Placed LAST in an armed track's chain, inside a rack of its own (an empty dry chain plus the device's chain, §3.2), so it reads the chain's end. Its output is silence (§3.2).
- **No parameters of their own, on either device (P1):** no `live.*` parameter object, nothing with `parameter_enable`; only Live's own `Device On`, which nothing writes. The faces show the readings in plain Max number boxes (`flonum`, `ignoreclick`, not parameters) and the version, so they can be read in Live's device view: on `FohMixer LUFS` (the owner, 27.10) both readings big, labelled `Short-term` and `Voice`, under the caption `LUFS` (one device for every point: the IN / OUT meaning lives in the hub's tag), the version small.
- **Who they are:** at load, and again every 1 s *(call)*, each device reads its own Max `id` through `live.path this_device`, and the `name` of each of its first four canonical parents through `live.path this_device canonical_parent …` and a `live.object` (`-` where there is none). A moved device or a renamed track is known within 1 s. *Revised 27.10 (the UDP probe): `getpath` gives no absolute path, so no path is read.*
- **Source in the repo, binary built on the PC** *(call)*: `live-device/FohMixer-LUFS/` and `live-device/FohMixer-Meter/` hold each patch (`.maxpat` JSON); the LUFS `gen~` code also as `.genexpr` text. The `.amxd` is never committed: a frozen device can embed the author's file paths, which hold the Windows user name (public-repo hygiene), and the denylist scan skips binary files.
- **Version:** `version.txt` beside each patch follows the workspace version; the face shows `v<version>` and every datagram carries it (§4.4), so the hub logs which build each device runs.
- **Built from the patcher** *(call)*: `build_amxd.py` writes the unfrozen `.amxd` (the `ampf`/`aaaa` header, then the `.maxpat` JSON byte for byte) for either patch; the `gen~` embeds its code, so the devices have no file dependencies. `scripts/check_lufs_device.py` (CI) keeps, for both patches: the patch's shape (`plugin~`, `plugout~`; in the LUFS device one `gen~` with one codebox whose code equals the `.genexpr`; one `udpsend 127.0.0.1 <port>` with the port equal to the hub's default; the OSC address and the version in the one `pack`; `plugout~` fed by `plugin~` alone; the identity's `live.path` messages); no parameter objects; no absolute paths; the face's version equal to `version.txt`. *History: until 27.10 it also kept a constants table equal to a Rust reference DSP (`fohmixer_proto::lufs::dsp`, PR #154); the owner dropped the reference for the trueLevel comparison (§9), and PR 2 deletes both.*
- **Edges of the readings** *(call)*: before 3 s of audio the missing sub-blocks count as silence (the device's rings start at zero); a reading above 0 LUFS is sent as 0; the gen~ rounds each reading to tenths before it is sent.

### 4.3 Probe P1 on the PC (done 27.10)

Run on a copy of the band set (the event set saved first and opened again afterwards, unchanged), with a draft `FohMixer LUFS` holding two `live.numbox` parameters, loaded through the LOM itself (the User Library's browser item and `load_item`, no GUI step). Results (#151, 27.10):

- **Class:** `class_name` `MxDeviceAudioEffect`, type `audio_effect`; the parameters `Device On`, `Short-term`, `Voice` (min −70, max 0, not quantized). The device loads in Live 12 with no Max error; its face showed both readings and the version.
- **Rate:** on a playing backing-track bus the two values followed at **~6.9 events/s each** over 40 s (275 / 274 events; the script coalesces per tick, the device wrote only 0.1 changes), readings −18…−34 LUFS. Right after load the LOM read 0.0 while the face showed −70 (an initial value is not written as a change).
- **Undo: FAIL.** With visibility "Automated and Stored" every value the device writes is an Undo step: Live's Edit menu showed `Undo Change "Voice"` right after the load and again after 60 s of playing. An engineer's Ctrl+Z would undo a meter reading instead of their own change.
- **"Stored Only": FAIL** for another reason: the parameters leave the LOM (`Device.parameters` lists only `Device On`), so the hub cannot follow them.
- **The set's changed mark is no criterion:** the copy turned changed within 1 s of `start_playing` even with both devices switched off (`Undo Change "Track Volume"` on top: something writes a volume at play).
- Not done: the rack's `latency_in_samples` and trueLevel's Configure look; the latency check moves to the rollout (§10 step 7).

Outcome: option (a) is out; the localhost path (§4.4) is built.

### 4.4 The localhost path (the design since 27.10)

- **Transport:** each device sends one UDP datagram per reading to `127.0.0.1:<port>` with Max's `udpsend`. The hub binds its UDP socket on `127.0.0.1` only (never `0.0.0.0`): nothing off the PC reaches it.
- **Format:** an OSC 1.0 message, because `udpsend` speaks OSC and nothing else *(call: a JSON datagram would need a `js` / `node.script` sender in each device; OSC is what Max sends natively, small and fixed)*. Address `/fohmixer/lufs` (or `/fohmixer/meter`), type tags `,iisssssii`, ONE message per reading carrying everything (the patch's `pack`, banged by its `metro`; the identity cached in it):

  | # | type | value |
  |---|---|---|
  | 1 | `i` | the wire version, `1` |
  | 2 | `i` | the device's Max id (`live.path this_device` → `id <n>`, per Live process) |
  | 3 | `s` | the device's version, `v<version.txt>` |
  | 4–7 | `s` | the names of the device's first four canonical parents, nearest first; `-` past the top |
  | 8 | `i` | `/lufs`: `S` in tenths of LU (−700…0); `/meter`: the L peak in tenths of dBFS (−700…120) |
  | 9 | `i` | `/lufs`: `V` in tenths (−700…0); `/meter`: the R peak |

  *Revised 27.10 after the UDP probe (#151):* Max's `live.path` gives no absolute path (`getpath` echoes the path it was given), so there is no path field. The device names its parents instead (`live.path this_device canonical_parent …` → `live.object get name`): on a track `[track, set, -, -]`, in a rack's chain `[chain, rack, track, set]`. The last named parent is the set (the band's Live from the master's), the one before it the track (`lufs::wire::track_and_set`); a walk "up to the track" was not built in the patch, the hub decides from the four names. About 100 bytes. The hub refuses a datagram over 512 bytes, another address, other type tags, another wire version, a string that is not UTF-8, a value out of its range, bytes after the last value, or fewer than two named parents. A new field is a new wire version, never a reinterpreted one.
- **The port** *(call)*: `39110`, the default of the hub's config (`lufs.port`) and fixed in the patches; `scripts/check_lufs_device.py` keeps the two equal. The device cannot read the hub's config, so another port means the devices rebuilt with `build_amxd.py --port <n>` (documented in the device README). Port `0` in the config turns the listener off.
- **Which strip (the match):** the datagram's track name and set name (fields 4–7, `track_and_set`) name the device's track. A strip is a candidate when it is a served strip with `lufs` (any served strip, for a meter device) whose track has that name in a Live whose set has that name *(revised 27.10: no index or path comes on the wire; a marker strip's track is known by the hub's own track list, a frame strip by its anchor's name)*. Both Lives send to one port and Max's ids are per process, so a datagram alone does not say which Live sent it. The hub therefore **confirms** a new device: for each instance with a candidate strip it walks that track's devices (and its racks' chains, as AUTOTUNE's find does) through that instance's script, and the device is matched to the one instance where a `MxDeviceAudioEffect` named `FohMixer LUFS` (or `FohMixer Meter`) is found. None confirms: unmatched, asked again after a track-list change of a candidate instance or after 10 s. Both confirm (the same device at the same place in both sets): the strips' problem `two LUFS meters`, nothing shown. A device is known by its key (device id, its four parents' names): a moved device or a renamed track is a new key, confirmed again, and the old key goes stale.
- **Bounded:** at most twice the nominal rate is taken from one key (10 datagrams/s for `/lufs`, 60/s for `/meter`), the rest dropped and counted; at most 64 keys are kept (a new key past that is dropped and counted). A refused datagram is counted by its reason.
- **Stale:** a LUFS key with no datagram for **1000 ms** (5 sends missed) is gone: its strip shows nothing (`none`) and the hub logs it lost. A meter key is gone after **500 ms**, and its strip's meter keys go back to Live's values (§3.4).
- **Offline:** an instance's disconnect drops its matches; its strips show nothing.
- Never writes anything to Live; the confirmation is a read.

### 4.5 Probe P2 on the PC (before the hub combines the armed tracks' levels)

On a copy of the band set, after a draft `FohMixer Meter` is built:

1. On a non-armed spare track, the meter device in a rack's parallel chain last in its chain, the fader at 0 dB, pan centre: a stepped 1 kHz sine clip (2 s each at 0, −3, −6, −12, −18, −24, −36, −48, −60 dBFS; a WAV written by `tools/live-probe`) plays. The probe reads Live's `output_meter_left` / `_right` and the device's datagrams together: the pairs give Live's meter scale (dBFS → 0…1) as a table.
2. The same at pan −1, −0.5, +0.5, +1: Live's pan law (each side's gain in dB per pan value).
3. The meter device's `latency_in_samples`: 0.

Results on #151 without host names, IPs or user paths; the tables go into `fohmixer_proto::meter` (§7).

## 5. How the hub gets it

### 5.1 The tag

- `+LUFS:<target>` on a track's Tuner marker: an INPUT point (§3.1), measured after the trim, its `V` judged against `<target>` LUFS, an integer −40…−10 (`+LUFS:-25`); caption `LUFS IN`.
- `+LUFS` alone: an OUTPUT point, its `S` as a plain number (the PA return, the Vocals / Backing buses); caption `LUFS OUT`.
- Problems (`markers::parse`, the existing problem list): a second `+LUFS` (`DuplicateLufs`), a target that is not an integer in range (`BadLufs`).
- A Tuner of only `+LUFS` makes a strip like any marker (it is a strip feature, not a link like `+MUTE`).
- A strip from the frame file (not a marker) may carry the same field in `layout.json` (`"lufs": {"target": -25}` or `"lufs": {}`), so the PA output works whether its strip is a marker or a frame strip (matched by its track's name, §4.4).
- The tolerances (±3, ±6 LU) are the owner's rule and constants in code, not tag arguments *(call)*.

### 5.2 The hub's half (`lufs_udp.rs`, `rules/lufs.rs`, `router/lufs.rs`)

The AUTOTUNE pattern (a pure rule, thin glue), with a listener instead of a LOM follow:

- **The listener** (`lufs_udp.rs`): one tokio task, a `UdpSocket` bound to `127.0.0.1:<lufs.port>`. Each datagram is parsed (`fohmixer_proto::lufs::wire::parse`, pure) and passed through the per-key bound (§4.4) before it reaches the router as `RouterMsg::Lufs`; refused and dropped datagrams are only counted there. A bind that fails is one error record and a `/api/status` line; the hub runs on without LUFS.
- **The rule** (`rules/lufs.rs`, pure): the keys seen, their confirmation state, the latest reading, the matches to strips, staleness on the router's tick, the problems (`no LUFS meter` for a `+LUFS` strip of an online instance with no fresh matched device 3 s after the instance connected or the layout was served; `two LUFS meters`; `no meter device`), and each strip's state for the log (§8). Every input returns actions: a read to confirm a key, a reading to send, a record to log.
- **The glue** (`router/lufs.rs`): runs the confirming reads through the instances' scripts with the connect-time retries (`retries.rs`); follows, with its own subscriber `LEVELS_CLIENT` (`ClientId::MAX - 5`, the next free id after `SONGMAP_CLIENT`; the builder confirms), the `mixer_device volume` and `panning` values and `mute` of each strip with a matched meter device, and `arm` of each `+LUFS:<target>` strip; substitutes the meter device's levels on those strips' meter keys (§3.4).
- A disconnect drops the instance's matches: the strips show nothing, with the offline look they already have.
- Never writes anything to Live.
- *History: until P1 this was a bounded walk of each `+LUFS` track's devices and a `$ref` follow of the device's `Voice` / `Short-term` parameter with a `LUFS_CLIENT`.*

### 5.3 To the pages

- `ServerMsg::Lufs { items: [{instance, target, tenths}] }`: `target` the strip's binding target as the layout serves it (`Binding::target`, the canonical form the page already keys a strip by), `tenths` the reading in tenths of LU as an integer, or none (no device, stale, offline) *(call: tenths kept for the log and a later detail view; the strip shows the integer)*. An input point gets `V`, an output point `S`.
- Paced with the meters (`meter_pace`, #51): at most once per the page's meter period, the latest value kept; an idle page with `meter_ms` 0 gets none until it wakes, then the current values at once.
- `/api/status` `lufs`: the listener (port, bound or its error, datagrams taken / refused by reason / dropped by the bound), per key the device (kind, id, version, parents, instance and strip matched, age of the last datagram, last values), per strip its state and problem.

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

v1's two variants (a pill in the readout line, or over the top of today's meter) are gone with the owner's rejection of v1. In v2 the readings sit in the information column the new fader zone frees (§6.3): the LUFS at the column's top, under a small caption, on `+LUFS` strips only; the row stays empty on every other strip, so the eye finds each number at the same place.

**The caption says which loudness it is (the owner, 27.10; §3.1):** `LUFS IN` on an input point (`+LUFS:<target>`, after the trim), `LUFS OUT` on an output point (`+LUFS`). It shows on every strip of 78 px or more while the number shows; a narrow strip shows the number alone, as before. The caption keeps PR 1's look (`--faint`, 700 7.5 px on a 9 px line, 0.08em spacing) on **two lines**, `LUFS` over `IN` / `OUT` *(call: on one line `LUFS OUT` is about 35 px at that size (an estimate; mockup v9 measures it), wider than the column's 29 px on the iPad's 83 px strips; two 9 px lines, the 2 px gap and the 16 px value row end at 36 px, above the group's name at the column's 40 px, so nothing else moves. The owner sees it on mockup v9 before the surface task is built: memory "approve layouts")*. *History: PR 1 drew a one-line `LUFS` caption, hidden.*

**The look (v5, the owner's ruling on v4, #151, 26.10):** v4's filled pill was the most glaring thing on the strip, and a lit box while the singer was silent was rejected outright. The LUFS is quiet text: JetBrains Mono 500 at 10.5 px (9.5 px upright on a narrow strip), dim grey (`#6e6e88` on the strip's `#1a1a28`), no background, no box, no border, no glow, never a shape. In range (`fine`) and on the output (`plain`) it is that dim grey; out of range only the text is tinted, muted: amber (`#a8894a`) for `warn`, red (`#b0585f`) for `far`. `silent` and `none` show nothing at all: no number, no dash, no box, and the caption goes too (the row keeps its height, so nothing moves). The hierarchy rule: the fader cap and the meter bars are the loudest; the strip's name reads well, and a muted strip recedes (§6.4); then Live's dB readout (12 px, 600, `#9be7a8`); the LUFS is clearly quieter than the dB: smaller, lighter and of lower contrast (mockup v5 checks all three on every strip; its contrast is at most 44 % of the dB's). Live's dB stays in the readout line under the name, as today (held: accent, as today); since v4 the line holds the status light and the dB only, no ☰ (§6.3).

On a narrow strip (under 78 px: the phones) the column is 13–24 px wide: the LUFS stands upright, digit under digit (`writing-mode: vertical-rl; text-orientation: upright`), and the caption leaves. Which layout a strip takes is decided from the strip width the arrangement already computes (`arrange.rs`), never by measuring text (#21). The mockup checks every reading inside its box on all screens and in a sweep of one strip from 64 to 120 px.

### 6.3 The strip's fader zone (mockup v2, the LV1 way)

The owner's reference is Waves LV1's new generation: a thin stereo meter pair inside the fader's own track, under the cap, and the dB scale on the track's right. The zone keeps today's height (the head's 59 px: v8's 37 + 2 + 20 px from the strip's top edge, as today's 5 + 30 + 2 + 22; the zone's padding, 8 px top and 10 px bottom, unchanged): **322 px of travel on the iPad's FOH page (1194 × 834, two lines), 292 px on a phone on its side and 327 px upright**, measured in the mockup, the same with and without LUFS. Right to left, in the zone:

- **The scale** (17 px, the strip's right edge): today's labels (+6 … −60), left-aligned beside the track. The cap (32 px on the iPad) overhangs the labels' first ~8 px as it passes (the price of a usable information column; the label under the cap is read from the cap's own position).
- **The track** (12 px, 2 px from the scale): two bars, L and R, 4 px each, 2 px apart, on the fader's dB scale (today's zones: green, yellow from −12 dB, red from −3 dB; the TouchOSC-calibrated position, as today), each with its peak line (the highest level of the last 1.5 s; never called or shown as a clip). The 0 dB mark is 4 px either side of the track.
- **No clip light, no clip state (v3, the owner's feedback to v2, #151, 26.10):** Live mixes in 32-bit float, so a track cannot clip inside Live; the strip drops today's clip light, and so does the channel detail's meter (the same reason; the plan's ruling 6: `MeterView`'s `clip` signal, `.meter-clip`, `behave::peak::clips` / `CLIP_LEVEL` are deleted), and v2's per-channel lights and red `CLIP` are gone. The zone keeps its 8 px top padding (today's room for the light), so the travel stays today's.
- **The cap**, centred on the track, opaque, with a 1 px dark ring (no blur): the bars run under it, its white line stays readable over them. Today's turquoise fill in the groove is gone.
- **The information column**, from the zone's left edge to 3 px left of the cap's widest reach (29 px on an 83 px iPad strip, 37 px on a wide one): LUFS (§6.2), then the group's name (#127, bottom to top; it moves here from the scale column), and at its foot the channel detail's key (below). Nothing else: v2's PEAK number was removed in v3 (the owner's feedback, #151, 26.10).
- **The channel detail's key (v4, the owner's feedback to v3, #151, 26.10):** today's ☰ (`StripMenu`, `.strip-menu` in `.strip-info`) leaves the readout line: right under MUTE it was a mis-tap risk, and a burger icon does not belong on a mixer strip. The readout line keeps the status light and Live's dB, centred (`--menu-w` 0). The key sits at the bottom left of the fader zone: its touch box runs from the strip's inner edge (over the strip's 4 px padding) to the cap's widest reach, `min(36px, 100% − 21px − cap-w / 2)` of the zone, and up from the zone's foot (into its 10 px bottom padding), 36 px high; on a strip under 78 px 44 px high. So it never meets the cap, even at −∞, and the travel is unchanged. **PR 1's ruling (the main session on #151, after PR #153's review):** the box ends 3 px short of the cap's widest reach, `min(36px, 100% − 24px − cap-w / 2)`: 33.2 × 36 px on the iPad's 83 px strips, 36 × 36 from 86 px, 17 × 44 px at 64 px. Measured in mockups v4 and v5 with v8's `− 21px`: **36 × 36 px on the iPad's 83 px strips** (from 83 px up), about 32 × 36 px on a 79 px strip (a phone upright as the app stretches it), **20 × 44 px at the narrowest 64 px** (a phone on its side; 26.4 × 44 on a wide strip there). The drawn key is 2 px inside its box. It takes its own touches (`use:owns_touches`, no keys); the rest of the zone stays the fader's. Its press is ☰'s, unchanged (`behave/hold.rs`: a hold of 500 ms opens the detail, a tap shows the hint, a slide past 10 px ends it), and its flight-recorder `detail` steps keep their names. The hint (`hold SEL` / `for detail`) is drawn above the key, the strip's width less its edge and 37 px high, 4 px over it (the plan's ruling 7: the finger covers the key, not the zone above it). The pictogram is the word `SEL` in the condensed face (a console's select key, as on Yamaha and LV1 desks): the owner approved it on v4 (#151, 26.10); v4's variant B (three mini faders) is dropped.
- Considered and left out of the column: a peak number and a clip mark (v3, above), the pan position (it would need the pan subscribed on every strip; the L/R bars show its effect, and it is in the channel detail), gain reduction (not in the LOM), automation state (shown by the REVERB/AUTO marks).

The whole zone stays the fader's touch target (TouchOSC's 1:1); the column, the scale and the bars take no touch. The meter moves as today: a cover's `scaleY` and a peak layer's `translateY` per bar, no layout per frame (#21).

### 6.4 The name button (mockup v5, the owner's feedback, #151, 26.10)

**The head's layout (v8, the owner on v7, #151, 26.10: the names looked glued to the button's bottom edge, and the marks sat ABOVE the name, the wrong order).** The name button IS the strip's top edge: the strip's 5 px top padding goes, and the button reaches over the strip's 4 px side padding to its 1 px edge (the full strip width less the edge), its top corners the strip's (8 px), 37 px high (today 30). Read top-down: the NAME first, at the button's top (3 px from it, centred, a 20 px line, the size rule of `.strip-label` unchanged, up to 17 px); under it, toward the button's bottom edge (3 px from it), ONE line of the secondary marks, 10 px high, left to right: a problem's `!` (10 px), the linked mute's chain and count (`.link-mark`, link.css's look), the fader link's mark (the chain and `dB`), and the red `MUTED` at the right end; the corner line `.strip-corner` above the name is gone. The readout line under the button (the status light and Live's dB) is 20 px (today 22; the dB's 17 px line fits). The head is 37 + 2 + 20 = 59 px from the strip's top edge, as today's 5 + 30 + 2 + 22, so the fader travel is unchanged (measured in mockup v8: 322 px on the iPad, 292 / 327 px on the phones; the head 59 px on every screen). The track colour's edge is 3 px on the button's left. The touch stays the button's whole box (MUTE on the down, as today).

**When the marks line fills (pure, from what the strip carries, nothing measured, #21):** while muted the chain drops its count (it is red, and `MUTED` says it); with three marks or more the fader link's mark drops its `dB` (the chain alone, as today's `flink_compact`); with all four the gaps narrow to 2 px, and on a strip under 78 px the fader link's mark yields beside both `!` and a chain (as today, where it could not fit beside `!` and a chain under 88 px; PR 1's sweep found that a mixed two-digit count such as `10/12` beside `!` leaves it no room at 64 px with three marks too, so the rule is `!` and a chain, all four included). `!`, the chain and `MUTED` never yield. Under 78 px the chain and count take today's compact form (7.5 px). Mockup v8 checks every combination (none, a chain, a muted chain, a mixed `1/2`, `!`, muted, `!` + a muted chain of 12, the fader link leading, following with a chain, all four) at 83 and 64 px: every mark seen whole on the line, the name unclipped above it.

**The mute word's look (v8, the owner: v7's red was still too loud):** `MUTED` in a muted, darker red, `#a8474f` (v7: `--mute` `#e63946`), of lower contrast on the dimmed button and still read as red, at 7 px (v7: 7.5 px), 800, 0.5 px spacing (0.3 px under 78 px); text only.

Today the strip's name button (it is MUTE, #63) is filled with the Live track's colour (R4) and lights while the track plays. The owner: extremely loud and garish, its colours look random, mute is used rarely, and the point is that the fader's NAME reads well. So:

- **Unmuted:** the strip's dark ground (`#15151f` with the 1 px `--line` ring), the name in light, large text (`--text`, the condensed face at today's size, up to 17 px by `--n`), no colour fill. The Live track's colour stays only as a 3 px bar on the button's left edge (`--tc`; v8), so neighbouring strips stay tellable apart.
- **No RET mark (v7, the owner, #151, 26.10):** the return's `RET` mark (`.strip-ret`, #68, drawn by `MuteView`'s `Corner.ret` in `.strip-corner`) goes from every strip; the corner line keeps `!`, the linked mute's chain, the fader link's mark and the mute word. (Its sweep rows in `strip-head.spec.ts` go with it.)
- **One colour per strip (v6, the owner's question on #151, 26.10: why do the header and the name show different colours?):** today a strip carries two: the 3 px top edge in the layout GROUP's colour (`--gc`, `.strip::before`, #127) and the name in the LIVE TRACK's colour (`--tc`, R4). The decision: one colour, the Live track's own as set in Ableton (so the engineer changes a strip's colour in Live, nowhere else), drawn only as the name's 3 px edge (v8). The group's top edge is not drawn on a strip (`.strip::before` gone there; a block's top edge, `group.css` `.block::before`, is not a strip and is outside this note), and the group's name on the strip's side (`.group-side`, now in the information column, §6.3) is calm grey text (`--faint`), no longer teal or violet by its instance; it still names the group. The run title over a group whose run starts with no strip (#127, `.group-caption`) is outside the strip and unchanged. Mockup v6 checks every strip for no top edge and a grey group name.
- **Muted (v6, the owner's ruling on v5, #151, 26.10):** the strip RECEDES; nothing on it is brighter than on an unmuted strip. v5's loud muted look (a dark red fill, a struck-through name, `MUTED`) made the switched-off channel the most striking thing on the mix and is gone. As today's surface does it (`strip.css`: `--panel-mute`, the top edge at 0.3, the cap, the meter and the readout at 0.4; the SEL key at 0.6), and the name button goes darker than an unmuted one (`#101018`, the `--line-soft` ring), its name grey and of low contrast (`#55556e`), its colour edge at 0.3. **The mute word (v7, the owner's ruling on v6, #151, 26.10; its place and look v8's, above):** the marks line carries the red text mark at its right end (v8: `#a8474f`, 800 7 px, 0.5 px spacing; v7 had today's 7.5 px `--mute` in the corner), its word changed from `MUTE` to `MUTED`, so the state is read for sure; red text only: no fill colour, no badge box, no strike. An unmuted strip shows no mute word. `MUTED` never yields on the marks line (v8, above); mockup v8 checks that it is seen whole on every muted strip, at 64 px too. Mockup v6 checks on every screen that each muted name button is no lighter and its name of lower contrast than every unmuted one, with no text decoration. A linked mute's `mixed` state keeps today's chain mark and the button's unmuted look (not every track is muted).
- **Unchanged:** its tap (MUTE on the down, the mute guard, the linked mute, #127) and its intent outline (#43); its size and its marks are v8's (37 px, the strip's whole width, the marks line under the name, above). Only `.mute.lit`'s colours change; `--tt` (the text colour on the track colour) is no longer needed on a strip.

### 6.5 Texts (English, #141)

- The name button: the strip's name; the red `MUTED` at the right of the marks line under the name while muted (today: `MUTE`), nothing while unmuted; no `RET`, also not in the instance tag (the plan's ruling 5: a return's tag says its instance alone).
- The detail key: `SEL`; its hint `hold SEL` / `for detail`.
- On the strip: the number only (`−25`, `−18`), nothing at all when silent or missing (no `—`); on a strip of 78 px or more the small caption over the number while it shows: `LUFS IN` on an input point, `LUFS OUT` on an output point (two lines, §6.2). No unit beside the number. No PEAK and no CLIP text (v3).
- In the problem list: `no LUFS meter`, `two LUFS meters`, `bad LUFS tag`, `no meter device` (§3.4).
- The tag manual (`znacky.html`): `+LUFS:<target>` (an input point, `LUFS IN`) and `+LUFS` (an output point, `LUFS OUT`), with the states table, and the two devices with their places in the chain.

## 7. Protocol and data additions (summary)

- `fohmixer_proto::markers`: `+LUFS`, `+LUFS:<n>`; problems `DuplicateLufs`, `BadLufs`.
- `fohmixer_proto::layout::Strip`: `lufs: Option<LufsSpec { target: Option<i8> }>` (`deny_unknown_fields` stays); `LufsSpec::kind()` is `In` with a target, `Out` without.
- `fohmixer_proto::lufs`: `judge(tenths, target) -> LufsState` and the display integer; the constants `FINE_LU` 3, `FAR_LU` 6, `SILENT_TENTHS` −700; the caption of a kind. `lufs::wire`: the datagram's parser (§4.4), `DEFAULT_PORT` 39110, `WIRE_VERSION` 1. The reference DSP `lufs::dsp` is deleted (§9).
- `fohmixer_proto::meter`: P2's tables (Live's meter scale, the pan law) and the post-fader level of a meter device's reading (§3.4).
- The problem list: `NoLufsMeter`, `TwoLufsMeters`, `NoMeterDevice`.
- `ServerMsg::Lufs`, paced as meters.
- The hub's config: `lufs.port` (default 39110, 0 off).
- The output meter of an armed track (§3.4): the meter device's L/R, combined with the fader's dB, the pan law and Live's meter scale, served on the strip's own meter keys.
- No change to the Live script or its protocol: the confirming reads (§4.4) are ordinary reads.

## 8. Logging

- `lufs` records (info): the listener bound (port) or its bind failed (error); a key confirmed and matched (kind, device id, version, parents, instance, strip) or lost (stale: how long it lived, its last values); a key no instance confirms (once per key) or both do (warn); a key the bound starts dropping (warn, once per key until it is under the bound for 10 s); a problem raised or cleared.
- A strip's state change (`fine`/`warn`/`far`/`silent`/`none` on an input point; `plain`/`silent`/`none` on an output point) as one `lufs` record with the value, at most one per strip per 3 s (the window's length) *(call)*, so the forensics timeline (#43) shows "Vox 2 far −33 from 19:04:12" next to the transport. No time series of values; refused datagrams are counted in `/api/status`, never one record each.

## 9. Tests

- **The device's numbers against trueLevel (PC, an L check; the owner, 27.10):** on a played song, our `V` against Sonible trueLevel's short-term on two PA singers (the same signal, the same rack), within 0.5 LU, recorded on #151 (§10 step 7). *History: until 27.10 a Rust reference DSP (`fohmixer_proto::lufs::dsp`, EBU Tech 3341 cases, the gate cases) and a CI check of one coefficient table shared with the `.genexpr` stood here; PR 2 deletes the module and the table half of `scripts/check_lufs_device.py`.*
- **The devices' patches (CI, `scripts/check_lufs_device.py`):** the embedded code equals the `.genexpr`; the patch's shape (§4.2); no parameter object; `udpsend 127.0.0.1 39110` equal to `lufs::wire::DEFAULT_PORT`; no absolute paths; the face's version.
- **proto:** the tag (`+LUFS`, `+LUFS:-25`, `+LUFS:-5` bad, `+LUFS:x` bad, two tags), the composition into `Strip.lufs` and its kind, `judge` on every boundary (−22 fine, −21 warn, −19 warn, −18 far, −28 fine, −29 warn, −31 warn, −32 far, −70 silent, rounding of −21.5 and −22.4), the frame file's `lufs` field; the wire parser (a good datagram of each address, every refusal of §4.4, the parents' track and set); `meter` (P2's table points, the pan law at −1, 0, +1, a muted track's floor).
- **hub (SimLive):** SimLive gains the two Max devices (class `MxDeviceAudioEffect`) and sends their datagrams to the hub's port, 5×/s and 30×/s, from values a control line sets. A device on a `+LUFS` strip's track is confirmed and its reading reaches a page; the same device id and parents in both sims is matched to the instance whose set holds it; a device moved (new parents) is confirmed again; a frame strip by its track's name; stale after 1000 ms (nothing shown, one lost record); a flood held to 10/s per key; the listener bound to 127.0.0.1 only; refused datagrams counted; a missing and a doubled device give their problems; a disconnect clears; pacing and an idle page; `/api/status`. The mutation gate covers `judge`, the parser, the bound, the match and the pacing.
- **hub (armed tracks, §3.4):** SimLive's meter device's L/R plus the fader's dB and the pan law give the strip's output level on its meter keys; a muted track reads the floor; a device silent for 500 ms gives the keys back to Live's values; a non-armed track keeps Live's meters; an armed `+LUFS:<target>` strip without the device has `no meter device`.
- **E2E (Playwright, Chromium and WebKit iPad, the phone sizes):** a SimLive `Voice` of −25, −21, −17, −30, −33, −70 shows `−25` in the quiet grey, `−21` and `−30` in the muted amber, `−17` and `−33` in the muted red (the text's colour only: the reading has no background, border or box shadow), and for −70 nothing (no text, no caption, no box, the row's height kept); the caption `LUFS` / `IN` on the input strip and `LUFS` / `OUT` on an output strip at 78 px and wider, every caption line inside the column, none under 78 px; the output shows its plain `Short-term` in the quiet grey; on every `+LUFS` strip the LUFS text is smaller, lighter and of lower contrast than the strip's dB readout; the device stopped: the number goes within 1.5 s; an armed strip's two bars follow its fader (SimLive's meter device at a fixed level, the fader moved down 10 dB: the bars fall); the fader travel on the FOH fixture at 1194 × 834 is at least 322 px and a `+LUFS` strip's equals a plain strip's; every reading inside its box in the 64–120 px sweep; zero console errors. (PR 1's strip tests, the head, SEL, bars and scale, stay as they are.)

## 10. Rollout on the PC

1. **P1** (§4.3) on a set copy: done 27.10, the localhost path chosen.
2. **PR 1: the strip** (`docs/superpowers/plans/2026-10-26-strip-v8.md`): shipped as v1.17.0 (PR #153, 27.10).
3. **The `lr` re-test** (§3.4): passed 27.10; the site runs `meter_source: lr`.
4. **PR 2: the LUFS source** (`docs/superpowers/plans/2026-10-27-lufs-pr2.md`): the devices' senders, the hub's listener and match, the `+LUFS` tags, the `LUFS IN` / `LUFS OUT` text, the armed tracks' output meter; P2 (§4.5) runs during it with a draft meter device. Deployed: the hub listens; nothing sends until step 5, and a strip without a device shows nothing.
5. **The devices:** both patches built on the PC (`build_amxd.py`), opened once in Live's Max editor (the faces show the readings, `getpath` answers in the Max console), installed into both Live users' User Libraries by `Install-Fohmixer.ps1` (the installer already checks Live's User Library for the script).
6. **The set work (ours, memory "restart Live and do set work yourself", "save the Live set yourself"):** in the band set, a new chain `LUFS` in each PA singer's `CORE Effects` beside trueLevel with `FohMixer LUFS` in it, `FohMixer Meter` in a rack (an empty dry chain and the device's chain) last in each armed PA singer's chain, `+LUFS:-25` on each PA singer's Tuner. In the master set, `FohMixer LUFS` on the speakers' return and on the Vocals / Backing buses, each in a rack of its own (an empty dry chain and the device's chain; never in series, §3.2), `+LUFS` on their Tuners (or the frame strip's `lufs`). Each set saved with Ctrl+S. Not during an owner-declared event.
7. **Check:** every device in a rack's parallel chain, none in series (§3.2), and the sound unchanged; each rack's and track's `latency_in_samples` unchanged by the devices; Live's Undo menu shows no step from them after 60 s of playing; `/api/status` lists every device matched to its strip, none refused; a played song: our `V` against trueLevel's short-term on two PA singers within 0.5 LU (recorded on #151); the armed strips' bars follow their faders; Live's CPU before and after (all devices together); the hub log's busy lines with the transport playing (open since the `lr` re-test). Deployed and verified on the iPad on real Wi-Fi (memory "real-device acceptance").
8. **Acceptance with the owner:** a song with a mic at the right trim (quiet grey number under `LUFS IN`), a trim moved 5 dB (amber text) and 8 dB (red text), the singer silent (nothing shown), the output's number under `LUFS OUT` while a song plays.

## 11. Open questions for the owner

- **Q1. Bleed on the vocal mics.** With the band playing and the singer silent, how loud is the stage in a vocal mic after its trim? If it can reach −40 LUFS, the gate (−40) should go lower than the bleed would read as a voice, e.g. −36. A one-song look at `V` with the singer silent during the rollout's check (§10 step 7) answers it; the owner only confirms which level counts as "not singing". (P1 did not reach it.)
- **Q4 (answered, 26.10):** the detail key is `SEL` (variant A); B dropped. The LUFS is quiet text (v5).
- **Q2 (answered, 26.10):** no PEAK number and no clip light or clip state on the strips (Live mixes in 32-bit float); mockup v3.
- **Q3. Which tracks:** the PA singers, the speakers' return and the Vocals / Backing buses (the owner, 27.10: any output bus may carry `+LUFS`); also the master Live's hand mics (speech) with the same −25 target?
- **Q5 (answered, 27.10):** the surface says which loudness a number is, `LUFS IN` or `LUFS OUT`. Its two-line form is shown on mockup v9 (§6.2).
