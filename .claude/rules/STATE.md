# Project State

The rehydration document. Any agent starting a session reads this
first to learn where the project stands right now. It answers "where
are we" — never "how does this work" (that's ARCHITECTURE.md and the
code) and never "how should we work" (that's METHODOLOGIES.md).

Last updated: 2026-09-03 (rules layer bootstrapped; no code yet)

## Active workstream

Greenfield. grim-vault is the Grim Dawn sibling of tq-univault
(`~/Projects/tq-univault`, GitHub `scottweaver/TQ-AE-Univault`): a
platform-independent item-vault manager in Rust with an egui/eframe
front-end. On 2026-09-03 the repo was initialised on `main` and the
rules layer installed; PROJECT.md is bound with `tracker: none`
(personal project — the Kunai Linear workspace is deliberately not
used here). Binding decisions from the bootstrap dialog (contract in
ARCHITECTURE.md): shared code moves into a **separate engine repo**
consumed as a git dependency (working name `univault-engine`;
extracting it from tq-univault is the first task); tq-univault's
data-flow contract carries over wholesale (unified JSON store with
computed buckets, autosave, backup-first, external-change guard) with
one departure — GD saves are rolling-XOR obfuscated, so writes are a
full re-encode gated by a byte-identical round-trip of the unmodified
baseline; the read-only MCP server, mod forge, and the `gdx3` overlay
are sanctioned surfaces but **not v1**. No Cargo workspace exists yet.

## Branches in flight

| Branch | Purpose | Status |
|---|---|---|
| `main` | trunk | freshly initialised, no commits yet |

## Next up

1. Extract the shared engine repo from tq-univault (typed LE reader,
   ARC/ARZ container parsing, store envelope + id scheme, platform
   discovery, safe-io; egui chrome as a second crate) and scaffold
   this workspace — `crates/grimvault-core` + `crates/grimvault-gui`
   — against it via a git dependency with a local `[patch]` override.
2. Read-only GD format smoke against the real install on
   `/Volumes/scott-games`: decode `player.gdc` + `transfer.gst`
   (rolling XOR) and `database.arz` + `GDX1`/`GDX2` overlays (LZ4),
   behind the byte-identical round-trip test gate. Write
   `docs/format-references.md` (GD edition, with license checks on GD
   Stash / GD Item Assistant) before the first parser PR merges.
3. Store + egui shell: `vault-store.json` (`grimvault-store`),
   character and stash panes, reusing tq-univault chrome through the
   shared UI crate.

## Most recent meaningful progress

- **2026-09-03 — Rules layer bootstrapped.** `git init` on `main`;
  installed RUST_BEST_PRACTICES, METHODOLOGIES, STATE, ARCHITECTURE,
  the CLAUDE.md authority map + AGENTS.md symlink, and the Cursor
  mirrors; PROJECT.md was bound earlier the same day (`tracker:
  none`). Why: identical workflow to tq-univault so the portable
  skills and every session rehydrate the same way across the two
  sibling projects. Risk: every ARCHITECTURE constraint is a pre-code
  inference from tq-univault plus the game's on-disk layout; the
  full-re-encode write path is unverified until the first parser
  round-trips a real save.

## Blocked / waiting

- *(nothing)* — environment note: the game install and saves live on
  a network mount (`/Volumes/scott-games/…`) that may not be present;
  check before assuming. The saves carry Steam-cloud (`remote/save/`)
  and KDE (`.directory`) markers, so the game most likely runs on the
  same separate Linux PC as tq-univault's — confirm — and the SMB
  staleness lessons from tq-univault (`safe_io`: uncached,
  length-checked reads) apply from day one.

## Maintenance

- **Refresh trigger:** any merge or milestone that changes what an
  incoming agent needs to know: workstream shifts, a branch opens or
  closes, "Next up" changes, something lands. Wired into
  METHODOLOGIES.md's post-merge routine (the "refresh STATE.md"
  step).
- **Always update:** "Last updated"; "Branches in flight"; prepend a
  progress entry (what / why / risk voice — a judgment edit, not a
  paste of the PR description).
- **As applicable:** "Active workstream" paragraph, "Next up",
  "Blocked / waiting".
- **Trim policy:** progress log holds at most 10 entries — drop the
  oldest when adding. Anything stable graduates out of this file
  into the appropriate rules doc; this file stays small because
  every agent loads it every turn.
- **Edit policy:** STATE.md is authored on feature branches,
  propagates through merges, and is refreshed (not deleted) on new
  branches. Never edit it directly on `main`; docs-only diffs under
  `.claude/rules/` ride the METHODOLOGIES.md docs-only carve-out.
- **Keep entries short:** each progress entry is a pointer — date,
  PR #, ticket, a sentence or two of judgment. If you're tempted to
  write more, the detail belongs in the PR, commit, or ticket.
