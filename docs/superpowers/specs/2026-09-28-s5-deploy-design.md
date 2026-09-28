# S5: deploy on the Ableton PC and the parallel run, design note

**Ticket:** #9. **Spec:** `docs/superpowers/specs/2026-09-28-fohmixer-program.md`:
- §2.1 processes;
- §6 S5 and the S1 checks K1, K2, K4;
- D7 LAN-only;
- D9 scheduled task;
- I7.

## 1. Goal

fohmixer runs on the Ableton PC next to TouchOSC.
- The hub is a logon scheduled task and serves the UI on the LAN.
- The FohMixer script is installed for both Live users.
- The layout is imported from the engineer's current TouchOSC project.

The engineer can then use the iPad app in parallel, and TouchOSC keeps working.

**Done means:**
- `/api/status` on the PC shows band and master online, not busy.
- The iPad's Home Screen app shows every core strip bound, with the real values.
- The K1, K2 and K4 results are recorded on #5.

## 2. Build: a Windows release bundle in CI

A `bundle` job runs on pushes to `master`, after `wasm` and `windows`. It builds `fohmixer-hub.exe` (release, `x86_64-pc-windows-msvc`, with the `wasm-dist` artifact embedded) and zips it together with:
- `live-script/FohMixer/`;
- `scripts/fohmixer-pc/*.ps1`;
- a `VERSION` file.

The zip is uploaded as the artifact `fohmixer-windows-<version>-<sha>`. As in iemmixer, only a hosted-CI build of `master` is ever installed on the PC.

## 3. Install script: `scripts/fohmixer-pc/Install-Fohmixer.ps1`

This is PowerShell 5.1. It is idempotent and never touches Live's preferences or a running Live. Parameters: `-BundleZip`, `-DataDir` (default `C:\ProgramData\fohmixer`), `-HttpPort 8480`, `-BandUser`, `-MasterUser`, `-BandPort 39101`, `-MasterPort 39102`, `-Layout`.

1. Stop the running hub gracefully if it is running: the scheduled task `End`, then wait for the process to exit, up to 10 s. Never force-kill (I7).
2. Unpack the zip to `<DataDir>\app\<version>` and point `<DataDir>\app\current.txt` at it.
3. Write `<DataDir>\fohmixer-hub.toml`: `http_port`, `instances = [{name="band",port=…},{name="master",port=…}]`, `layout = "<DataDir>\layout.json"`, `data_dir`.
4. Copy the layout file given by `-Layout` (the import output) to `<DataDir>\layout.json`. The hub then keeps the backups.
5. For each Live user, copy `FohMixer\` into `C:\Users\<user>\Documents\Ableton\User Library\Remote Scripts\FohMixer\` and rewrite `Config.py`'s `INSTANCE` and `PORT` for that user. An existing copy is replaced only when its `version.py` differs. Live reads the folder only at startup, so this is safe while Live runs.
6. Register the scheduled task `fohmixer-hub`:
   - logon trigger for the band user, `LogonType Interactive`, `RunLevel Limited`;
   - no time limit (`PT0S`), `MultipleInstances IgnoreNew`, no idle or battery stop;
   - action: `<DataDir>\app\<version>\fohmixer-hub.exe --config <DataDir>\fohmixer-hub.toml`, with the `FOHMIXER_HUB_DATA` environment variable.
   - Then start it.
7. Poll `http://127.0.0.1:<HttpPort>/api/version` until it answers, up to 20 s, and print the version.

`Test-Fohmixer.ps1` is a self-test (`scripts/fohmixer-pc/Test-Fohmixer.ps1`, run in the `windows` CI job): it installs into a temp `DataDir` with fake users' folders, checks the files, `Config.py`, the toml and the idempotence, and uses no real scheduled task (`-NoTask`).

## 4. On the PC (the S5 run), in order

1. **Claude, autonomous.**
   - Download the bundle artifact of the current `master` and copy it to the PC with the MCP file tools.
   - Run `Install-Fohmixer.ps1` with the layout from `C:\temp\fohmixer-import\layout.json`.
   - The hub starts. It shows both instances offline, because the script is not selected yet.
2. **Claude with the owner present.**
   - The risk: a Live restart can raise an unsaved-set prompt, and either answer loses something.
   - The step: in each Live, select `FohMixer` in a free control-surface slot (band slot 3, master slot 2), Input and Output `None`, then restart that Live the way it is normally started.
3. **Claude, autonomous.**
   - Check the status: both instances online and not busy.
   - Check `Log.txt` for FohMixer errors.
   - Run K1 and K2 with a measurement script over the hub protocol, and record them on #5:
     - K1: timer and drain delays, the longest command, heartbeat ages while the band set plays;
     - K2: CPU and main-thread lag with `output_meter_level` against `left/right` for all subscribed strips.
4. **The engineer on the iPad.** Owner action, because it is physical.
   - Open `http://<PC LAN address>:8480` in Safari, Add to Home Screen, log in with the PIN.
   - In Settings: multitasking gestures off, Auto-Lock Never.
   - K4: 4 or more faders at once, frame rate, the screen staying on, recorded on #5.
5. **PIN.** Claude generates a random 6-digit engineer PIN, runs `fohmixer-hub pin set-engineer` with it on stdin, and delivers it to the owner through `airuleset.py secret show` (a one-shot URL, owner action). It is never put in chat.

## 5. Rollback

- Deselect `FohMixer` in the Live slot. TouchOSC and AbletonOSC were never changed, so nothing else is needed.
- Disable the `fohmixer-hub` task.
- The data folder stays for the next attempt.

## 6. Not in S5

Removing AbletonOSC or TouchOSC (S6), HTTPS, internet access.
