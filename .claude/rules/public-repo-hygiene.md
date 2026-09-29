---
paths:
  - "**"
---

# Public-repo hygiene (spec §5.2)

This repository is public. These never enter it — not in code, tests, fixtures, docs, commit messages or PR bodies:

- host names, IPs and Windows user names (the Ableton PC's included);
- PINs, keys, tokens and certificates;
- people's names — also inside track names: the imported TouchOSC layouts hold real track names, so they stay on the Ableton PC (with dated backups), never here;
- the parity inventory;
- the `.tosc` and `.als` files.

What is public: the code, the import tool, and a **synthetic** `.tosc` fixture and layout (invented track names such as `Klavir #`, `Vox 1`; never a copied real one).

- `.mcp.json` is git-ignored (it carries the MCP servers' auth); never commit it or paste its values.
- Live checks on the Ableton PC (spec §5.3, "L") record their results on the tickets without host names, IPs or user paths.
- CI runs gitleaks over the full history (`secrets` job, `.gitleaks.toml`: the default rules; an allowlist entry needs a comment naming the fixture and why it is not a secret, a real secret is never allowlisted).
- Commits use the GitHub noreply identity: every clone sets it locally (`git config user.name` / `user.email` to the account's `…@users.noreply.github.com`), never the box's global identity. The commits before the S0 merge were made with a personal identity; the owner decided to keep them as they are (#3, 2026-09-29): history is never rewritten to remove it. Merge with plain `gh pr merge --merge` (GitHub then authors the merge with the noreply address).
- This follows the rule the owner set for the sibling public repo iemmixer.

## The private denylist scan (#3)

`scripts/denylist_scan.py` (ported from iemmixer) matches a private term list against the repo; it prints only a location and the entry number, never a term, a matched line or an email. A path component that holds a term is printed as `[redacted]` (the whole path when a term spans components). Find it locally with `git -c core.quotePath=false ls-files | grep -i -F -f <(sed -e 's/\r$//' -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' -e '/^#/d' -e '/^$/d' <private list>)` (for history, `git -c core.quotePath=false log --name-only --format=%h <range>` piped the same way) and never paste that output anywhere. Matching is case-insensitive on NFC-normalised text. Both modes judge a file by its whole blob: UTF-16 with a BOM is read as text, any other file holding a NUL is skipped as binary; `.gitattributes`, textconv, replace refs, grafts and the local git config cannot hide a commit's lines or paths. Each output line starts with `tree` or a commit's short SHA, never with a path.

- **The list** (people's names from the real track names, the PC's host name, Windows user names, LAN and tailnet names and addresses) is compiled by the main session from the site data. It lives only as the `DENYLIST` repo secret and a mode-600 file outside any repo. Never copy it, name its contents, or paste a term into the repo, a ticket, a PR body, a commit message or a log.
- **CI** (`secrets` job, after gitleaks): the whole tree at `HEAD`, and every commit after the kept legacy history up to the PR head (not the synthetic merge commit): paths, added lines, messages, author and committer names and emails. The job fails when the secret is missing, a fork PR included (a maintainer then runs the scan locally, and the merge push runs it again).
- **Legacy boundary:** `scripts/denylist-boundary.txt` pins the two last legacy commits (the S0 and S2 lines); their ancestors are exactly the 22 commits that keep their identity, and they are not scanned in CI. Only part of their content comes back: through the tree, and through the first-parent diffs of the later merges that brought the legacy lines in (a merge whose first parent is itself legacy does not re-diff the first legacy commits). So the main session scans the legacy commits' content once, locally, with the real list before it sets the secret: `--commits "b3d26d64f6ee89df8619197142f478a10d2922fa aca52f28a4489197d90b9f324043b41cda25c1ac"` without `--identities` and `--boundary` (a metadata hit there is the kept identity, by the owner's decision; a path or line hit is real site data). The boundary never changes: with `--identities` the scan fails if it would hide a commit with any allowed identity.
- **Identities:** every scanned commit must be authored and committed by an email in `scripts/allowed-identities.txt` (the account's noreply address and GitHub's own `noreply@github.com` for merges). The legacy identity is never written into any file.
- **A lane runs it before any push** (the wip backup included: every ref pushed to this public repo is readable), on its committed work, with the list path the main session gives it:

  ```
  python3 scripts/denylist_scan.py --denylist <private list outside the repo> \
    --identities scripts/allowed-identities.txt --boundary scripts/denylist-boundary.txt \
    --tree HEAD --commits HEAD
  ```

  Exit 0 = clean, 1 = findings, 2 = the scan could not run (an empty list, a missing or non-UTF-8 input file, a bad boundary line, a boundary commit missing from a shallow clone, a failing git command such as an unknown revision).
- **A hit is fixed by removing the site data**, never by allowlisting a real name. A hit in the tree goes with a new commit; a hit in a commit's message, author or added lines stays in that commit, so an unpushed branch is rebuilt without it (the main session decides how). An allow file (`scripts/denylist-allow.txt`, a line key from `--hash <path> <line>` plus a reason that never hints at the term, passed with `--allow`) is only for a reviewed line of invented or ordinary text that equals a term; none exists today.
