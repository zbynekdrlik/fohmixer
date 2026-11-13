# The MBC Ableton in fohmixer — Implementation Plan (#184)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The MBC Ableton becomes a third Live instance, `mbc`, of the same fohmixer: a hub on the MBC PC that works with the FOH PC off, the FOH hub reaching it through an authenticated instance link, the MBC faders in the four sectors' pool, per-strip and `STREAM LUFS` loudness with #151's look, and PIN roles (`mbc` reaches only MBC, `engineer` everything).

**Architecture:** One binary, two running copies. The MBC hub owns the MBC Live's script (127.0.0.1) and its LUFS listener; it serves its own four-sector page and an instance link on the LAN. The FOH hub's instance `mbc` is a *linked* instance: its reconnecting Live client dials the link (a bearer JWT per connection) instead of 127.0.0.1, and the link relays a script connection of its own plus the MBC LUFS datagrams. Roles are a pure module in `fohmixer-proto` (`roles.rs`) enforced at three choke points of the hub: the HTTP routes, the WebSocket's inbound messages, and the WebSocket writer's outbound messages.

**Tech Stack:** Rust (axum 0.8, tokio, tokio-tungstenite 0.28, jsonwebtoken 9, argon2), Leptos 0.7 PWA (Trunk), SimLive (`sim/host.py`) and the FohMixer script (Python 3.11), Playwright (Chromium + WebKit iPad), PowerShell 5.1 for the PC install.

**Spec:** `docs/superpowers/specs/2026-11-12-mbc-instance-design.md` (§6.1 transport, §6.2 set, §6.3 sectors, §6.4 roles, §6.5 script, §6.6 rollout); #184's ROZHODNUTÉ comments (the latest win: 2026-11-13 mockup v3 approved, MBC faders in the four sectors, no new page); mockup `docs/mockups/mbc-layout-v3.html`.

## Global Constraints

- Public repository: no host names, IPs, Windows user names, real track names, PINs, secrets or repo URLs in code, tests, fixtures, docs, commit messages or PR bodies. Strip names in fixtures are invented (`Hand 1`, `Moderator 1`, `Ambience`, `Vox 1`). Before every push run the denylist scan (`.claude/rules/public-repo-hygiene.md`) with exit 0.
- Commits use the repo's local noreply identity; the message names only `(#184)`; it ends with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Versions: one workspace version (`Cargo.toml` `[workspace.package]`, `live-script/FohMixer/version.py`, both `live-device/*/version.txt` and the device faces/`pack` that `scripts/check_lufs_device.py` checks). Feature = +0.1.0 over the version on `dev` at the PR's start (dev already ahead of `master` counts: when this plan was written `master` was 1.39.0 and #190 had taken 1.40.0 on `dev`), the FIRST commit of each PR; read it again at each PR's start, never reuse a version. Below, `vA` is PR A's version, `vB` = `vA` + 0.1.0, `vC`, `vD` likewise. `Cargo.lock`: only the `fohmixer-*` blocks without a `source` line (`.claude/rules/ci.md`). The bump commit carries `[no-test: version only]`; commit and push in separate calls.
- Never redesign tuned components: the strip, #151's LUFS display (`crates/fohmixer-ui/src/behave/lufs.rs` `look`, `fohmixer_proto::lufs::{judge, text, caption, in_band}`), the four sectors (#148, `crates/fohmixer-proto/src/sectors.rs`, `docs/superpowers/specs/2026-10-28-sectors-design.md`). Only what is new is added.
- Scope = today's MBC TouchOSC page (12 strips: volume, mute, pan in SEL's detail, meter, dB) + per-strip and stream LUFS + PIN roles + MBC working with the FOH PC off. No latency alignment, no `STREAM` fader, no sends/EQ/plug-in windows on MBC, nothing from recon.
- `127.0.0.1` stays the only bind of the FohMixer script (C1) and of the LUFS listener (C2, `[lufs] port` default 39110). The link binds the LAN address it is given, never `0.0.0.0`/`::` (C3).
- Ports: MBC script `39103`, instance link `39201`.
- Surface words are established English audio terms (#141): the login line reads `logged in: MBC` / `logged in: ENGINEER`, the button `LOG OUT` (DECIDED here, replacing the mockup's Slovak words; layout and place as the mockup).
- Tests: TDD; no skips; mocks only for external services. Hub integration tests are modules of `crates/fohmixer-hub/tests/it/main.rs`; a SimLive-backed file starts `#![cfg(unix)]`, takes `support::serial()` and is added to the `hub-hosts` group in `.config/nextest.toml`. Every UI feature has a Playwright spec run in both projects (`chromium`, `ipad`), importing `test`/`expect` from `./support/fixtures` (console clean). Mutation gate: a decision lives in a small pure function with a unit test; no `..Default::default()` in an added literal; no hand-stepped index loops.
- Live set changes on the MBC PC and every TouchOSC deletion are main-session steps (owner-approved, backup first, never during a stream; each deletion only with the owner's yes). They are listed under "Main-session steps", never as code tasks.

## Review Focus

1. **An MBC label equal to a FOH label.** `markers::compose` marks equal labels across instances as conflicts (pinned for band/master). An MBC Tuner named like a FOH strip must not turn either into a CONFLICT (a FOH strip would lose its sector place). Expected: a linked instance is its own label domain. Test in Task B6.
2. **A third Live on the FOH page's foot and column.** The foot keeps ≤ 34 px and every rail row its 44 px with three Lives (#178's `roomKept`); MBC strips appear in no VIEWS button and no page tab on the FOH page (mockup v3). Tests in Tasks B6 and B8.
3. **The link while the MBC side flaps.** MBC PC off, MBC hub restarting, the MBC Live closed, a wrong or old link secret: the FOH hub shows `MBC not running` on MBC strips only and never stalls its band/master routing (the link client runs in the instance's own task, like a local Live). Tests in Tasks B5, B7.
4. **A direct API call with an `mbc` token.** Every token-gated HTTP route and every WebSocket message type is either allowed for `mbc` on instance `mbc` only or refused (403 / `error`), table-driven over the whole list so a new route or message type without a role decision fails a test. Tests in Tasks A5, A6.
5. **The STREAM LUFS point on a track that is also a strip.** If the stream group's track carries a strip with `+LUFS` and the readout's `+CTRL:STREAM`, both read the one device (points dedupe by instance + target, `Layout::lufs_points`), the readout showing the strip point's reading. Test in Task C2.

---

## Decisions made in this plan (record on #184 as DECIDED)

- **The link relays its own script connection (no request-id mapping).** The FohMixer script serves several clients, each with its own listeners and results (`.claude/rules/live-script.md`). For each FOH link connection the MBC hub opens one more `ws://127.0.0.1:39103` connection and relays text frames verbatim both ways. The FOH hub's ids can never collide with the MBC hub's (separate connections), a fader moved on either tablet reaches both hubs through Live's listeners, and the FOH hub's client code stays the local one. This meets §6.1's contract (no collision, liveness exactly as a local Live) with less code than a mapping.
- **LUFS over the link:** the MBC hub's listener also hands every datagram it reads to a broadcast tap; the link writes each as `{"event":"lufs","osc":"<base64>"}`. The FOH hub's client parses it as `Frame::Lufs` and feeds it to the existing `lufs_udp::handle_datagram` with a `Bound` of the link's own. Readings are matched to points by their set's name (the rule already does, `rules/lufs.rs` `sets`), so no instance tag is needed.
- **A linked instance is its own label domain and its tag groups make pool-only pages** (`Page.pool_only`): in the FOH pool (ARRANGE), never a tab or a VIEWS button. On the MBC hub `mbc` is local and its frame places the `MBC` group as usual.
- **`STREAM LUFS` is a frame control** `{"kind": "lufs_readout", "label": "STREAM LUFS", "target": -14, "binding": {"instance": "mbc", "anchor": {"kind": "ctrl", "id": "STREAM"}}}` in the frame's `global`, resolved by the `+CTRL:STREAM` Tuner on the stream group (#104's mechanism), drawn under LAYOUTS/ARRANGE with #151's look.
- **The AbleSet/Presenter proxies are out of the role check:** they carry no fohmixer token today (LAN trusted, Access on the internet) and reach nothing of the PA that the LAN does not reach directly. The role check covers every token-gated route and every WebSocket message.
- **ARRANGE for the `mbc` role:** allowed on a hub whose sectors document holds only `mbc` faders (the MBC hub); refused (`role`) on the FOH hub, where the `mbc` role sees a derived document.

---

## File structure

| File | Responsibility | PR |
|---|---|---|
| `crates/fohmixer-proto/src/roles.rs` (+ `roles/tests.rs`) | `Role`, which instance a role reaches, the layout/sectors/colours/mute-alarm docs a role sees, whether a role may edit the sectors | A |
| `crates/fohmixer-hub/src/pin_store.rs` | `mbc` entry beside `engineer` | A |
| `crates/fohmixer-hub/src/provision.rs`, `src/bin/fohmixer-hub.rs` | `pin set-mbc`, equal PINs refused | A |
| `crates/fohmixer-hub/src/auth.rs` | role in the token, login against both hashes, `require_role` | A |
| `crates/fohmixer-hub/src/routes.rs` | per-route role decision, trimmed layout and status | A |
| `crates/fohmixer-hub/src/ws/role.rs` (+ tests) | inbound gate and outbound filter of one socket | A |
| `crates/fohmixer-hub/src/ws.rs`, `router.rs`, `router/sectors.rs`, `router/colors.rs`, `router/mute_alarms.rs` | wiring: the socket's role, `Attach { role }`, role-checked edits | A |
| `crates/fohmixer-ui/src/behave/login.rs`, `components/login_line.rs`, `store/live.rs` | the role from the hello, the login line and LOG OUT | A |
| `crates/fohmixer-hub/src/config.rs` | `InstanceCfg.link`, `[link]` (`LinkCfg`) | B |
| `crates/fohmixer-hub/src/link.rs` (+ `link/server.rs`, `link/tests.rs`) | the link secret (sealed), its JWT, the MBC hub's link server | B |
| `crates/fohmixer-hub/src/live/client.rs`, `live/mod.rs` | a linked instance's URL and header, `Frame::Lufs`, `LiveEvent::Lufs` | B |
| `crates/fohmixer-hub/src/lufs_udp.rs`, `lib.rs` | the datagram tap; link LUFS into the parser | B |
| `crates/fohmixer-proto/src/markers.rs`, `layout.rs` | label domains, `Page.pool_only` | B |
| `scripts/fohmixer-pc/mbc-frame.json` | the MBC hub's frame (synthetic, generic: tags and ids only) | B, C |
| `sim/fixtures/mbc-site.json` | SimLive's MBC set: 11 tracks (12 strips), a stream group, invented names | B |
| `e2e/harness/harness.py`, `e2e/tests/*.spec.ts` | the MBC hub and its SimLive in the harness; the specs | A–C |
| `crates/fohmixer-proto/src/layout.rs` | `Control::LufsReadout`, its LUFS point | C |
| `crates/fohmixer-ui/src/components/stream_lufs.rs`, `behave/lufs.rs` (one new pure fn), `pages/sectors.rs` | `STREAM LUFS` in the column | C |
| `crates/fohmixer-hub/src/script_watch.rs` | `script missing` (60 s, Live running) | D |
| `scripts/fohmixer-pc/Install-Fohmixer.ps1`, `FohmixerPc.psm1`, `Test-Fohmixer.ps1` | `-Site Mbc` install, `-MbcLink` on the FOH install | D |

## PR split (each ships working software)

- **PR A — roles and the second PIN (`vA`).** Ships alone: an `mbc` PIN can be set and logs in to a page that shows only MBC things (none yet), the engineer is unchanged. It is the security foundation the link must not ship without.
- **PR B — the instance link (`vB`).** Ships the MBC hub (MBC tablet works with the FOH PC off) and the FOH hub's linked `mbc`, whose faders the engineer can already place in a sector by ARRANGE; per-strip LUFS works through the link (the existing strip display).
- **PR C — STREAM LUFS and the MBC page's column (`vC`).** The one new readout and the MBC hub page as approved in mockup v3.
- **PR D — install on the MBC PC (`vD`).** The PowerShell path and `script missing`; then the main-session rollout.

---

## PR A — roles and the second PIN

### Task A1: Version `vA`

**Files:** `Cargo.toml`, `Cargo.lock` (fohmixer-* blocks only), `live-script/FohMixer/version.py`, `live-device/FohMixer-LUFS/version.txt`, `live-device/FohMixer-Meter/version.txt` and whatever `scripts/check_lufs_device.py` checks.

- [ ] **Step 1:** `git fetch origin && git merge origin/dev`; set every version to `vA` (the next minor; 1.41.0 if #190's 1.40.0 is the last on dev).
- [ ] **Step 2:** Run `python3 scripts/check_version.py` and `python3 scripts/check_lufs_device.py` → both pass; `git diff Cargo.lock` names only `fohmixer-*`.
- [ ] **Step 3:** Commit `chore: bump the version to <vA> (#184) [no-test: version only]`.

### Task A2: `fohmixer_proto::roles`

**Files:** Create `crates/fohmixer-proto/src/roles.rs`, `crates/fohmixer-proto/src/roles/tests.rs`; Modify `crates/fohmixer-proto/src/lib.rs` (`pub mod roles;`).

**Interfaces — Produces:**
- `pub enum Role { Engineer, Mbc }` (`Serialize`/`Deserialize` lowercase, `Copy`, `Eq`, `Debug`).
- `pub const MBC_INSTANCE: &str = "mbc";`
- `impl Role { pub fn from_sub(sub: &str) -> Option<Role>; pub fn sub(self) -> &'static str; pub fn reaches(self, instance: &str) -> bool; pub fn as_str(self) -> &'static str }` — `reaches`: engineer every instance, mbc only `MBC_INSTANCE`.
- `pub fn layout_for(role: Role, layout: &Layout) -> Layout` — engineer: a clone. mbc: every page's controls kept only when bound to `mbc` (strip binding, param fader's link/first target, `lufs_readout` from PR C); rails and `global` likewise; a group, pager or page left empty dropped; `default_page` the first page left (a layout with no page left keeps one empty page `mbc` titled `MBC`, so the UI has something to mount); `config` kept, `report` `Value::Null`.
- `pub fn sectors_for(role: Role, doc: &SectorsDoc) -> SectorsDoc` — mbc: each layout's slots keep only `mbc` refs (others become `None`, then `trim`), a layout left with no ref dropped unless it is its sector's only one (then emptied, title kept); `rev` unchanged.
- `pub fn may_edit_sectors(role: Role, doc: &SectorsDoc) -> bool` — engineer true; mbc only when `sectors_for(Mbc, doc) == *doc`.
- `pub fn reaches_strip(role: Role, strip: &StripRef) -> bool`.

- [ ] **Step 1: Write the failing tests** in `roles/tests.rs`: `the_engineer_reaches_every_instance_and_mbc_only_its_own` (`reaches("band")` false for Mbc, true for Engineer; `reaches("mbc")` true for both); `a_token_subject_names_a_role` (`from_sub("engineer") == Some(Engineer)`, `"mbc"` → `Mbc`, `"hub"`/`""` → `None`, `sub()` round-trips); `the_mbc_layout_keeps_only_mbc_controls` (a layout with a band strip, a master rail stage, an mbc strip in a view page and a band `global` alert → mbc gets one page holding the mbc strip, empty rail, empty `global`, `default_page` that page's id); `an_mbc_layout_with_nothing_left_has_one_empty_page`; `the_mbc_sectors_keep_only_mbc_faders` (lu `[band A, mbc X, None, mbc Y]` → `[None, mbc X, None, mbc Y]`; a sector whose only layout held band faders keeps it emptied); `mbc_may_edit_only_a_document_of_its_own` (an all-mbc doc → true; one band ref → false; engineer → true).
- [ ] **Step 2:** `cargo test -p fohmixer-proto roles` → FAIL (module missing).
- [ ] **Step 3:** Implement the module per the interfaces (iterators only; no index loops).
- [ ] **Step 4:** `cargo test -p fohmixer-proto roles` → PASS; `cargo clippy -p fohmixer-proto -- -D warnings` clean.
- [ ] **Step 5:** Commit `feat: the roles the hub enforces, as pure decisions (#184)`.

### Task A3: The `mbc` PIN

**Files:** Modify `crates/fohmixer-hub/src/pin_store.rs`, `crates/fohmixer-hub/src/provision.rs`, `crates/fohmixer-hub/src/bin/fohmixer-hub.rs` (USAGE, `["pin", "set-mbc"]`).

**Interfaces — Produces:**
- `PinFile { engineer: Option<String>, mbc: Option<String> }` (`#[serde(default)]` each; both checked by `check_phc`).
- `PinStore::hash(&self, role: Role) -> Option<&str>`, `PinStore::set_hash(&mut self, role: Role, phc: String) -> io::Result<()>`; `engineer_hash`/`set_engineer_hash` stay as thin wrappers (callers unchanged); `has_hashes` true when either is set.
- `provision::run_role(data_dir: &Path, input: impl BufRead, role: Role) -> Result<(), ProvisionError>`; `run` = `run_role(.., Role::Engineer)`. Setting a PIN that verifies against the OTHER role's stored hash is `ProvisionError::Invalid("the engineer and MBC PINs must differ")` (exit 2), checked with the hub's `PinHasher` before anything is written.
- CLI `fohmixer-hub pin set-mbc` (stdin), exit 0 stored / 2 invalid / 1 error, printed line `stored the MBC PIN hash`.

- [ ] **Step 1: Failing tests** in `pin_store.rs` tests: `the_mbc_hash_survives_a_reload_beside_the_engineers`; `a_plaintext_mbc_entry_is_a_load_error`; `an_older_file_with_only_the_engineer_loads` (`{"engineer": "$argon2id$…"}` → `hash(Mbc) == None`). In `provision.rs` tests: `an_mbc_pin_equal_to_the_engineers_is_refused` and the reverse (`set-engineer` equal to the stored mbc), each leaving the file unchanged. In `tests/it/config_cli.rs` (host-free, runs on Windows too): `pin_set_mbc_stores_a_hash_and_exits_0` and `pin_set_mbc_with_the_engineer_pin_exits_2`.
- [ ] **Step 2:** `cargo nextest run -p fohmixer-hub pin_store provision config_cli` → FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Same command → PASS.
- [ ] **Step 5:** Commit `feat: an MBC PIN beside the engineer's (#184)`.

### Task A4: The role in the login and the token

**Files:** Modify `crates/fohmixer-hub/src/auth.rs`, `crates/fohmixer-proto/src/client.rs` (`AuthResponse.role`).

**Interfaces:**
- Consumes: `Role` (A2), `PinStore::hash` (A3).
- Produces: `issue_token(secret: &str, now: u64, role: Role) -> Result<String, _>` (`sub` = `role.sub()`); `Auth::issue(&self, role: Role)`; `Claims::role(&self) -> Option<Role>`; `Auth::claims` returns `None` for a token whose `sub` names no role; `Auth::require(&self, headers) -> Result<(Claims, Role), Rejection>`; `pub fn login_role(engineer_ok: bool, mbc_ok: bool) -> Option<Role>` (engineer first; pure); `AuthResponse { token, expires_in, #[serde(default)] role: Option<Role> }`.
- Login: verify the PIN against both stored hashes in ONE hashing-gate permit (`HashGate`, two `verify_optional` in the blocking task), `login_role` decides; a failure is recorded once (`guard.record_failure`), a success logs `"{role} logged in"` with the peer.

- [ ] **Step 1: Failing tests** in `auth.rs`: `a_token_carries_its_role` (issue Mbc → `claims(..).role() == Some(Mbc)`); `an_engineer_token_of_before_stays_valid` (a token whose claims are `{"sub":"engineer",…}` signed the old way → `Engineer`); `a_token_naming_no_role_is_refused` (`sub: "hub"` → `claims` None); `the_login_names_the_role_its_pin_matches` (router test with both hashes stored: engineer PIN → `role: "engineer"`, mbc PIN → `role: "mbc"`, a wrong PIN → 401 `INVALID_PIN`); `a_wrong_pin_counts_as_one_failure` (`guard.stats()` failures +1 after one wrong attempt with both hashes set); `login_role` table: (true,false)→Engineer, (false,true)→Mbc, (false,false)→None, (true,true)→Engineer.
- [ ] **Step 2:** `cargo nextest run -p fohmixer-hub auth::` → FAIL.
- [ ] **Step 3:** Implement; update every `auth.issue()` / `require` caller (tests and tools: `rg 'auth\.issue\(|\.require\(&headers\)'`).
- [ ] **Step 4:** Same → PASS, plus `cargo nextest run -p fohmixer-hub` all green.
- [ ] **Step 5:** Commit `feat: the login names its role in the token (#184)`.

### Task A5: The HTTP routes by role

**Files:** Modify `crates/fohmixer-hub/src/routes.rs` (and its tests).

**Interfaces:**
- Produces: `fn route_role(path: &str) -> RouteAccess` where `enum RouteAccess { Open, AnyRole, EngineerOnly }` — the one table: `/api/version`, `/api/client-error`, `/api/client-report`, `/api/auth` Open; `/api/layout`, `/api/status`, `/ws` AnyRole; `/api/markers/migration`, `/api/eq/picture`, `/api/songmap` EngineerOnly. A 403 is `error_response(FORBIDDEN, "ROLE", "Not allowed for this login")` with one info line `role refused` (`role`, `path`, `peer`).
- `get_layout`: `roles::layout_for(role, &layout)`; `get_status`: `fn status_for(role: Role, status: HubStatus) -> HubStatus` (mbc: `instances` only `mbc`, every other field spelled out empty/none; no struct-update base).

- [ ] **Step 1: Failing tests** (router `oneshot` tests with `lan()`): `every_api_route_has_a_role_decision` — iterate the paths `api_routes()` registers (a const list `API_PATHS` beside the router, and a test that the router answers non-404 for each, so a new route without an entry fails); `an_mbc_token_gets_403_on_every_engineer_route` (table over `EngineerOnly` paths); `an_mbc_token_gets_only_mbc_in_the_layout_and_the_status` (a hub config with band, master and a local `mbc` instance; layout JSON holds no `"band"`/`"master"` binding; status `instances` names only `mbc`); `an_engineer_token_is_unchanged` (same layout as before).
- [ ] **Step 2:** `cargo nextest run -p fohmixer-hub routes::` → FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** → PASS.
- [ ] **Step 5:** Commit `feat: the HTTP routes by role (#184)`.

### Task A6: The WebSocket by role

**Files:** Create `crates/fohmixer-hub/src/ws/role.rs` (+ `ws/role/tests.rs`); Modify `crates/fohmixer-hub/src/ws.rs`, `crates/fohmixer-hub/src/router.rs` (`RouterMsg::Attach { role }`, per-client role kept beside `lan`), `router/sectors.rs`, `router/colors.rs`, `router/mute_alarms.rs`, `crates/fohmixer-proto/src/client.rs` (`ServerMsg::Hello { #[serde(default)] role: Option<Role> }`); Test: `crates/fohmixer-hub/tests/it/roles.rs` (+ `mod roles;` in `tests/it/main.rs`, `test(/^roles::/)` in `.config/nextest.toml` `hub-hosts`).

**Interfaces — Produces:**
- `pub fn inbound(role: Role, msg: &ClientMsg) -> Result<(), &'static str>` — engineer: always Ok. mbc: `Cmd`/`Sub`/`Set` Ok only for `instance == "mbc"`; `Sets` only when every item's instance is `mbc`; `Unsub`, `Ping`, `Trace`, `Idle`, `EqFrameAck` Ok; `SectorTick`/`SectorEdit`/`SetColor`/`SetMuteAlarm` Ok here (the router checks the document, below); `SetHub`, `Deck*`, `Presenter*`, `Eq*` (but the ack) refused. The `match` is exhaustive with no `_` arm, so a new message type is a compile error until it gets a decision. A refusal is answered with the existing `ServerMsg::Error` for the message's id (or none) and one rationed info line `role refused` (`ration::Ration`, 8 per 10 s).
- `pub fn outbound(role: Role, msg: ServerMsg) -> Option<ServerMsg>` — engineer: `Some(msg)`. mbc: `Values`/`Lufs` items kept only for `mbc` (a value's key through `hub_key`'s first field), `Instance`/`Link` only for `mbc`, `Sectors` through `sectors_for`, `Colors`/`MuteAlarms` entries only `reaches_strip`, `Hub`, `Deck*`, `Eq*` (all), `Ableset`, `Iem`, `Presenter*`, `Reverb`, `Autotune`, `Rec` dropped; `Hello`, `Result`, `Subbed`, `Layout`, `Ack`, `Pong`, `Error`, `Unknown` kept. Exhaustive, no `_` arm.
- `ws_handler` takes the role from the claims; `session` keeps it in `Conn`; `handle_text` calls `inbound` first; the writer task maps every message of a batch through `outbound` before `text(..)`.
- Router: `SectorEdit`/`SectorTick` from an mbc client are refused with the word `role` unless `roles::may_edit_sectors(Mbc, &stored_doc)`; `SetColor`/`SetMuteAlarm` refused (`role`) unless `reaches_strip`.

- [ ] **Step 1: Failing unit tests** in `ws/role/tests.rs`: `mbc_writes_and_reads_only_its_instance` (a `Set` for `band` → Err, for `mbc` → Ok; a `Sets` with one `band` item → Err); `mbc_may_not_touch_hub_values_deck_presenter_or_eq` (one case each); `the_engineer_is_never_filtered` (each `ServerMsg` sample → unchanged); `mbc_gets_no_foh_message` (an `Ableset`, `Reverb`, `Hub`, band `Instance`, `Values` with a band and an mbc item → only the mbc item survives); `mbc_gets_its_own_sectors_view` (a `Sectors` message → `sectors_for`'s doc). Router unit tests (`router.rs` test module, `attach` helper gains `role`): `an_mbc_arrange_save_on_a_mixed_document_is_refused_with_role`; `an_mbc_colour_for_a_band_strip_is_refused`.
- [ ] **Step 2: Failing host test** `tests/it/roles.rs` (three SimLive hosts band/master/mbc on `sim/fixtures/test-site.json` and a small `mbc` site, `TestHub` with a local `mbc` instance): `an_mbc_socket_cannot_move_a_band_fader` (send `set` for a band volume over an mbc-token socket → an `error`, SimLive's value unchanged via the engineer's `LiveClient`-style read); `an_mbc_socket_sees_no_band_or_master_state` (collect 2 s of messages: no `instance` naming band/master, no `hub`, no `reverb`); `an_mbc_socket_moves_an_mbc_fader` (value changed in SimLive).
- [ ] **Step 3:** `cargo nextest run -p fohmixer-hub ws::role router:: roles::` → FAIL.
- [ ] **Step 4:** Implement the module and wiring; `Hello.role` set from the socket's role.
- [ ] **Step 5:** → PASS; full `cargo nextest run --workspace` green.
- [ ] **Step 6:** Commit `feat: the WebSocket by role, in and out (#184)`.

### Task A7: The login line and LOG OUT

**Files:** Create `crates/fohmixer-ui/src/behave/login.rs`, `crates/fohmixer-ui/src/components/login_line.rs`; Modify `crates/fohmixer-ui/src/behave/mod.rs`, `components/mod.rs`, `store/live.rs` (`LiveStore.role: RwSignal<Option<Role>>` from each hello; `pub fn log_out(self)` = today's private `logout`), `pages/sectors.rs` and `pages/surface.rs` (`ColumnFoot` area per the mockup: under STREAM LUFS on the sectors page, above the foot), `style.css` or `sectors.css` (the mockup's `.who` and `.logout`); `.cargo/mutants.toml` (the component's glue, with a reason); `e2e/harness/harness.py` (the harness sets an MBC PIN with `pin set-mbc` beside the engineer's; its value in the harness only), `e2e/tests/support/live.ts` (`openSurface(page, { role: "mbc" })`), Create `e2e/tests/roles.spec.ts`.

**Interfaces — Produces:** `pub fn login_line(role: Option<Role>, one_instance: bool) -> Option<&'static str>` — `Some("logged in: MBC")` for Mbc, `Some("logged in: ENGINEER")` for Engineer only when `one_instance` (the MBC hub's page), else `None`; `one_instance` = every pool fader of the served layout has the same instance (`fohmixer_proto::sectors::pool`). LOG OUT: a button `data-testid="logout"`, `use:owns_touches` with no keys, acting on `pointerup` (like AbleSet's sign-in), calling `LiveStore::log_out`.

- [ ] **Step 1: Failing native tests** in `behave/login.rs`: `the_line_names_the_role_where_it_matters` (Mbc → MBC line both ways; Engineer + one instance → ENGINEER line; Engineer + several → None; None → None).
- [ ] **Step 2: Failing E2E** `roles.spec.ts` (both projects, the harness's FOH hub): `an MBC login shows only MBC things` (log in with the harness's MBC PIN through the login page; no `strip`, no rail button, no `deck-tab`/`ableset-tab`/`iem-row`, no band/master `badge`, `data-testid="login-line"` reads `logged in: MBC`); `LOG OUT returns to the login`; `the engineer's page has no login line` (the existing FOH page). Console clean.
- [ ] **Step 3:** `cargo test -p fohmixer-ui login` → FAIL; the spec → FAIL.
- [ ] **Step 4:** Implement.
- [ ] **Step 5:** Native tests PASS; `npx playwright test roles.spec.ts --project=chromium --project=ipad` against a local harness → PASS (`PLAYWRIGHT_BROWSERS_PATH=/opt/ms-playwright`).
- [ ] **Step 6:** Commit `feat: the login line and LOG OUT for the MBC login (#184)`.

### Task A8: Playbook and the PR

- [ ] **Step 1:** `.claude/rules/hub-rust.md`: a "Roles (#184)" bullet (the three choke points, the exhaustive matches, the `API_PATHS` table); `.claude/rules/deploy-pc.md`: `pin set-mbc` as the band user, like `set-engineer`; `ui-rust.md`: the login line.
- [ ] **Step 2:** Denylist scan (exit 0), `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`, commit, `git fetch && git merge origin/dev`, push `dev`, open PR `dev` → `master` (body ends with the attribution line), watch every job (push run and PR run) to green, merge with `gh pr merge --merge`, tag `v<vA>`.

---

## PR B — the instance link

### Task B1: Version `vB`

- [ ] As Task A1, `vB`, after `master` holds `vA`.

### Task B2: Config: a linked instance and `[link]`

**Files:** Modify `crates/fohmixer-hub/src/config.rs` (+ tests), `crates/fohmixer-hub/src/live/client.rs` (`LiveHandle::port` returns 0 for a linked one).

**Interfaces — Produces:**
- `InstanceCfg { name, #[serde(default)] port: u16, #[serde(default)] link: Option<String> }`; validation: exactly one of `port != 0` and `link` (`"instance {name:?}: port or link, not both"` / `"…: a port or a link"`); `link` passes `pub fn is_link_url(url: &str) -> bool` (`ws://`, a host or IPv4 address, an explicit port, no path beyond `/`, no credentials, no query); two instances may not share a link.
- `pub struct LinkCfg { pub listen: SocketAddr, pub instance: String }` as `Config.link: Option<LinkCfg>` (`[link]`); validation: `listen` not unspecified (`0.0.0.0`/`::`), port not 0, `instance` names a LOCAL instance (one with a port).
- Doc comment example at the file's top gains the `[link]` table and a linked instance (generic addresses from the documentation ranges only).

- [ ] **Step 1: Failing tests:** `a_linked_instance_has_a_link_and_no_port`; `port_and_link_together_are_refused`; `a_link_url_is_ws_with_a_port_and_nothing_else` (table: `ws://10.0.0.5:39201` ok; `wss://`, no port, a path `/x`, `user@`, `?q` refused); `the_link_listens_on_a_given_address_never_on_all` (`0.0.0.0:39201` and `[::]:39201` refused); `the_link_serves_a_local_instance_only`. In `tests/it/config_cli.rs`: `config_check_refuses_an_unspecified_link_listen`.
- [ ] **Step 2:** `cargo nextest run -p fohmixer-hub config` → FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** → PASS.
- [ ] **Step 5:** Commit `feat: a linked instance and the link's listen address in the config (#184)`.

### Task B3: The link secret and its token

**Files:** Create `crates/fohmixer-hub/src/link.rs` (+ `link/tests.rs`); Modify `src/lib.rs` (`pub mod link;`), `src/bin/fohmixer-hub.rs` (`["link", "set-secret"]`).

**Interfaces — Produces:**
- `pub const LINK_SECRET_FILE: &str = "link-secret.sealed";` in `<data>/secrets/`, written and read with `sealed::write` / `sealed::read` (DPAPI on Windows).
- `pub struct LinkSecret([u8; 32])` with a redacting `Debug`; `pub fn read_secret(input: impl BufRead) -> Result<LinkSecret, ProvisionError>` (one line, standard base64 of exactly 32 bytes, else `Invalid("the link secret is 32 bytes in base64")`); `pub fn store(data_dir: &Path, s: &LinkSecret) -> io::Result<()>`; `pub fn load(secrets_dir: &Path) -> io::Result<Option<LinkSecret>>`; CLI exit 0 / 2 / 1, `stored the link secret (sealed)`, never printing it.
- `pub const LINK_TOKEN_SECS: u64 = 60;` `pub fn link_token(s: &LinkSecret, now: u64) -> String` (HS256, `{"sub":"hub","iat":now,"exp":now+60}`); `pub fn link_claims_ok(header: Option<&str>, s: &LinkSecret, now: u64) -> Result<(), LinkRefusal>` with `enum LinkRefusal { NoHeader, BadToken, Expired, NotHub }` and `fn text(self) -> &'static str`.

- [ ] **Step 1: Failing tests:** `a_secret_is_32_bytes_of_base64` (44-char line ok; 31/33 bytes, not base64, empty → Invalid); `the_secret_survives_store_and_load` (Unix: `sealed` is plain there, as `cf_token`'s tests); `the_secret_never_shows_in_debug`; `a_fresh_hub_token_is_accepted`; `a_token_of_another_secret_is_refused`; `a_token_61_s_old_is_expired_and_60_s_is_not` (boundary); `a_token_for_another_subject_is_refused`; `no_bearer_header_is_refused`. `tests/it/config_cli.rs`: `link_set_secret_exits_2_on_a_short_secret`.
- [ ] **Step 2:** `cargo nextest run -p fohmixer-hub link::` → FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** → PASS (also in the `windows` job's `sealed::`-style run: add `link::` to the Windows test filter in `.github/workflows/ci.yml` beside `pepper::`, `sealed::`, `cf_token::`).
- [ ] **Step 5:** Commit `feat: the instance link's secret and token (#184)`.

### Task B4: The MBC hub's link server

**Files:** Create `crates/fohmixer-hub/src/link/server.rs`; Modify `crates/fohmixer-hub/src/lufs_udp.rs` (a tap), `crates/fohmixer-hub/src/lib.rs` (start the server beside the listeners when `[link]` is set; stop it in `HubInner::stop`); `crates/fohmixer-hub/src/routes.rs` / status (`/api/status` `link: Option<LinkStatus { listen, peer, connected_since, refusals, last_refusal }>` in `fohmixer_proto::client`, `#[serde(default)]`).

**Interfaces:**
- Consumes: `LinkCfg` (B2), `link_claims_ok` (B3), the local instance's script port.
- Produces: `pub fn spawn(cfg: LinkCfg, script_port: u16, secret: Option<LinkSecret>, tap: broadcast::Sender<Bytes>, events: EventLog, stop: watch::Receiver<bool>) -> io::Result<(SocketAddr, JoinHandle<()>)>`. Per connection: `tokio_tungstenite::accept_hdr_async` whose callback answers 401 for a `LinkRefusal` (and when no secret is stored: `"no link secret"`), 409 `busy` while a link is open (one at a time); then a WebSocket to `ws://127.0.0.1:<script_port>` within `CONNECT_TIMEOUT` (failure: the link is closed with code 1011 `script unreachable`, so the FOH side reads the instance offline); then two pumps: script text frames → link verbatim, link text frames → script verbatim, and each tap datagram → link `{"event":"lufs","osc":"<base64>"}` (a lagged tap drops, counted); a write that makes no progress for `LINK_STALL` (3 s, the script's `SEND_STALL_S`) ends the link; either side's close ends both.
- `lufs_udp`: `spawn(.., tap: Option<broadcast::Sender<Bytes>>)`; every datagram `recv` returned is sent to the tap before parsing (the FOH side parses and bounds it itself). Capacity `LUFS_TAP` 256.
- Each connect, refusal (with `LinkRefusal::text`) and drop is an info/warn line and an `instance_link` event record (`peer`, `why`), refusals rationed (`ration::Ration`, 8 per 10 s). Pure helpers carry the decisions: `fn admit(open: bool, check: Result<(), LinkRefusal>) -> Admission`, `fn lufs_frame(datagram: &[u8]) -> String`.

- [ ] **Step 1: Failing tests** in `link/tests.rs` (a fake script: a tokio WebSocket server that sends `connect` for `mbc`, echoes `result`s and pushes a value frame): `a_link_relays_the_scripts_frames_both_ways` (a request in, its result out, a pushed value out, byte-equal); `a_wrong_token_gets_401_and_a_record`; `a_second_link_gets_409_while_one_is_open`; `a_link_without_a_stored_secret_refuses_everything`; `a_dead_script_closes_the_link_with_1011`; `a_lufs_datagram_reaches_the_link_as_base64` (send a datagram to the tap → the frame's `osc` decodes to the same bytes); `a_link_that_never_reads_ends_after_the_stall` (a client on a 4 KiB receive buffer, a flood of values; closed no earlier than 3 s); `admit` and `lufs_frame` unit cases.
- [ ] **Step 2:** `cargo nextest run -p fohmixer-hub link::` → FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** → PASS.
- [ ] **Step 5:** Commit `feat: the MBC hub serves the instance link (#184)`.

### Task B5: The FOH hub dials a linked instance

**Files:** Modify `crates/fohmixer-hub/src/live/client.rs` (`run`), `crates/fohmixer-hub/src/live/mod.rs` (`Frame::Lufs(Vec<u8>)` in `parse_frame`), `crates/fohmixer-hub/src/lib.rs` (the event sink: `LiveEvent::Lufs` → `lufs_udp::handle_datagram` with the hub's `Counters` and a `Mutex<Bound>` per linked instance).

**Interfaces:**
- `fn instance_request(cfg: &InstanceCfg, secret: Option<&LinkSecret>, now: u64) -> Result<Request<()>, String>` (pure): a local instance `ws://127.0.0.1:<port>`; a linked one its `link` with `Authorization: Bearer <link_token>`; a linked one with no secret → `Err("no link secret: run fohmixer-hub link set-secret")` (a failed attempt like any other: counted, first warn, `last_error`).
- `LiveEvent::Lufs { osc: Vec<u8> }`; `parse_frame` maps `{"event":"lufs","osc":…}` (bad base64 → `Err`, the session's existing unreadable-frame path).
- The connect check is unchanged: the script's `connect` must name `mbc`.

- [ ] **Step 1: Failing tests:** `a_local_instance_dials_loopback_without_a_header`; `a_linked_instance_dials_its_link_with_a_fresh_token` (decode the header's JWT: `sub` hub, `exp - iat == 60`); `a_linked_instance_without_a_secret_fails_with_a_reason`; `parse_frame` `a_lufs_frame_carries_its_datagram` and `a_lufs_frame_with_bad_base64_is_unreadable`; in `lib.rs` tests: `a_lufs_event_reaches_the_router_as_a_reading` (a valid OSC datagram from `fohmixer_proto::lufs::wire` test helpers → `RouterMsg::Lufs`).
- [ ] **Step 2:** `cargo nextest run -p fohmixer-hub live:: lib::` → FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** → PASS.
- [ ] **Step 5:** Commit `feat: the FOH hub reaches a linked instance (#184)`.

### Task B6: Label domains and pool-only pages

**Files:** Modify `crates/fohmixer-proto/src/markers.rs` (+ `markers/tests.rs`), `crates/fohmixer-proto/src/layout.rs` (`Page.pool_only`), the hub's layout store caller of `compose` (`crates/fohmixer-hub/src/layout.rs`), the `layout check` CLI caller, `crates/fohmixer-ui/src/pages/surface.rs` (`ColumnHead`: tabs and VIEWS skip `pool_only` pages), `crates/fohmixer-ui/src/binding.rs` if it lists pages for selection.

**Interfaces:**
- `pub fn compose(frame: &Layout, found: &[Found], linked: &BTreeSet<String>) -> Composed` — conflicts are counted per `(domain, label)` where an instance's domain is its own name when it is in `linked`, else the shared domain `""` (band/master stay one domain: the existing test is unchanged). A tag group whose markers are all from linked instances and that the frame does not place becomes a view page with `pool_only: true`.
- `Page { …, #[serde(default, skip_serializing_if = "std::ops::Not::not")] pub pool_only: bool }`; `Layout::validate` refuses `pool_only` without `view`.
- The hub passes the config's linked instance names (`Config::linked_instances() -> BTreeSet<String>`).

- [ ] **Step 1: Failing tests** in `markers/tests.rs`: `an_mbc_label_equal_to_a_band_label_is_no_conflict_when_mbc_is_linked` (`"Vox 1"` on band and on linked mbc: neither marked); `band_and_master_still_conflict` (the existing test, `linked` empty, unchanged); `a_linked_tag_group_is_a_pool_only_view`; `a_local_mbc_group_the_frame_places_is_a_normal_group` (the MBC hub's case). In `sectors/tests.rs`: `the_pool_holds_a_pool_only_pages_faders`. UI native test in `pages/surface.rs` tests: `a_pool_only_page_is_neither_a_tab_nor_a_view` (the pure list function extracted from `ColumnHead`: `fn column_pages(layout: &Layout) -> (Vec<(usize, String, String)>, Vec<(usize, String, String)>)`).
- [ ] **Step 2:** `cargo test -p fohmixer-proto markers sectors && cargo test -p fohmixer-ui column_pages` → FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** → PASS.
- [ ] **Step 5:** Commit `feat: a linked Live's labels and tag groups stay its own (#184)`.

### Task B7: Two hubs, one MBC Live (host test)

**Files:** Create `crates/fohmixer-hub/tests/it/instance_link.rs` (`mod instance_link;`, `test(/^instance_link::/)` in `hub-hosts`); Modify `crates/fohmixer-hub/tests/it/support/` (a `TestHub` builder option for `[link]` and a linked instance, and the link secret written into both data dirs); Create `sim/fixtures/mbc-site.json` (11 tracks + one group, the stream group, a Tuner `"<label>" +G:MBC:<n>` on each of the 12 strips' tracks — one track with a second strip as a param fader `Hand 1 sync` — invented names only).

- [ ] **Step 1: Failing tests:** `a_fader_moved_on_either_hub_shows_on_both` (an engineer socket on each hub subscribes the same MBC volume; a `set` on the MBC hub's socket → the FOH socket's `values`, and the reverse); `the_foh_hub_reads_mbc_offline_when_the_mbc_hub_stops` (stop the MBC hub → FOH `instance mbc online: false` within the backoff; band still answers a `cmd`); `it_comes_back_by_itself` (MBC hub started again → online); `a_foh_hub_with_another_secret_stays_offline_with_the_reason` (`/api/status` `last_error` names the 401); `an_mbc_lufs_reading_reaches_the_foh_page` (SimLive `maxdev lufs track <i> add`, `lufs track <i> -230 -230` → the FOH socket's `lufs` item for that MBC point); `the_mbc_hub_runs_without_the_foh_hub` (no FOH hub at all: the MBC hub's own socket moves a fader).
- [ ] **Step 2:** `cargo nextest run -p fohmixer-hub instance_link::` → FAIL until B2–B6 are wired, then PASS; on failure follow `superpowers:systematic-debugging`.
- [ ] **Step 3:** Commit `test: two hubs and one MBC Live (#184)`.

### Task B8: The MBC hub in the E2E harness

**Files:** Create `scripts/fohmixer-pc/mbc-frame.json` (schema 2: one page `mbc` titled `MBC`, row 0 one group `{"tags": ["MBC"]}`, no rail, no `global` yet); Modify `e2e/harness/harness.py` (`--mbc-port 39103`, `--mbc-hub-port 8490`, `--mbc-link-port 39201`, `--mbc-lufs-port 39111`: a third SimLive host `mbc` on `mbc-site.json` with `--lufs-port` the MBC one, a second hub with `[[instances]] mbc port`, `[link] listen = "127.0.0.1:<p>"`, `[lufs] port`, the frame above, its own PINs (the same engineer PIN, the MBC PIN); the FOH hub's config gains `name = "mbc", link = "ws://127.0.0.1:<p>"`; one generated link secret written to both with `link set-secret`), `.github/workflows/ci.yml` (the e2e legs pass the new ports and `E2E_MBC_URL`), `e2e/tests/support/live.ts` (`MBC_BASE`, `openSurface(page, { base: MBC_BASE })`); Create `e2e/tests/mbc-link.spec.ts`; Fix the specs that assume two Lives (`rg -n '"band", "master"|badge' e2e/tests`) in their own commit, each change justified in the message (a third Live is now on the harness's FOH hub).

- [ ] **Step 1: Failing E2E** `mbc-link.spec.ts` (both projects): `the MBC hub's page shows twelve MBC strips in LU and RU` (on `MBC_BASE`: LU 6 + RU 6 `.slot[data-sector]` strips, LD/RD `MAIN` empty, no rail button, no extra tab, badge `mbc` only); `a fader moved on the MBC page moves on the FOH page` (ARRANGE on the FOH page: add a layout to RD holding two MBC faders, page RD to it, drag on the MBC page, read the FOH strip's value; put both back); `MBC faders are in ARRANGE's pool, not in VIEWS` (FOH page: no `view` button, the detail's tick rows list MBC strips); `with the MBC hub stopped the FOH page shows MBC not running on MBC strips only` (harness control `POST /mbc-hub/stop`, then `/start`; the band strips stay `data-live="on"`); `the FOH foot keeps its room with three Lives` (live-down's `roomKept` at 1194 × 834 and 844 × 390).
- [ ] **Step 2:** Run the spec against a local harness → FAIL before the harness change, PASS after.
- [ ] **Step 3:** Commit the harness + spec (`test: the MBC hub and its link in the E2E harness (#184)`), then the spec fixes (`test: the harness's FOH hub has a third Live (#184)`).

### Task B9: Playbook and the PR

- [ ] **Step 1:** `.claude/rules/hub-rust.md` "The instance link (#184)" (per-link script connection, the tap, the stall, the 401/409, the record); `.claude/rules/e2e.md` (the MBC hub, its ports, `MBC_BASE`, the harness control); `CLAUDE.md` playbook router: no new rules file needed unless a file grows past its scope.
- [ ] **Step 2:** As A8 Step 2; tag `v<vB>`.

---

## PR C — STREAM LUFS and the MBC page's column

### Task C1: Version `vC`

- [ ] As Task A1, `vC`.

### Task C2: `lufs_readout`

**Files:** Modify `crates/fohmixer-proto/src/layout.rs` (`Control::LufsReadout`, `lufs_points`, validation), `crates/fohmixer-proto/src/markers.rs` (a `Ctrl` anchor in `global` resolved like a rail's), `crates/fohmixer-proto/src/roles.rs` (`layout_for` keeps an mbc readout).

**Interfaces — Produces:**
- `Control::LufsReadout { label: String, target: i8, binding: Binding }` (`"kind": "lufs_readout"`; `deny_unknown_fields`); allowed only in `Layout.global` (`frame_problems`/`validate`: elsewhere `"lufs_readout only in global"`); `target` passes `lufs_target_ok`; `label` 1..=16 characters.
- `Layout::lufs_points()` adds one point per readout: its binding's instance and canonical track target, `LufsSpec { target: Some(target) }`, label the readout's; deduped with a strip point on the same `(instance, target)` (the strip's wins: its spec and label).
- `pub fn readouts(layout: &Layout) -> Vec<&Control>` (the `global` readouts, document order).

- [ ] **Step 1: Failing tests:** `a_readout_in_global_is_a_lufs_point` (instance `mbc`, target `-14`); `a_readout_outside_global_is_refused`; `a_readout_and_a_strip_on_one_track_share_one_point` (Review Focus 5); `a_readout_naming_a_ctrl_id_takes_that_tuners_track` (compose with a `Found` `"" +CTRL:STREAM` on mbc track 11 → anchor `TrackAt { index: 11 }`); `roles::layout_for` keeps an mbc readout and drops a band one.
- [ ] **Step 2:** `cargo test -p fohmixer-proto layout markers roles` → FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** → PASS; the hub's LUFS host test `tests/it/lufs.rs` gains `a_readout_point_gets_its_reading` (SimLive `maxdev lufs track <i> add rack` on the `+CTRL:STREAM` track, a reading → the `lufs` item of the readout's point) → PASS.
- [ ] **Step 5:** Commit `feat: a LUFS readout bound by a +CTRL id (#184)`.

### Task C3: `STREAM LUFS` in the column

**Files:** Create `crates/fohmixer-ui/src/components/stream_lufs.rs`; Modify `crates/fohmixer-ui/src/behave/lufs.rs` (one pure fn), `crates/fohmixer-ui/src/pages/sectors.rs` (mount under `ArrangeButton`), `crates/fohmixer-ui/src/pages/surface.rs` (the rail's foot skips readouts), `crates/fohmixer-ui/sectors.css` (the mockup's column cell: 30 px number on the FOH column, 40 px on a one-instance page; the number, caption and colours are `behave::lufs::look`'s classes, unchanged), `.cargo/mutants.toml` (component glue), `scripts/fohmixer-pc/mbc-frame.json` (`global`: the readout `STREAM LUFS`, `-14`, `mbc`, `{"kind":"ctrl","id":"STREAM"}`), `sim/fixtures/mbc-site.json` (`"" +CTRL:STREAM` on the stream group's Tuner), `e2e/harness/harness.py` (appends the same readout to the FOH hub's layout's `global` when the MBC hub runs); Create `e2e/tests/mbc-page.spec.ts`.

**Interfaces:**
- `pub fn stream_shown(instance: &str, picked: &[&SectorLayout], one_instance: bool) -> bool` — true when `one_instance`, or any picked layout holds a `StripRef` of `instance`.
- `StreamLufs` reads the readout's LUFS item from `LiveStore`'s LUFS map by its point key and renders `look(tenths, problem, Some(LufsSpec { target: Some(target) }))` with its label as the caption line; `data-testid="stream-lufs"`, `data-state` the `LufsState` name; no touch.

- [ ] **Step 1: Failing native tests:** `stream_shown` at the cases (one instance → true with nothing picked; FOH with RD on a layout of MBC faders → true; with RD back on a FOH layout → false).
- [ ] **Step 2: Failing E2E** `mbc-page.spec.ts` (both projects): `the MBC page shows STREAM LUFS under ARRANGE` (MBC hub; SimLive `lufs` on the stream group at `-14.4` → text `−14.4`, `data-state="in"`, 40 px number; at `-16.0` → `data-state="off"`); `each MBC strip shows its own LUFS` (a strip with `+LUFS:-18` in `mbc-site.json` shows #151's reading); `the FOH column shows STREAM LUFS only while a sector shows MBC faders` (RD paged to the MBC layout → shown at 30 px, the rail rows still 44 px at 1194 × 834; paged back → gone); `the MBC page's column matches the mockup` (LAYOUTS, ARRANGE, STREAM LUFS, login line, LOG OUT, `mbc` badge, version; no STREAM DECK, ABLESET, IEM MIXER, PRESENTER, no rail); every text inside its box (`clipped()`).
- [ ] **Step 3:** Native → FAIL; spec → FAIL.
- [ ] **Step 4:** Implement.
- [ ] **Step 5:** Native PASS; spec PASS locally in both engines.
- [ ] **Step 6:** Commit `feat: STREAM LUFS in the column (#184)`.

### Task C4: Playbook and the PR

- [ ] **Step 1:** `ui-rust.md` "STREAM LUFS (#184)" (the readout, `stream_shown`, sizes, the shared look); `deploy-pc.md`: the FOH frame's `global` gets the readout in both layout files (#51's rule) at deploy.
- [ ] **Step 2:** As A8 Step 2; tag `v<vC>`. After the deploy the main session walks every state of mockup v3's captions on the deployed pages (`ui-rust.md`'s live check).

---

## PR D — install on the MBC PC

### Task D1: Version `vD`

- [ ] As Task A1, `vD`.

### Task D2: `script missing`

**Files:** Create `crates/fohmixer-hub/src/script_watch.rs` (pure decision + a Windows probe); Modify `crates/fohmixer-hub/src/lib.rs`, the status (`InstanceStatus.script_missing: bool`, `#[serde(default)]`), the UI's `behave::live_down` word for it (`script missing` beside `not running`, the #178 look).

**Interfaces:** `pub const SCRIPT_MISSING_AFTER: Duration = Duration::from_secs(60);` `pub fn script_missing(live_running: bool, offline_for: Option<Duration>) -> bool` (true only when Live runs and the instance has been offline at least 60 s); the probe `fn live_running() -> bool` lists processes named `Ableton Live*` on Windows (`#[cfg(windows)]`, mutants-excluded glue), `false` elsewhere; checked every 5 s for local instances only.

- [ ] **Step 1: Failing tests:** `script_missing` at 59.999 s / 60 s, Live not running, never offline; `live_down` word table gains `ScriptMissing` → `script missing`; E2E is not needed (no Live process in CI); `live-down.spec.ts`'s static word check gains the text if it lists words.
- [ ] **Step 2:** → FAIL; implement; → PASS.
- [ ] **Step 3:** Commit `feat: script missing when Live runs without the FohMixer script (#184)`.

### Task D3: `Install-Fohmixer.ps1 -Site Mbc` and `-MbcLink`

**Files:** Modify `scripts/fohmixer-pc/Install-Fohmixer.ps1`, `scripts/fohmixer-pc/FohmixerPc.psm1`, `scripts/fohmixer-pc/Test-Fohmixer.ps1`; the bundle (`New-FohBundle`) carries `mbc-frame.json`.

**Interfaces:**
- `-Site Foh|Mbc` (default `Foh`: today's install unchanged). With `Mbc`: `-MbcUser <account>` (the MBC Live's user; the hub runs as it, as the band user on FOH), `-LinkListen <LAN address>:39201`, `-MbcPort 39103`, `-ScriptFolder <Live's built-in MIDI Remote Scripts folder>` (C5; the FohMixer folder copied there with `Config.py` `INSTANCE = "mbc"`, `PORT = 39103`; a dated copy of anything it replaces kept beside it), `-Layout` default the bundle's `mbc-frame.json` only when no `layout.json` is installed. `New-FohHubToml -Site Mbc` writes one `[[instances]]` (`mbc`, port) and `[link]` (`listen`, `instance = "mbc"`), never `[companion]`, `[ableset]`, `[iem]`, `[presenter]`; the firewall rule `fohmixer-hub-link` for the link port (LAN profile), the HTTP port's rule as on FOH.
- FOH: `-MbcLink ws://<MBC LAN address>:39201` adds the linked instance; an install without it keeps an installed linked instance (`Get-FohInstalledLinkedInstances`), like the remote tables.
- Every toml goes through `Test-FohHubToml` (the hub's own `config check`).

- [ ] **Step 1: Failing Pester-less checks** in `Test-Fohmixer.ps1` (the `windows` job): `-Site Mbc -NoTask` into a temp data dir writes a toml with exactly one instance and `[link]`, which `fohmixer-hub config check` accepts; `-LinkListen 0.0.0.0:39201` is refused by the hub's check before anything changes; the script copy writes `Config.py` with `mbc`/`39103`; a FOH install with `-MbcLink` adds the linked instance and a later one without it keeps it.
- [ ] **Step 2:** Push the red alone first (CI-only PowerShell, `.claude/rules/ci.md`), read the failures, then implement, then green.
- [ ] **Step 3:** Commit `feat: the MBC PC's install and the FOH install's link (#184)`.

### Task D4: Playbook and the PR

- [ ] **Step 1:** `deploy-pc.md` "The MBC PC (#184)": the install line (no host names: `<MBC LAN address>`, `<MBC user>`), `pin set-engineer`/`pin set-mbc`/`link set-secret` as the MBC user, the slot-2 selection, `script missing`, the rollback (deselect slot 2, disable the task); public-repo-hygiene unchanged.
- [ ] **Step 2:** As A8 Step 2; tag `v<vD>`.

---

## Main-session steps (not code tasks; in order, each recorded on #184 without site names)

- [ ] **M1 — FOH deploys.** Deploy each merged PR to the FOH PC by `deploy-pc.md` (after A: `pin set-mbc` as the band user with a new MBC PIN, given to the owner by `secret show`; after C: the FOH frame's `global` gets the `STREAM LUFS` readout in both layout files).
- [ ] **M2 — The link secret.** 32 random bytes in base64, generated on the dev box into a mode-600 file outside any repo; piped to `fohmixer-hub link set-secret` as the hub's user on each PC over MCP; never echoed or logged.
- [ ] **M3 — The MBC PC install (after D).** Install with `-Site Mbc`; PINs (the same engineer PIN, the MBC PIN); select `FohMixer` in a free control-surface slot (slot 2) while the TouchOSC script AbletonOSCMBC keeps slot 1 and keeps working; FOH install with `-MbcLink`; check: the MBC tablet's page loads from the MBC hub, the FOH page's `mbc` badge is on, `/api/status` on both, TouchOSC still moves the MBC faders.
- [ ] **M4 — The MBC set change** (owner-approved 2026-11-12; after M3, never during a stream): a dated `.als` backup first; read each metering rack's `latency_in_samples`; through the LOM (`insert_device`): a Tuner `"<today's TouchOSC strip name>" +G:MBC:<n>` on each of the 11 tracks in today's order (read the second strip of the one track, `… sync`, from the TouchOSC layout on the PC first: if it is a device parameter it becomes a `param_fader` control in the MBC frame, not a second Tuner); `+LUFS:<target>` on each with targets proposed from a read of today's levels and confirmed by the owner (the one owner question of this rollout, asked then); a `FohMixer LUFS` device in a parallel chain on each strip's track (a rack with a dry chain where there is none) and one in the stream group's existing metering rack; `+CTRL:STREAM` on the stream group's Tuner; latency read again (must be unchanged); save (`Ctrl+S` after marking the set modified, `live-script.md`), the file's time and content read back.
- [ ] **M5 — The FOH engineer's sectors.** In ARRANGE on the FOH page, an `MBC` layout (two of six, `MBC 1` / `MBC 2`) in RD as mockup v3 shows; the other sectors untouched.
- [ ] **M6 — A service with both.** The operator on fohmixer, TouchOSC still there as fallback; afterwards read the MBC hub's and FOH hub's event logs for that window (touches, drops, LUFS readings, link drops) and post the findings on #184.
- [ ] **M7 — Retire TouchOSC** after the owner confirms: AbletonOSCMBC out of slot 1, its folders, its ~6.5 GB log and ~4 GB of old copies — each deletion only with the owner's yes. The TouchOSC app on the operators' tablet is removed by whoever holds it.
