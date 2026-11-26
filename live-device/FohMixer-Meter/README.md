# FohMixer Meter (Max for Live audio effect, #151)

Sends its input's L and R peak levels to the FohMixer hub on `127.0.0.1`,
30×/s, with no parameters. Its output is silence: it goes in a rack of its
own (an empty dry chain plus the device's chain) last in an armed track's
chain, never in series (spec `docs/superpowers/specs/2026-10-26-lufs-design.md`
§3.2, §3.4). What it sends, how it is checked and built:
`../FohMixer-LUFS/README.md`.

## Pulled by the hub (#264, the design of 26.11.2026)

Until then the meter sent on its own clock: `peakamp~ 33` (L and R) outputs
every 33 ms on Max's scheduler, which Max for Live runs inside Live's audio
thread, and each output posted from there (the faces' number boxes redraw,
the `deferlow` before the `udpsend`; dasio measured 58 posts per 20 s on
the band Live). It now works as the LUFS device does since 1.58.0:

- the hub sends the meter's port `/fohmixer/meter/poll` every 33 ms (the
  LUFS devices' `/fohmixer/lufs/poll` stays at 200 ms);
- `obj-poll-receive` (`udpreceive 39999 @defer 1`, the port drawn at load
  into it and the `pack`'s last slot) hands the poll to Max's main thread,
  `route /fohmixer/meter/poll` → `obj-pull-gate` (Device On, observed, as
  the LUFS device's) → `obj-pull` (`t b b b`): first the re-read's count,
  then `obj-peak-l`, then `obj-peak-r`;
- the readers are `peakamp~` with no interval: Max's reference (read
  26.11.2026) says "By default, the interval is 0", and with 0 "the
  internal clock is not used, so peakamp~ will only output data when it
  receives a bang message", a bang sending "the greatest (absolute value)
  signal amplitude received since the previous report", i.e. over the
  poll's 33 ms;
- `atodb` → `clip -70. 12.` (the faces, fed from the poll only) → tenths →
  the `pack`, R through `t b f`, which bangs it; the `pack` goes straight
  into `udpsend`: wire version 2 (`,iisssssiii`, the port last);
- the identity is read as before (`live.path` per parent, `live.object`
  `get name`), but the load comes straight from `live.thisdevice` (no
  `deferlow`) and the 10 s re-read rides the poll: `live.thisdevice` →
  `random 300` → `prepend set` → `counter 0 299` (banged by the pull) →
  `sel 0` → `t b b b b` → the four parents' paths, every 300th poll (9.9 s),
  each device from its own place in the count, and before that poll's
  reading, so its datagram already names the new parents;
- nothing in the device runs a clock or hands a message over: no
  `deferlow`, `defer`, `metro`, `qmetro` or `delay`
  (`scripts/poll_reread_check.py` fails on any, in the LUFS device too
  since 1.60.3; `scripts/meter_pull_check.py` on a `peakamp~` with an
  interval and on a `udpreceive` without `@defer 1`).

**Open for the PC:** that `live.thisdevice`'s load bang comes in Max's main
thread (it is one message at load either way, not one per 33 ms); dasio's
stack trace with the new device (no `PostMessageW` stack on a Live audio
thread naming `peakamp~`, `number`, `udpreceive` or `deferlow` from the
meter); the hub's `lufs.listener.polled` growing by
about 30 a second per meter.

**A hub restarted under a running Live** (the DECIDED of 26.11 on #264): a
meter sends only when polled (and at its load, and when Device On turns 1),
and since 1.60.3 so does a LUFS device (its 10 s re-read rides the poll
too), so the hub, at its start, sends `/fohmixer/lufs/poll` and then
`/fohmixer/meter/poll` once to every port of the range the devices draw
from (40000–49999) on 127.0.0.1; every running device answers its own
address and is learned within its first poll, its `route` dropping the
other kind's. `/api/status` `lufs.listener.swept` / `sweep_failed` count
those sends, both kinds together (one info line too); a foreign program in
the range sees two unknown OSC datagrams once. On the PC: after a hub
restart, `lufs.listener.polling` counts every LUFS device and meter again
within a second.
