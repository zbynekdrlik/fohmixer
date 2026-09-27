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
- Commits use the GitHub noreply identity. Merge with plain `gh pr merge --merge`.
- This follows the rule the owner set for the sibling public repo iemmixer.
