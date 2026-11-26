# FohMixer Claps (Max for Live MIDI effect, #220)

Plays the hub's clap pattern on Live's own grid, so the claps are as tight as
Live's clock (the hub, ~30 Hz through the FohMixer script, could not send
notes in time). It is placed once on the Claps track, FIRST in its chain,
before the Instrument Rack: the arrangement clips' notes pass through it
untouched (`midiin` straight into `midiout`). No parameters.

- **The hub's words**, UDP on `127.0.0.1:39114` (OSC, sent at every change and
  again each second; the device acts only on a change), taken by
  `udpreceive 39114 @defer 1`, which outputs in Live's main thread (#264):
  `/fohmixer/claps/notes <pitch…>` (the notes a clap plays: the Claps
  track's instrument picks its sound by the note), `/fohmixer/claps/pattern
  b1 b2 b3 b4` (1: that beat of the bar claps), `/fohmixer/claps/silent 0|1`
  (1 while an active clip covers the playhead: the clip claps, never twice),
  `/fohmixer/claps/on 0|1` (1 arms: it starts at the next bar's first beat;
  0 stops at once), and `/fohmixer/claps/poll` (no argument, each second,
  with or without a +CLAPS track: the device answers with its hello).
- **Live's beats:** a `metro 4n @quantize 4n` on Live's transport fires on
  every beat while Live plays (it is silent while Live stops); `transport`
  gives the bar's beat (a tick a hair before a beat counts as that beat).
  On beat 1 an armed device starts; on every beat while it runs and is not
  silent, a beat of the pattern plays each pitch with `makenote 100 150`
  (velocity 100, 150 ms: half a beat at 200 bpm; a length in ms, so a stop of
  Live's transport never freezes a note-off) into `noteout`.
- **Its watchdog:** every `on` word from the hub (sent each second with the
  whole state while the hub has a +CLAPS track) restarts a `delay 3000`;
  when it runs out (the hub gone) the device stops and forgets its switch,
  so the hub's next `on 1` arms it again. A poll alone does not restart it.
- **Its hello**, UDP to the hub's `127.0.0.1:39113`, as the answer to each
  poll (#264): `/fohmixer/claps 1 v<version> <1 while it claps>`
  (`/api/status` `claps.device`; the hub shows it gone 3 s after the last
  one) straight into `udpsend`, in the poll's main-thread call. The poll
  (`t b b`) first shows the running state on the face: it is kept in an
  `i` whose cold inlet the arm and the stop set, so the face never redraws
  from the beats (a number box fed from the scheduler posts from Live's
  audio thread). *History:* until 1.60.3 the hello came from its own
  `metro 1000` through a `deferlow` (#240), the `udpreceive` had no
  `@defer 1` and the face was fed from the beats: each of them posted from
  Live's audio thread (dasio's probes on #264).
- **The one clock** is the beats' `metro 4n`: its MIDI out to Live posts
  nothing. `scripts/check_claps_device.py` fails on any `deferlow`, `defer`
  or `qmetro`, any other `metro`, a number box fed from a scheduler box and a
  `udpreceive` without `@defer 1`.

Every decision (the taps, the beats, the clip silence, 4/4 only) is the
hub's (`crates/fohmixer-hub/src/rules/claps.rs`); the patch only plays.
`scripts/check_claps_device.py` (CI) holds the patch to it box by box. Built
like the other devices (never committed as `.amxd`), as a MIDI effect:

    python3 live-device/FohMixer-LUFS/build_amxd.py --patch "live-device/FohMixer-Claps/FohMixer Claps.maxpat"

`--port` and `--device-port` rewrite its two port boxes for another hub
config. `version.txt` is the workspace version (`scripts/check_version.py`).
The install writes it into each Live user's
`Presets\MIDI Effects\Max MIDI Effect\`.
