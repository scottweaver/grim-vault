---
# Project skill bindings — consumed by ~/.claude/skills/daily-stand-up and ~/.claude/skills/wrap-up.
# Hand-edited rule file; the portable skills are project-agnostic, all the per-project values live here.
# Schema source of truth: ~/.claude/skills/bootstrap-project/SKILL.md

tracker: none

# Linear bindings (required when tracker: linear; ignored otherwise;
# commented out because tracker: none). Linear (kunai workspace) is reserved
# for Kunai/PwC OSS projects — grim-vault is personal and must not bind it.
# linear:
#   workspace: <slug>
#   team_prefix: <PREFIX>
#   assignee_id: <uuid>
#   state_uuids:
#     in_progress: <uuid>
#     in_review:   <uuid>
#     done:        <uuid>

# GitHub bindings (required when tracker: github; ignored otherwise;
# commented out because tracker: none). Pre-filled with the expected values
# so switching to GitHub Issues later is a two-line edit.
# github:
#   owner: scottweaver
#   repo:  grim-vault
#   assignee: scottweaver

# Project state file consumed by daily-stand-up (T discovery) and wrap-up (step-4 refresh).
# Set state_file.path to null to disable both behaviours.
# With tracker: none, {ID} matches free text to the end of the line — the task name itself.
state_file:
  path: .claude/rules/STATE.md
  next_up_patterns:
    - "**Next: {ID}**"
    - "**next up: {ID}**"
    - "Next up in Phase N: {ID}"

# Stand-up-only settings.
standup:
  no_blockers_sentinel: ":none:"
  # email_to: <address>             # uncomment to have daily-stand-up offer email delivery

# Wrap-up-only settings.
wrapup:
  docs_pr_branch_prefix: docs/state-post-
  docs_pr_commit_prefix: "docs(state):"
  auto_merge_carve_out_path: .claude/rules/
  state_refresh_authority: .claude/rules/METHODOLOGIES.md
  audit_doc: null
---

# Project bindings — grim-vault

Grim Vault: an OS-agnostic item vault manager for Grim Dawn, written in Rust
with egui. Sibling of TQ-AE-Univault (`~/Projects/tq-univault`), which it
should reuse from wherever possible. Bootstrapped 2026-09-03 before the git
repo, remote, or state file existed.

- **Tracker**: `none`. This is a personal project; the Kunai Linear workspace
  is reserved for Kunai/PwC OSS work and is deliberately not bound here even
  though `LINEAR_API_KEY` resolves on this machine. Stand-up drafts Y from
  git activity + STATE.md and T from the state file's next-up line as free
  text; wrap-up skips ticket verification. If the project lands on GitHub
  with Issues enabled, rerun `/bootstrap-project` (edit mode) and switch to
  `tracker: github` — the commented block above is pre-filled with the
  expected `scottweaver/grim-vault` values.
- **State file**: `.claude/rules/STATE.md`, the rehydration document every
  session reads first (created by `/bootstrap-agent-rules` on 2026-09-03).
- **Wrap-up**: bound with the standard defaults;
  `.claude/rules/METHODOLOGIES.md` ("After a PR merges") is the authority
  for the routine, and its step 6 is project-specific: **reinstall the MCP
  server** (`cargo install --path crates/grimvault-mcp --locked`) whenever
  the landed change touched anything the binary is built from — the copy
  Claude Code runs lives at `~/.cargo/bin/grimvault-mcp` and does not follow
  `main` by itself (user rule, 2026-09-10). Until the tracker is rebound to
  `github` and the user opts into PRs, the PR steps run as their local
  equivalents: feature branches fast-forwarded into `main`, the docs refresh
  on a `docs/state-post-*` branch fast-forwarded the same way, then
  `git push`.
- No `agent_sync` block — the project has no rules-sync script.
