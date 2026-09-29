---
paths:
  - "crates/fohmixer-hub/**"
  - "crates/fohmixer-proto/**"
---

# Hub and protocol crates (learned on S3, #7)

## Code the mutation gate can judge

- **Never scan with a hand-stepped index** (`while … { pos += 1 }`). A `+=`→`*=`/`-=` mutant loops forever: the timeout recheck then fails the shard ("a hang"), and if the loop grows a `String`/`Vec` it eats the runner's memory ("The runner has received a shutdown signal", the whole shard lost). Count runs with iterators (`iter().take_while(..).count()`, `enumerate().skip(pos)`), and bound a scan loop by the input length (`for _ in 0..=len`). See `fohmixer-proto/src/path.rs` (`run_of`, `read_name`, `steps_from`).
- **A branch that only logs survives mutation.** Put the decision in a small pure function with a unit test and log through `if let Some(..) = decision(..)` (`first_of_outage`, `write_failure`, `read_problem`, `pin_warning`, `failure_note`).
- **One counter per kind of id.** A run id and request uuids taken from one counter make the run counter's `+=`→`*=` mutant equivalent (`live/names.rs`: `next_run`, `next_uuid`).
- **Nothing compiles locally, so mind trait bounds in tests:** `Result::unwrap_err` / `expect_err` need `T: Debug` — for a type that must not derive it (it holds a key, e.g. `acme::StoredAccount`) take the error with `.err().expect(..)` (#17 lost a CI cycle to E0277).
- **No struct-update base (`..X::default()`) in a literal the diff adds:** cargo-mutants 27's "delete field … from struct X expression" mutants ignore the recheck's `--re` and fail it with "tested a mutant it did not select". Spell every field out.

## Test harness (`crates/fohmixer-hub/tests/`)

- Host-backed tests start real `sim/host.py` processes: take `support::serial()`, mark the binary `#![cfg(unix)]`, and add a NEW binary to the `hub-hosts` group filter in `.config/nextest.toml`.
- `support::Host` lives in `support/host.rs` (Unix only: it stops hosts with `kill -s TERM`). `tests/layout.rs`, `tests/auth.rs` and `tests/config_cli.rs` need no host and also run in the `windows` CI job (the hub's platform): keep them host-free.
- `Client::next_message` reads in arrival order (pending first) — use it to assert ORDER (offline → online → fresh value); `wait` searches and cannot.
- A client that "never reads" only blocks the hub's writer when the kernel buffers are full: open it on a `TcpSocket` with `set_recv_buffer_size(4096)` and make the hub send megabytes (a `sub` with a ~200 kB bad prop is echoed twice in its `subbed`). Proof that the writer blocked: the client is closed no earlier than `SEND_TIMEOUT` after the burst.
- Expected `values` items come sorted by hub key: `bandx|…` sorts before `band|…` (`x` < `|`).
- **Asserting a log line (#9):** put the line in a small function and read it back in a unit test through a scoped subscriber: `ws/tests.rs` `logged(|| …)` (`tracing_subscriber::fmt()` with a capturing writer, `with_ansi(false)`, `without_time()`, `tracing::subscriber::with_default`). Its output is the hub log's shape: ` INFO <target>: <message> <field>=<value> …\n` (match with `contains`, never `starts_with`). A `%` value is unquoted; a `?` value or a bare `&str` is Debug-quoted. **Log a client's own text quoted** (`?`, as `ws` does for `origin` and `client_report` for its fields): unquoted, a space in it adds fake `key=value` pairs to its line. A test like that kills the "replace with ()" mutant of a function that only logs. For a line the real listener must produce, use the end-to-end form `tests/ws_log.rs`: a global subscriber (`set_global_default`), the binary's only test, then poll the captured log for the line.
- **A LAN peer in a test is a private address** (`10.0.0.5`, as in `routes.rs` `lan()`). The documentation ranges (192.0.2.x, 198.51.100.x, 203.0.113.x) are public to `access::is_private_ip`, so they are `internet`. Use them for a forwarded client or a port-forwarded peer.

## Design facts to keep

- Subscription table (`live/subs.rs`): a guard is its own entry, key `hub_key(..)|guard` (never a client's key, those end in `|true`/`|false`); a guard keeps its Live key when its re-resolution fails, so a rename back fires it again. A new `sub` of a name binding or of one in error resolves it again (the UI's REFRESH). `seq` is numbered across the table.
- Outbox: instance states are ORDERED with the replies (not coalesced); `offline` drops the instance's pending values.
- Live client: no request goes out before the script's `connect` names this instance; while connecting or offline a request is dropped (the caller reads "offline").
- Live client (#9): requests are written by a writer task of their own (`write_requests`); the session's `select!` loop only reads frames, ticks and queues, and never awaits a socket write. A write that waits (the script reads slowly) must not stop the heartbeat stamping, or the instance flaps busy while Live is fine (one of the two mechanisms the #9 flapping could come from; which one the PC hit is for its log to tell). `tests/live_client.rs` `deaf_script` (a 4 KB receive buffer, never reads, 16 MB of requests) proves it.
- Busy (#9): `busy_reason` is the one decision (150 ms tick age, 300 ms heartbeat overdue — never raise the 300 ms); the session logs it on every change, and `late_heartbeat` logs a heartbeat after an overdue gap with the script's `gap_ms` (long: the script did not make it; since #5 it is made in Live's main-thread tick, so a stall stops the heartbeats and the first one after reports it) and its time on the way (`ts` to `wall_ms`, one PC clock). Read those lines in the PC's `hub.out.log` before guessing where a late heartbeat was held.
- Layout schema 2 (#21): every struct and the `Control` enum have `deny_unknown_fields`; the internally tagged `Section` has none of its own (its newtype variants `Group` / `Pager` deny). One id namespace (pages, sub-pages, groups, pagers); `Page::controls` / `Layout::controls` are the one document-order walk the hub's `bindings()` and `stage_aut_binding()` use.
- `/api/status` carries `connect_failures`, `last_error` per instance and `layout.unresolved` (checked on layout accept and on connect; `live/names.rs` `layout_targets` checks the controls' bindings and, since #9, every `config.unfold` group as its track binding's target — a new name-bearing layout field belongs there too).
- Client reports (#26, `client_report.rs`): `POST /api/client-report` is public (before a login; Access guards the internet path), 10 KiB like `/api/client-error`. Only `fohmixer_proto::client::ReportFields` survive (serde drops the rest; a new field goes into `clean_fields`, the log line and the route tests' expected JSON — #5 K4 added `fps`, `long_frame_ms`, `touches_max`, `pointer`, all text from the page), each value `clean`ed (control characters out, 64 characters, `ua`/`error` 300), logged at INFO as `client report` with `peer`, `client` (the `cf-connecting-ip` of an internet request, `forwarded_client`) and `source` (`access::classify`), the newest 50 kept for `/api/status` `client_reports` — next to `clients`, the socket count, which keeps its meaning. A peer's budget is 60 per 10 s (`Budget`, at most 256 peers tracked); over it the report is dropped with one warn per window, and the answer is still 204 (a 429 is a console error on the page). The ring drops one per push with an `if`, never a loop a mutant could spin in.
- Socket lines (#9, `ws.rs`): `client connected` / `client disconnected` carry `client` (the socket number, as on every `ws` line), `peer`, `forwarded`, `source` and `origin`. `ws::opener` works them out from the upgrade's `ConnectInfo` and headers:
  - `source` is `client_report::source(access::classify(..))`, the one classifier.
  - `forwarded` is `client_report::forwarded_client` (`cf-connecting-ip` of an internet upgrade, cleaned), `"-"` without one, Debug-quoted. A client report calls the same value `client`; on a socket line `client` is the socket number.
  - `origin` is the page's `Origin` host through `access::origin_authority`, the Origin guard's parser, `clean`ed to 64 characters, or `-` without one. It is Debug-quoted on the line, and so is `source` (`source="lan"`, as in a client report).

  A new per-client field belongs on those lines too, never in a second classifier or parser.
