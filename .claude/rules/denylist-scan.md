---
paths:
  - "scripts/denylist_scan.py"
  - "scripts/test_denylist_scan.py"
  - "scripts/denylist-boundary.txt"
  - "scripts/allowed-identities.txt"
---

# The denylist scan's internals (#3)

How to use the scan and what a hit means: `.claude/rules/public-repo-hygiene.md`. This file is for changing `scripts/denylist_scan.py`.

## Output invariants (every change keeps them)

- A finding prints a location and an entry number only: never a term, a matched line, an email or git's own stderr (git failures become exit 2 with the subcommand and exit code, in `git()`).
- A path component holding a term prints as `[redacted]`, the whole path when a term spans components or the printed form holds one; kept components have control characters escaped.
- Each finding line starts with `tree` or a short SHA, never with a path: a path starting with `::` would be a GitHub workflow command in the CI log.
- stdout is flushed before the stderr count, so a combined log never cuts a finding.

## git's output depends on config, attributes and the object: pin it

- Commit mode takes paths and new blob ids from `git show --raw -z --no-abbrev` (exact path bytes), reads all of a commit's blobs with one `cat-file --batch`, and judges each file by its whole blob exactly like tree mode (UTF-16 with a BOM = text, a NUL = binary). Added lines come from the `-U0` patch parsed as bytes: a `+++` label is C-quoted when it holds a non-ASCII or special byte and gets a trailing tab when it holds a space; `+++ ` inside a hunk is content.
- `DIFF` pins `--text --no-textconv --no-relative --ignore-submodules=none --root -m --first-parent --no-renames` and fixed prefixes; `GIT_ENV` turns off replace refs and grafts. Each exists because a `.gitattributes` `-diff`, a textconv driver, `diff.relative`, `diff.ignoreSubmodules`, `log.showRoot=false`, a replace ref or a graft hid content in a probe.
- Commit metadata is read as stored (`cat-file commit`) and as rendered (`show --format`): an `encoding` header converts the rendering (a mislabelled one garbles a diacritic name, UCS-2 hides a message, UTF-7 renders a stored `dev+AEA-example.org` as `dev@example.org`). Identities are every stored `<email>` plus the rendered one; all must be allowed.
- Signature noise is only base64 and the exact BEGIN/END markers inside armour opened by a `gpgsig` header or a continuation-line marker; a `Comment:`, a name holding marker text and unknown headers are scanned.

## Tests

- Every term, name and address in the tests is invented: words like `zyxname`, `Zorblax`, `klávor`; addresses only from the RFC 5737 documentation ranges (`192.0.2.`, `198.51.100.`, `203.0.113.`); domains under `example.org`/`.net`. Never a private-range address (`10.`, `172.16.`, `192.168.`, `100.64.`) or a common first name: a value that only looks invented can be a real site value, and an earlier fixture was one.
- `raw_commit` / `write_commit` build odd commit objects (`hash-object`, `--literally` for ones fsck refuses).
- No mutation gate covers `scripts/`: after a change, run hand mutants of the changed lines against the suite on a scratch copy; a surviving mutant is a missing test (or a documented equivalent).
- The boundary (`denylist-boundary.txt`) is the two legacy tips, `b3d26d6` (S0) and `aca52f2` (S2): one SHA alone leaves the other line's 6 commits scanned. It never changes.
