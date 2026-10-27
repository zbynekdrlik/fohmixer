# FohMixer Meter (Max for Live audio effect, #151)

Sends its input's L and R peak levels to the FohMixer hub on `127.0.0.1`,
30×/s, with no parameters, and passes the audio through unchanged. It goes
LAST in an armed track's chain (spec `docs/superpowers/specs/2026-10-26-lufs-design.md`
§3.4). What it sends, how it is checked and built: `../FohMixer-LUFS/README.md`.
