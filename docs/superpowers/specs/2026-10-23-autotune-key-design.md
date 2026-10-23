# Autotune key per song, switched by the hub (#140)

Status: draft for the owner's review. Every rule in §2 is the owner's, recorded on #140 between 2026-10-21 and 2026-10-23 with the owner's own words. The main session's calls are marked *(call)*. What is left to the builder is in §12. Nothing is built before the owner approves this note, and nothing on the surface is built before the owner approves mockup v2.

## 0. Zhrnutie pre vlastníka

**Čo sa stane**

- fohmixer hub na PC preberie celé riadenie autotune. Tablet ani stará reťaz v Abletone to už nerobia.
- Tóninu piesne berie z lokátora piesne `{F 95 4/4 …}`.
- Zmenu tóniny v piesni berie z klipu na stope Sections, napríklad `Chorus {G}`.
- Tóninu zapíše do každého Auto-Tune (stream cez AudioGridder, PA priamo) aj do lišty Live a spätne si ju prečíta.

**Kedy autotune ladí**

- Len keď je zapnutá AUT a zároveň band Live hrá, sme vnútri piesne, pieseň má tóninu a spevák nie je ručne vypnutý.
- Inak sa vyberie tvoj DRY chain.
- Pri stope sa autotune vypne hneď, aj v strede piesne. Vypne sa aj medzi piesňami a v piesni bez tóniny.
- AUT vypnutá znamená, že je vypnutý úplne. Tóniny sa však zapisujú stále, takže po zapnutí AUT sú hneď správne.

**Latencia**

- Autotune sa zapína a vypína len prepnutím Latencies chainu na tvoje vyladené cesty.
- Žiadne zariadenie sa nikdy nevypína, neodstraňuje ani nevymieňa. Všetky Auto-Tune ostávajú stále zapnuté.
- Pred prvým ostrým eventom sa latencia skutočne zmeria.

**Istoty**

- Kým tónina nie je zapísaná a spätne overená, spevák ostáva na DRY. Nezhoda sa ukáže na iPade.
- Zlá tónina nikdy nehrá potichu.
- Ak by fohmixer počas hrania vypadol, Live sa sám vráti na DRY.

**Na iPade**

- Tlačidlo AUTOTUNE ostáva vždy. Zapína a vypína AUT, ukazuje tóninu piesne a nezhody.
- Konkrétneho speváka vieš vypnúť ručne.
- Pred kódom dostaneš na schválenie maketu v2.

**Stará reťaz**

- kontrolný rack hlavnej speváčky, Max mapovače, automatizácia tónin a CC36 sa nepoužijú.
- Mapovania CC36 sú už zmazané (23.10.).
- Mapovače prestanú zapisovať pri nasadení, aby nezapisovali dvaja naraz. Odstránia sa až po prvom úspešnom evente, na tvoje áno.

**Nasadenie**

- Najprv testy v CI a E2E.
- Potom test AudioGriddera na PC, zmeranie latencie a skúška bez publika s band Live.
- Až potom prvý event s AUT zapnutou.

## 1. Why

**The old way.** The master singer's rack carries arrangement automation of two macros ("Tune KEY", "Scale"). Four Max for Live devices copy them into every other Auto-Tune by stored device IDs.

- The IDs break silently when a device is replaced.
- Several values sit on key boundaries.
- Every singer depends on one singer's rack.
- When a link breaks, the autotune pulls the singer into a wrong key, which is worse than no autotune.

The owner: "je to veľmi pracné udržiavať správne".

**The 2026-10-21 switch-off (post-mortem on #104).** It turned the plug-ins off instead of using the owner's delay-matched DRY chains. The songs' CC36 then put the stream selectors back on the autotune chain, which without the plug-in lacked the delay the DRY chain is tuned to. A stream singer was out of sync at the 2026-10-22 event.

**What #140 does instead.** It replaces both with one owner of the job: the hub reads the key from the set and switches only through the owner's chains.

## 2. Rules (the owner's, binding)

### 2.1 Where the key comes from

- **The song's key** is the first word of the song locator's only `{…}`: `NNN Name {KEY [tempo …]}`.
  - All 165 tagged songs follow this today (verification 2, #140 comment of 2026-10-21).
  - Flat and double spellings are translated by the hub (`Db`→C#, `Ab`→G#, `Gb`→F#, `F#/Gb` and `Gb/F#`→F#). No locator is renamed.
  - A trailing `m` is minor: the root plus Auto-Tune's "Minor", which is a natural minor per the Auto-Tune Artist manual.
- **A key change inside a song** is a clip on the MIDI track named `Sections` (AbleSet's section track, in `CONTROL grp`) whose `{…}` holds a key token and nothing else, e.g. `Chorus {G}`.
  - It applies from the clip's start to the next such clip in the same song, or to the song's end.
  - Where the switch must come before the singer's entry, the owner adds a new short Sections clip on the exact beat. A switch never happens while the singer is already singing.
  - The 8 changes the outside musician worked out, which live today only in the old automation, are converted once, on their exact beats:
    - 031 → G at beat 22897;
    - 086 → B at 50834;
    - 108 → C at 62431;
    - 133 → G# at 75136.6 (moved onto a beat), then A at 75169.5;
    - 153 → Fm at 86963.5, then Dm at 86990.5;
    - 158 → A at 89553.5.
  - The automation artefacts in 095, 123 and 148 are not converted *(call, open to the owner's veto)*.
- **A song's span** runs from its locator to the next locator that is neither a section (`>`) nor ignored (`*`). That locator must be `SONG END`, `STOP` or `AUTOSTOP`. Outside every span there is no key.
- **A song without a valid key** gets no autotune for its whole span. A key is never guessed, never taken from the old automation, and never carried over from the previous song.
  - Song 088 has no key; it is a dance song the owner removes.
- **Live's own key and scale** (the control bar, `Song.root_note` / `scale_name`) are a mirror only.
  - The hub writes them, so the bar shows the active song's key and Live's scale-aware devices follow.
  - They are never a source: in Live 12 they follow the selected clip, and Auto-Tune does not read them.

### 2.2 When autotune is on

There are two states only.

- **AUT on.** A singer's autotune chain is selected exactly while all of these hold:
  - the band Live plays;
  - the position is inside a span;
  - the span (or its latest key clip) has a key;
  - the singer is not switched off by hand;
  - the singer's key is written and read back (§5).

  Otherwise the DRY chain is selected.
- **Stop** selects DRY at once, even with the position in the middle of a song.
- **AUT off.** Every singer is on DRY, always. Autotune comes on only by switching AUT on. There is no manual "on" without AUT.
- **The keys follow the song position always,** with AUT on or off and with Live playing or stopped. Every Auto-Tune and Live's bar are then ready the moment AUT goes on, or a song starts.
- **A per-singer manual off.** That singer stays on DRY whatever the automation says.
  - It covers both of the singer's tracks, stream and PA.
  - It is kept across a hub restart.
- **The AUTOTUNE button is always on the surface.**
  - It switches AUT and shows the active key and any mismatch.
  - The gesture and the look are mockup v2's, for the owner's approval.
  - AUT is kept in `hub-state.json` across a restart, and is off after a first install.

### 2.3 How it switches (latency)

- **Only the `Latencies` rack's chain selector is moved,** between the owner's own chains.
  - Stream: the autotune chain (on value 127) and `DRY 64 + 58` (0). Its Align Delays are tuned to the AudioGridder path.
  - PA: `Effects 2.3` (0) and `DRY 2.3` (127).
- **A device is never switched off, removed or replaced** to turn autotune off.
  - A switched-off device's reported latency says nothing about the real delay. The 2026-10-21 failure proved it.
  - Every autotune device (the stream AudioGridders, the PA Auto-Tune Artists) stays on for good. The 3 PA Auto-Tune Artists still off (tracks 16, 17, 19) are switched on at the deployment.
- **Nothing else moves the selectors.** The CC36 mappings were removed on 2026-10-23, and the old AUTOTUNE `param_toggle` leaves the frame at the deployment.
- **A latency claim needs a real timing measurement,** or a path-by-path comparison against the owner's delay devices (§10).

### 2.4 Who

- **Stream, `+AUTOTUNE` (on 127, off 0):** the 5 singers whose stream rack has an autotune chain. One singer's stream has none and stays as it is.
- **PA, `+AUTOTUNE:0` (on 0, off 127):** all 6 singers.
- **The stream autotune stays in AudioGridder.** It runs the remote plug-ins on the PC's other chiplet, so the band Live keeps its low latency. Its Key and Scale are reached through AudioGridder's automation slots.
- **The PA autotune** is the native Auto-Tune Artist in Live.

### 2.5 Where and how well

- **It runs in the hub on the PC, never in the tablet.**
- **It is production-critical:**
  - deep CI and E2E coverage;
  - the mutation gate on every pure part;
  - the PC acceptance of §11 before the first event.

## 3. Today's code it builds on

The code map is in the main session's work products (`autotune-140-codemap.md`). In short:

**1.8.0's AUTOTUNE AUT (#104):**

- `+AUTOTUNE[:0]` tags (`markers.rs`) and a `{"kind":"autotune"}` rail control;
- the `Latencies` rack find (`rules/autotune/find.rs`), with writes of parameter 1 by `$ref`;
- a pure rule on the band Live's `is_playing` (`rules/autotune.rs`) and its router glue (`router/autotune.rs`);
- writes held until the markers settle (`reverb_settled`);
- `ServerMsg::Autotune`, `/api/status` `autotune`, and `hub-state.json` `autotune_aut`;
- the UI button (`behave/autotune.rs`, `components/rail.rs`).

**Not there yet:**

- The hub reads neither `cue_points`, `arrangement_clips`, `current_song_time`, `root_note` nor `scale_name`.
- The FohMixer script can get, set and observe any of them (`ops.py`; any `add_<prop>_listener`).
- SimLive models none of them.
- No locator parser exists.

## 4. The song map (pure)

A pure module (in the proto crate, beside `markers.rs`, or in the hub; *(call)*: the proto crate, so the UI could show the map later) turns the set's locators and Sections clips into a song map.

- **Input:**
  - the band set's cue points `(name, time)`;
  - the clips of the MIDI track named `Sections` `(name, start_time)`. AbleSet finds it by that name, so the hub follows AbleSet's convention here.
- **Output:** `SongMap { spans, problems }`.
  - Each span is `{ number, start, end, key: Option<Key>, changes: [(time, Key)] }`.
  - `Key` is `{ root: 0..=11, minor: bool }`.
- **Lookup:** `key_at(map, time) -> Option<(SpanRef, Key)>`, the key in force at a song position, or `None` outside a span or in a keyless span.
- **The grammar** is verification 2's §7: song locator, section key clip, spans, the errors. It is exact:
  - the second word after the key must be the tempo (a number), or the key stands alone, so `{A cappella}` is not A major;
  - names are trimmed as AbleSet trims them.
- **Problems are values, never panics,** as in the Tuner parser. They are shown on the surface and in `/api/status`, and are never silently dropped:
  - a duplicate song number;
  - two locators at one time;
  - two `{…}`;
  - a tag that does not parse;
  - a span not ended by `SONG END`/`STOP`/`AUTOSTOP`;
  - a key clip outside every span;
  - a Sections clip whose `{…}` starts like a key but holds more words.

  An affected song is keyless (no autotune), never guessed.
- **A song map report** lists every span with its number, span, key and changes (`/api/status`, and a CLI or forensics view, *(call)*). At the rehearsal it is compared with verification 2's table, so every locator is checked before the first event.

## 5. Keys: write and read back

- **Targets.** Each tagged track's `Latencies` rack is found as today. Inside it is the autotune device, the one in the rack's chain 0 *(call: true for all 11 racks today; the builder confirms)*. Exactly one is required, otherwise a problem, and that singer gets no autotune.
  - **PA:** the Auto-Tune Artist plug-in, with its parameters `Key` and `Scale`.
  - **Stream:** the AudioGridder plug-in, with its automation slot 0 (Key) and slot 1 (Scale), per the decoded plug-in state. Live labels them `0:(unused):(unassigned)` / `1:…`. The PC spike (§11) confirms the mapping, e.g. by `str_for_value`, before any write.
  - The master singer's rack holds a second, native Auto-Tune in a chain no value selects. It is not a target.
- **Values.**
  - Key: root `k` → `k/11` (12 steps, C … B).
  - Scale: Major `0`, Minor `1/28` (29 steps), as read on the PC.
  - Live's bar: `root_note = k`, `scale_name = "Major" | "Minor"`.
  - The builder reads each parameter's `min`/`max` and quantization on the PC and keeps the conversion in one pure, tested function.
- **When.**
  - The keys are written whenever the key in force changes: a new span, a key clip passed, a jump, a stop and start elsewhere, a set load, a reconnect, a target found anew (`$ref` changed).
  - Every stream key is also written again at every song start.
- **Read-back.** Every target's Key and Scale values are observed.
  - A PA value equals what was written: a real read-back of the plug-in.
  - An AudioGridder value is Live's side of the slot. The spike decides whether a change inside the remote Auto-Tune comes back there, and what a reconnect does.
  - A value that differs from the wanted key, or a write that failed, is a **mismatch**:
    - that singer's chain goes to DRY;
    - the mismatch is recorded and shown on the button;
    - the hub writes again, bounded *(call: once at once, then at the next key change; never a write loop against another writer)*.
- **One writer.** The old Max mapping devices write the same parameters. Their writes stop at the deployment, before the hub writes:
  - by removing the arrangement automation of the master's "Tune KEY"/"Scale" macros, or by turning the 4 mapping devices off;
  - the builder chooses with evidence that neither changes any audio path or latency (file diff, reported latency, and the rehearsal's listening).
  - A value that changes without the hub writing it is shown as "another writer", never fought.
- **Live's bar** is written with each new key and read back. A mismatch there is shown but gates nothing.

## 6. The position

- The hub observes the band set's `current_song_time` (the script flushes about 30 times a second while it plays) and `is_playing`, and keeps the song map current:
  - `cue_points` and their names and times;
  - the Sections track's `arrangement_clips` and their names and start times;
  - a change rebuilds the map.
- **A key clip's change is written when the position reaches the clip's start.**
  - The owner places such clips before the singer's entry: today 0.5 to 9 beats.
  - The hub may look ahead by the tempo *(call: write at the clip start less one tick, ~35 ms; never earlier than 1 beat)*.
  - Each change records its lateness against the clip's start, and its bound is checked in the tests and on the PC: at most 100 ms after the clip's start.
- **Jumps** (AbleSet, a click in Live) re-evaluate at once, playing or stopped.
- **The position read through the script is the only clock.** The hub never predicts the position from the tempo alone for longer than one tick.

## 7. The rule (pure)

1.8.0's `AutotuneAut` becomes a per-singer decision. It is a pure function, tested exhaustively, with a recorded `why`:

```
selected(singer) = aut
                 ∧ playing
                 ∧ key_in_force(position) is Some
                 ∧ ¬singer_off(singer)
                 ∧ key_confirmed(target)      // §5: written and read back
```

- **One write per change,** as STAGE AUT today. A selector already at the wanted value is not written again.
- **The order at a switch-on:** keys first, read back, then the selectors.
- **At a switch-off** the selectors go at once, with no wait for anything.
- **Writes are held while the instance's markers are not settled,** as REVERB and AUTOTUNE are today: a set load, a track-list change, a reconnect.
  - A hold that would leave a singer on autotune keeps the last decision only when that decision was DRY.
  - Otherwise the hub writes DRY through the last known `$ref` first. If that `$ref` is stale, nothing is written: the old track index never gets a write.
- **The `why` values:** `aut`, `play`, `stop`, `span`, `no_key`, `singer_off`, `mismatch`, `found`, `reconnect`, `jump`.

## 8. When the hub is gone

If the hub stops while autotune is selected, a stop would no longer select DRY, and speech between songs would be tuned.

- **Required:** the selectors go to DRY within 2 s of losing the hub, whatever the transport.
- **Mechanism *(call)*:** the hub registers its "safe" writes with the FohMixer script (the tagged selectors' `$ref`s with their off values). The script performs them when its hub connection drops, and records it in Live's log.
- The script's change is its own PR, with its tests. The hub re-decides on reconnect.
- A hub restart for a deployment therefore leaves every singer on DRY until the hub decides again.

## 9. The surface

**The AUTOTUNE button** (rail, mockup v2) shows:

- AUT on or off;
- the active song's key (`F`, `Am`), or `—` outside a song or in a keyless one;
- whether the singers are tuning now;
- a mark when any singer has a mismatch or a target problem, and the song map's problems.

**The gesture** that switches AUT is mockup v2's choice. 1.8.0 holds 1 s, and a tap no longer has a manual meaning.

**The per-singer off.** A singer is the PA strip with `+ID:<id>` and the stream track that carries `+MUTE:<id>`: #127's paired mute, the same person. The off is kept by that id in `hub-state.json`.

- Where it is switched is mockup v2's choice: on the singer's strip, or in a list opened from the button.
- The procedure (the owner's "postup") goes into the tag manual and the rail's help.

**Protocol:**

- `ServerMsg::Autotune` grows:
  - the key in force and the song number;
  - per singer: on, off, mismatch, problem, off by hand;
  - the map's problem count.
- `ClientMsg::SetHub` carries `autotune_aut` and the per-singer off (`autotune_singer {id, off}`).
- The old `autotune` tap message goes. A page that still sends it is answered with a refusal record.

**`/api/status` `autotune`:** the map report summary, every target's key read-back, and the last decision with its `why`.

## 10. Latency

- **Structure.** The only switch is the selector between the owner's chains, and every device stays on. That keeps the path the owner calibrated. The stream singer who stayed on DRY at the 2026-10-22 event was in sync.
- **The real measurement, before the first event, with the owner's yes and Live stopped.** It runs for each tagged rack: the time from input to the track's output on the autotune chain and on the DRY chain. The method is the builder's (a test signal through the rack with a recording, on the PC, out of the event). It is recorded on #140.
  - The two paths of a rack match within *(call: 1 ms)*.
  - A rack that does not match is reported to the owner, never "fixed" by the builder.
- A test reads `latency_in_samples` of every device in every tagged rack before and after a switch, to prove the hub moves nothing but the selector. This is a guard against a regression, not the latency proof.

## 11. Acceptance before the first event with AUT on

1. **CI green on every job,** the mutation shards with 0 missed. The Playwright E2E covers the button, AUT, the key shown, the per-singer off, a mismatch mark and the phones, with the console clean.
2. **The PC spike on AudioGridder** (the owner's yes, Live stopped, never during an event). It writes only a stream rack's slot 0 while its autotune chain is not selected, so nothing is heard. It answers:
   - Does the slot's `str_for_value` name keys?
   - Does a change in the remote Auto-Tune's window come back to the slot?
   - Does the remote plug-in keep the key across an AudioGridder reconnect?

   The remote window is looked at only, never clicked (#71's lesson). The answers go to #140 and decide §5's AudioGridder read-back.
3. **The set changes** (the owner's yes; saved with Ctrl+S, then read back from the saved file):
   - the 11 Tuner tags;
   - the 3 PA Auto-Tune Artists switched on;
   - the 8 key clips;
   - the old writers stopped (§5);
   - the frame edit: the old `param_toggle` out, the autotune control in.
4. **The latency measurement** (§10).
5. **A rehearsal without an audience, the band Live playing, AUT on.**
   - Every song is started: its key is checked on the surface, in Live's bar and in an Auto-Tune window.
   - The 6 songs with changes are played through each change.
   - A stop in the middle of a song selects DRY.
   - A keyless or out-of-span position selects DRY.
   - A per-singer off holds.
   - The song map report matches verification 2.
6. **The first event with AUT on.** The old Max mapping devices, the key automation and the hidden extra Auto-Tune are removed only after it, on the owner's yes.

## 12. Left to the builder

- Module boundaries and names. Where the song map lives (§4 suggests the proto crate).
- SimLive's additions:
  - `CuePoint`, `Clip`, `Song.cue_points`, `current_song_time` (advancing at the tempo while playing, settable), `root_note`, `scale_name`, `Track.arrangement_clips`;
  - the fixture keys and host lines for them;
  - plug-in Key/Scale parameters with items;
  - a parameter that can refuse or change on its own, to simulate a mismatch.
  - Chain-selector zones only if a test needs them.
- The tests:
  - the parser on synthetic locators covering every shape of verification 2 (no real names: public repo);
  - the rule's exhaustive table;
  - the hub host tests through SimLive: a song start, a stop mid-song, a jump, a key clip with its lateness bound, a keyless span, AUT on and off, the per-singer off, a mismatch, a reconnect, a set load and a track-list change under the settled hold, a `$ref` change, and the script's hub-loss fall back to DRY;
  - the E2E above.
- The `autotune` records (warn class): `map` (problems), `key` (span, key, why), `key_write`, `readback` (mismatch), `select` (singer, on, why), `late` (ms after a key clip). Plus a forensics timeline view of `autotune` and `transport` (with #130's PR 6 if it is still open).
- The PRs. A sensible order:
  1. the song map, with its report (read-only on the PC: the map is checked against the set before anything writes);
  2. keys written and read back, with AUT still off (the keys follow, nothing switches);
  3. the rule, the surface (after mockup v2) and the script's fall back;
  4. the PC steps of §11.

  Each is its own green PR, deployed and verified.

## 13. Non-goals and anti-goals

**Non-goals:**

- detecting a key from audio (Auto-Key);
- Live 12's Auto Shift instead of Auto-Tune (a later, separate decision);
- moving the stream Auto-Tune out of AudioGridder;
- any key logic in the tablet.

**Anti-goals** (a failure even if it "works"):

- a wrong key heard in silence;
- a device switched off to mean "off";
- a timing change of any singer's path;
- two writers of one Auto-Tune;
- a key carried from one song into the next;
- the hub fighting another writer in a loop;
- any surface element above the faders, or a change in the faders' height.
