# Project State

The rehydration document. Any agent starting a session reads this
first to learn where the project stands right now. It answers "where
are we" — never "how does this work" (that's ARCHITECTURE.md and the
code) and never "how should we work" (that's METHODOLOGIES.md).

Last updated: 2026-09-03 (M2 vault loop landed on
`feat/gd-read-stack`; GUI shell next)

## Active workstream

Fast track to a usable Grim Dawn tool (user decision 2026-09-03; the
separate-repo extraction of the shared engine is deferred,
ARCHITECTURE.md "Crate layering"). On `feat/gd-read-stack` the
**read stack (M1) and the first usable loop (M2) are done**: a
five-crate workspace — `univault-engine` (tq-univault's parsers
vendored, GD LZ4 dialect, `platform::config_dir`), `univault-io`
(safe-io with post-write re-read), `univault-ui` (art-free egui kit),
`grimvault-core` (rolling-XOR codec, `player.gdc` / `*.gst` parsers,
layered game-data facade, `vault-store.json` store, computed buckets,
transfer-stash moves, the `Loaded` lossless gate, save/game dir
candidates), and a `grimvault-gui` scaffold — 190 tests, clippy
pedantic clean. `examples/vault_cli` vaults an item out of a copy of
the user's `transfer.gst` into the store and places it back, ending
byte-identical to the original with `grimvault-bak` backups. Binding
facts learned: **opaque save blocks cannot be re-keyed**, so
`player.gdc` stays read-only until blocks 5–17 are typed (yagde, MIT,
has read+write layouts) while `transfer.gst` is writable; and
`formulas.gst` is plaintext TQ-style. Next: M3, the egui shell, so
the loop is usable without a terminal. grim-vault is the Grim Dawn
sibling of tq-univault (`~/Projects/tq-univault`); PROJECT.md is
bound with `tracker: none`.

## Branches in flight

| Branch | Purpose | Status |
|---|---|---|
| `main` | trunk | at `810f7a8` (rules layer), no code |
| `design/shared-engine-split` | engine-split proposal + GD format references | docs only, one commit ahead of `main`; `feat/gd-read-stack` branched from it |
| `feat/gd-read-stack` | workspace, read stack (M1), vault loop (M2), GUI scaffold | local, not pushed (no remote yet); 190 tests green |

## Next up

1. **M3 — egui shell (`crates/grimvault-gui`, scaffolded):** first-run
   setup (game dir + save dir pickers seeded from
   `grimvault_core::platform` candidates; settings in the app config
   dir), background game-data load with progress, transfer-stash tabs
   as editable 10×19 grids with icons (`tex::decode`, fallback tile),
   the store by Group/Bucket, characters read-only (inventory /
   equipped / personal stash); drag-and-drop and double-click between
   stash and store via `transfer::{vault_from_stash, place_in_stash,
   place_in_stash_at}`; autosave (600 ms quiet, backup-first once per
   load via `univault-io`) and an external-change guard (poll
   size+mtime every 2 s; clean pane reloads, dirty pane suspends
   autosave and prompts). Theme through `univault-ui` with
   grim-vault's own palette, no custom fonts yet.
2. **Type `player.gdc` blocks 5–17** by porting yagde (MIT,
   `wr8fdy/yagde`, read+write for every block) so character
   inventories become writable under the lossless gate; then
   `ItemOrigin::Character` gets a producer.
3. **Game-data cache** under the config dir (names, rarity, class,
   footprint, icon RGBA per record actually referenced), stamp-keyed
   to the archives — the three `Items.arc` files are ~740 MB and the
   user's install is on a network mount.
4. Deferred: extract the `univault-*` crates to their own repo and
   re-point tq-univault (R1–R5 done on the vendored copies).

## Most recent meaningful progress

- **2026-09-03 — M2 first usable loop (branch `feat/gd-read-stack`,
  not merged).** `store.rs` (`grimvault-store` v1, monotonic ids
  never reused, unknown fields preserved, foreign documents refused
  as `WrongFormat`), `bucket.rs` (25 buckets in 5 groups from the
  real class census), `transfer.rs` (occupancy, first fit,
  `can_place_at`, vault/place with the store untouched on every
  error), `loaded.rs` (`Loaded<T>` refuses any model that cannot
  reproduce its file), `platform.rs` (save/game dir candidates),
  `Item` serde. `vault_cli` on a copy: vault → place ends
  byte-identical to the original `transfer.gst`; backups rotate. 190
  tests. Why: this is the product's core feature (items out of the
  shared stash into external storage and back), proven before any UI
  exists. Risk: the game has not yet loaded a rewritten
  `transfer.gst` — byte-identity after a round trip is strong but not
  the same as an in-game read; first real use must be with the game
  closed and a backup in hand (the app takes one automatically).

- **2026-09-03 — M1 read stack (branch `feat/gd-read-stack`, not
  merged).** Workspace scaffolded; four crates built in parallel:
  engine (82 tests; ARZ/ARC one parser each behind `Codec` +
  `ArzDialect`; found ARZ records are never stored raw and GD
  `TEX\x02` has no pad byte), io (15; backup-suffix policy +
  post-write re-read), ui (21; art-free, one slicer), core (37;
  cipher order XOR-then-update verified on real saves, lossless
  blocks, `gamedata` layered facade with relic/transmuter bitmap
  fallbacks). `smoke` example: 3 characters + transfer stash, every
  item named with rarity and footprint, every round-trip
  byte-identical. Why: the user chose a usable GD tool over migrating
  tq-univault; this is the foundation every later milestone reads
  through. Risk: opaque blocks cannot be re-keyed (ARCHITECTURE "Data
  flow") — a write to `player.gdc` before blocks 5–17 are typed would
  corrupt the file silently in-game; `encode` refuses, and that
  refusal must survive the GUI work.

- **2026-09-03 — Shared-engine split decided (design dialog; docs only,
  branch `design/shared-engine-split`).** Surveyed tq-univault core
  and gui module by module and researched the GD formats; wrote
  `docs/engine-extraction.md` (dispositions, seams, refactors R1–R12,
  three phases) and `docs/format-references.md` (GD edition).
  Decisions: repo `univault-engine` with crates engine / io / ui;
  phase 1 = pure modules; plain copy with a provenance commit; the ui
  kit ships no art. Why: the first real code (R1–R5 in tq-univault)
  now has an agreed boundary, and ARCHITECTURE's provenance TBDs are
  resolved (gdlc MIT end to end; iagd MIT, no save decoder; GD Stash
  closed). Risk: dispositions come from reading, not compiling — the
  `RecordId` / `GridPos` move (R1) and the codec seam (R2) may
  surface couplings the survey missed; verify with tq-univault's
  tests, not by re-surveying.

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
