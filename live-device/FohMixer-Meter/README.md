# FohMixer Meter (Max for Live audio effect, #151)

Sends its input's L and R peak levels to the FohMixer hub on `127.0.0.1`,
30×/s, with no parameters. Its output is silence: it goes in a rack of its
own (an empty dry chain plus the device's chain) last in an armed track's
chain, never in series (spec `docs/superpowers/specs/2026-10-26-lufs-design.md`
§3.2, §3.4). What it sends, how it is checked and built:
`../FohMixer-LUFS/README.md`.
