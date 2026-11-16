# FohMixer Claps (Max for Live MIDI effect, #220)

Plays the hub's clap pattern on Live's own grid, so the claps are as tight as
Live's clock (the hub, ~30 Hz through the FohMixer script, could not send
notes in time). It is placed once on the Claps track, FIRST in its chain,
before the Instrument Rack: the arrangement clips' notes pass through it
untouched (`midiin` straight into `midiout`). No parameters.

- **The hub's words**, UDP on `127.0.0.1:39114` (OSC, sent at every change and
  again each second; the device acts only on a change):
  `/fohmixer/claps/notes <pitch…>` (the notes a clap plays: the Claps
  track's instrument picks its sound by the note), `/fohmixer/claps/pattern
  b1 b2 b3 b4` (1: that beat of the bar claps), `/fohmixer/claps/silent 0|1`
  (1 while an active clip covers the playhead: the clip claps, never twice),
  `/fohmixer/claps/on 0|1` (1 arms: it starts at the next bar's first beat;
  0 stops at once).
- **Live's beats:** a `metro 4n @quantize 4n` on Live's transport fires on
  every beat while Live plays (it is silent while Live stops); `transport`
  gives the bar's beat (a tick a hair before a beat counts as that beat).
  On beat 1 an armed device starts; on every beat while it runs and is not
  silent, a beat of the pattern plays each pitch with `makenote 100 8n`
  (velocity 100, half a beat, as the track's clips) into `noteout`.
- **Its hello**, UDP to the hub's `127.0.0.1:39113` each second:
  `/fohmixer/claps 1 v<version> <1 while it claps>` (`/api/status`
  `claps.device`).

Every decision (the taps, the beats, the clip silence, 4/4 only) is the
hub's (`crates/fohmixer-hub/src/rules/claps.rs`); the patch only plays.
`scripts/check_claps_device.py` (CI) holds the patch to it box by box. Built
like the other devices (never committed as `.amxd`), as a MIDI effect:

    python3 live-device/FohMixer-LUFS/build_amxd.py --patch "live-device/FohMixer-Claps/FohMixer Claps.maxpat"

`--port` and `--device-port` rewrite its two port boxes for another hub
config. `version.txt` is the workspace version (`scripts/check_version.py`).
The install writes it into each Live user's
`Presets\MIDI Effects\Max MIDI Effect\`.
