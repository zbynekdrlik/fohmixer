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
- **No struct-update base (`..X::default()`) in a literal the diff adds:** cargo-mutants 27's "delete field … from struct X expression" mutants ignore the recheck's `--re` and fail it with "tested a mutant it did not select". Spell every field out.

## Test harness (`crates/fohmixer-hub/tests/`)

- Host-backed tests start real `sim/host.py` processes: take `support::serial()`, mark the binary `#![cfg(unix)]`, and add a NEW binary to the `hub-hosts` group filter in `.config/nextest.toml`.
- `support::Host` lives in `support/host.rs` (Unix only: it stops hosts with `kill -s TERM`). `tests/layout.rs` and `tests/auth.rs` need no host and also run in the `windows` CI job (the hub's platform): keep them host-free.
- `Client::next_message` reads in arrival order (pending first) — use it to assert ORDER (offline → online → fresh value); `wait` searches and cannot.
- A client that "never reads" only blocks the hub's writer when the kernel buffers are full: open it on a `TcpSocket` with `set_recv_buffer_size(4096)` and make the hub send megabytes (a `sub` with a ~200 kB bad prop is echoed twice in its `subbed`). Proof that the writer blocked: the client is closed no earlier than `SEND_TIMEOUT` after the burst.
- Expected `values` items come sorted by hub key: `bandx|…` sorts before `band|…` (`x` < `|`).

## Design facts to keep

- Subscription table (`live/subs.rs`): a guard is its own entry, key `hub_key(..)|guard` (never a client's key, those end in `|true`/`|false`); a guard keeps its Live key when its re-resolution fails, so a rename back fires it again. A new `sub` of a name binding or of one in error resolves it again (the UI's REFRESH). `seq` is numbered across the table.
- Outbox: instance states are ORDERED with the replies (not coalesced); `offline` drops the instance's pending values.
- Live client: no request goes out before the script's `connect` names this instance; while connecting or offline a request is dropped (the caller reads "offline").
- Layout schema: every struct/kind has `deny_unknown_fields`, except `Item` (it `#[serde(flatten)]`s its kind — serde cannot deny there; the internally tagged `ItemKind` and the boxed `Strip` deny instead).
- `/api/status` carries `connect_failures`, `last_error` per instance and `layout.unresolved` (checked on layout accept and on connect).
