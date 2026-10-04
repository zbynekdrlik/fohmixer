---
paths:
  - "tools/forensics/**"
---

# Forensics timeline (#43, design note §5.3)

- **What it is:** `tools/forensics/timeline.py`, the command, with three modules beside it: `timeline_read.py` (values, times, reading the logs), `timeline_model.py` (the analysis and the summary), `timeline_report.py` (the HTML). Stdlib only (Python 3.11+); the four files always travel together (the command finds the others through its own folder on `sys.path`). It draws a time window of the hub's event log (`events-YYYY-MM-DD.jsonl`, one per UTC date) and the hub's text logs (`hub.out.log`, `hub.out.<stamp>.log`, for `Live busy changed` and late-heartbeat lines, ANSI colours stripped) as ONE self-contained HTML file: inline CSS and SVG, no script, no external resource. A report like "fader 3 jumped at 19:40" is answered from the logs alone.
- **CLI:** `python timeline.py --events <logs folder> --from <local time> --to <local time> [--key TEXT ...] --out report.html`. Times are the local time of the machine running it (`YYYY-MM-DD HH:MM[:SS[.fff]]`, `T` allowed, or `HH:MM[:SS[.fff]]` for today). `--key` keeps the controls whose key holds TEXT (any of them); the link and Live lanes and `records` stay whole. Exit 1 with one `timeline: ...` line on a usage or input error; exit 0 with a report also when the window is empty.
- **What it reads:** only the day files of the UTC dates from `--from` − 1 min to `--to` + 1 min, line by line, parsing only lines whose `"ts":` is in range. Records count from 60 s before `--from` (offsets, busy episodes that started earlier) to `--to`; a `trace` up to 60 s after `--to` (its page events can fall inside). A line that does not parse is counted in `skipped_lines`.
- **Page time → hub time:** `t + offset`, where the offset is the `offset_ms` of the ping of the trace's own `client` nearest in time to the trace record. Fallbacks: the nearest ping of any client, else 0 (with a "page clock not aligned" note). A `set` carries its own `offset_ms`. Exact duplicate page events (a batch resent after a lost socket) count once, compared with sorted keys.
- **Gaps:** in each control row (page sends, hub arrivals at `hub_ms`, Live's applied values at the `applied` record's `ts`), two consecutive points more than 100 ms apart inside one gesture are a `gap`. A gesture runs from a touch `down` naming the key to its pointer's `up`/`cancel`, plus 1000 ms. A key no touch names (a page before the recorder): points up to 10 s apart count as one gesture.
- **Jumps (volume keys only, TouchOSC's `value2db`, over 3 dB between consecutive applied values), first cause that holds:**
  - `link`: the largest arrival gap is over 100 ms while the page's send gap is at most 100 ms, or a dropout overlaps the jump. Both gaps are measured from the one before S1 to S2: the hub's setter writes the first set after a stall alone, so S1 can be the first of the burst.
  - `live`: the batch's `rtt_ms` or S2's wait at the hub is over 100 ms, or a busy episode overlaps.
  - `page`: a send gap over 100 ms after S1, or a long `frame` overlaps.
  - `move`: none of these.
- **stdout:** `name=value` lines, the same names as the report's summary cells (`data-k`): `records`, `pages`, `confirmation_*` (applied `ts` minus the page's send on the hub clock; nearest rank), `worst_confirmation_at/_key`, `dropouts`, `longest_dropout_*`, `resets`, `rtt_page_*`, `rtt_hub_*`, `gap_{send,arrival,applied}_max_ms`, `busy`, `busy_longest_ms`, `late_heartbeats`, `jumps`, `skipped_lines`, `notes`.
- **Hygiene:** stdout never names a key: a control is `key#<10 hex of sha256(key)>`, so the numbers can go on a public ticket. The HTML holds real track names. The tool refuses an `--out` inside a git checkout (any `.git` entry in the folder or above it), and the header shows file names only, never a folder path, peer or address.
- **On the Ableton PC:**
  - copy all four files into one folder there (e.g. `airuleset.py share`, `Invoke-WebRequest`), checking one SHA-256 per file;
  - run `timeline.py` from that folder with the PC's own Python over the hub's data folder `logs`, with the PC's local times;
  - keep the HTML on the PC, or bring it to `~/.claude/work-products/` on the dev box (never into a repo); hand it to the owner through `share`;
  - over a remote shell, read only the stdout summary.
- **E2E:** the harness's `POST /forensics/timeline {from_ms, to_ms, key?}` runs it over the harness data folder's `logs` (local times from `datetime.fromtimestamp`) into a temporary folder and answers `{exit, stdout, stderr, html}`.
- **Tests:** `python3 -m unittest discover -s tools/forensics -p 'test_*.py'` (`test_timeline.py` imports each name from the module that owns it; synthetic logs in temp folders, the fixture's invented names only, times through the tool's own `local_text` so any TZ passes, plus one run under `TZ=XYZ-05:45`); `ruff format --check tools`.
