---
paths:
  - "scripts/denylist_scan.py"
  - "scripts/test_denylist_scan.py"
  - "scripts/denylist-boundary.txt"
  - "scripts/allowed-identities.txt"
  - "scripts/denylist-accepted.txt"
---

# The denylist scan's internals (#3)

How to use the scan and what a hit means: `.claude/rules/public-repo-hygiene.md`. This file is for changing `scripts/denylist_scan.py`.

## Output invariants (every change keeps them)

- A finding prints a location and an entry number only: never a term, a matched line, an email or git's own stderr (git failures become exit 2 with the subcommand and exit code, in `git()`).
- A path component holding a term prints as `[redacted]`, the whole path when a term spans components or the printed form holds one; kept components have control characters escaped.
- Each finding line starts with `tree`, a short SHA or `accepted`, never with a path: a path starting with `::` would be a GitHub workflow command in the CI log.

## Accepted history (`--accepted`)

- A commit-mode `Hit` carries `key = (full SHA, path)`, or `(full SHA, None)` for a metadata hit (the list's literal `commit metadata`, so a file of that name can never be accepted by mistake). Tree hits have `key=None`; `IdentityProblem` and `BoundaryProblem` are not hits. `apply_accepted` marks matching hits `accepted` and adds an `AcceptedProblem` for every entry whose commit is not in the scanned set; only unaccepted findings count toward exit 1.
- The scanned set is the union of all `--commits` ranges after the boundary (a commit in two ranges is scanned once), so an entry for a legacy commit, a typo or an unreachable commit fails loudly instead of accepting something else.
- The path in an entry is the real path (the decoded bytes the scan keys on), never the printed `[redacted]` form.
- stdout is flushed before the stderr count, so a combined log never cuts a finding.

## git's output depends on config, attributes and the object: pin it

- Commit mode takes paths and new blob ids from `git show --raw -z --no-abbrev` (exact path bytes), reads all of a commit's blobs with one `cat-file --batch`, and judges each file by its whole blob exactly like tree mode (`read_content`: UTF-16 with a BOM = text, `.tosc`/`.als` inflated, a NUL = binary). Added lines come from the `-U0` patch parsed as bytes: a `+++` label is C-quoted when it holds a non-ASCII or special byte and gets a trailing tab when it holds a space; `+++ ` inside a hunk is content.
- `DIFF` pins `--text --no-textconv --no-relative --ignore-submodules=none --root -m --first-parent --no-renames` and fixed prefixes; `GIT_ENV` turns off replace refs and grafts. Each exists because a `.gitattributes` `-diff`, a textconv driver, `diff.relative`, `diff.ignoreSubmodules`, `log.showRoot=false`, a replace ref or a graft hid content in a probe.
- Commit metadata is read as stored (`cat-file commit`) and as rendered (`show --format`): an `encoding` header converts the rendering (a mislabelled one garbles a diacritic name, UCS-2 hides a message, UTF-7 renders a stored `dev+AEA-example.org` as `dev@example.org`). Identities are every stored `<email>` plus the rendered one; all must be allowed.
- Signature noise is only base64 and the exact BEGIN/END markers inside armour opened by a `gpgsig` header or a continuation-line marker; a `Comment:`, a name holding marker text and unknown headers are scanned.

## Compressed files (`.tosc`, `.als`)

- `read_content` is the one place that decides how bytes are matched, for tree mode, commit mode and `--hash` alike, so line keys agree. A regular file (`100644`/`100755`) named `.tosc` is a zlib stream, `.als` a gzip file (every member, zero padding skipped as Python's gzip reader skips it); the extension check is case-insensitive. A symlink with such a name is plain text (its blob is the target path). Both are matched whole, never through their (binary) diff.
- `inflate` feeds 64 KiB pieces (many gzip members stay linear) and stops at `INFLATE_CAP` (64 MiB): past it, empty, bad or truncated data, or bytes after a zlib stream give a `ContentProblem` (`unreadable compressed file` / `... inflates past the 64 MiB cap`, path only) that fails the scan. Never reject these files by extension: a synthetic `.tosc` fixture may be committed. An inflated file holding a NUL is binary like any other.
- Cost: `Scanner.entries_in` first searches one case-insensitive alternation of all terms (every boundary-checked match also matches it), and `lines_to_match` splits a whole file lazily, only when the whole text holds a term. A clean 64 MiB inflated file takes about 1 s and 224 MiB; one that holds a term can take minutes, with the same memory, and fails anyway.
- A commit-mode `ContentProblem` carries the same key as a hit, so a published one can be accepted. What did inflate is not matched, so look at the blob locally before listing it.

## Matching limit: joined words

- A term matches only as a whole word (iemmixer's boundary rule): a name glued to another word (`<Name>Vox`, a CamelCase join) is neither matched nor redacted. Main checked the real track names: no listed name is glued to another word. A new source of names that joins words needs the list (or this rule) revisited.

## Tests

- Every term, name and address in the tests is invented: words like `zyxname`, `Zorblax`, `klávor`; addresses only from the RFC 5737 documentation ranges (`192.0.2.`, `198.51.100.`, `203.0.113.`); domains under `example.org`/`.net`. Never a private-range address (`10.`, `172.16.`, `192.168.`, `100.64.`) or a common first name: a value that only looks invented can be a real site value.
- `raw_commit` / `write_commit` build odd commit objects (`hash-object`, `--literally` for ones fsck refuses).
- No mutation gate covers `scripts/`: after a change, run hand mutants of the changed lines against the suite on a scratch copy; a surviving mutant is a missing test (or a documented equivalent).
- The boundary (`denylist-boundary.txt`) is the two legacy tips, `b3d26d6` (S0) and `aca52f2` (S2): one SHA alone leaves the other line's 6 commits scanned. It never changes.
