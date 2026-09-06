# Project State

The rehydration document. Any agent starting a session reads this
first to learn where the project stands right now. It answers "where
are we" — never "how does this work" (that's ARCHITECTURE.md and the
code) and never "how should we work" (that's METHODOLOGIES.md).

Last updated: 2026-09-06 (six feature tracks landed on `main` at
`f1f10f6` — `.gds` import, respecs, blueprints + illusions, item
facets with the game's tile symbols, the item stat engine, and the
badge-size fix; the tq-univault search view is in flight on
`feat/search-view`; the in-game acceptance run is still pending)

## Session handoff
<!-- transient; owned by the checkpoint skill -->
**Resume here:** `main` at `f1f10f6` holds everything landed; there is
no remote (the user declined creating the GitHub repo for now — do not
push unprompted, next-up item 8). One track is in flight:
`feat/search-view` (FEATURES.md item 10, the tq-univault search view)
in an agent worktree — when it reports, rebase it onto `main`, run
the gates, fast-forward. Six merged branches (`feat/gds-import`,
`feat/respec`, `feat/blueprints-illusions`, `feat/item-facets`,
`feat/item-stats`, `fix/tile-badges`) and their worktrees under
`.claude/worktrees/agent-*` are still present — delete only with the
user's say-so. Parallel tracks were integrated by rebasing each onto
the moving `main` and fast-forwarding; every conflict so far sat in
`lib.rs` module lists and crate docs, `app.rs` frame hooks,
`panes/mod.rs` `DragFrame` fields, `vault_cli.rs` commands, and the
two docs files — expect the same shape. The **user's acceptance run**
(next-up item 1) is still open: nothing the app writes has been read
by the game yet. The user runs `target/release/grimvault-gui` and
rebuilds with `cargo run --release -p grimvault-gui`; their report
that the tile badges were "not showing" (2026-09-06) was a size bug,
not wiring — fixed in `f1f10f6` and verified on a captured window.

- **GD Stash is an eyes-only reference (user, 2026-09-06):** the
  user's copy at `/Volumes/scott-games/GDStash_v190a` decompiles
  cleanly — `cfr-decompiler GDStash.jar --outputdir <scratchpad>`
  (`brew install cfr-decompiler`, already done on this machine) —
  and the jar embeds the author's plain-text format notes. Read it
  for format facts, verify them against real files, never transcribe
  code or its data tables, never let decompiled output into the repo
  (ARCHITECTURE "Parser provenance"). Everything it has told us so
  far is in `docs/format-references.md` "GD Stash (eyes-only)
  findings": the `.gds` export layout (verified on the user's own
  export), the illusion slot ids (corroborated), names for the
  `player.gdc` fields the typing pass left unnamed, the equipment
  slot order, and where the level / respec numbers come from
  (`records/creatures/pc/playerlevels.dbr`).
- **The user's feature queue is `FEATURES.md`** in the repo root
  (untracked, user-authored, growing): items 1–9 are landed on `main`
  (blueprints, illusions, `.gds` import, gold, both respecs, and the
  three item facets with search constraints); item 10 (tq-univault's
  search view) is in flight; the user was typing items 11+ in their
  editor at last sight — they announce additions with "new items in
  FEATURES.md", and nothing is read until then. The user asked for
  items to be executed on the fly, in parallel where independent, and
  agreed that FEATURES.md gets status tags once the work lands (one
  pass, when they are not editing it).
- **Verifying the GUI without touching the user's setup:** seed a
  fake `HOME` with `Library/Application Support/grim-vault/
  settings.json` (`gameDir` = the real install, `saveDir` = a scratch
  copy of the save tree), optionally a `vault-store.json` beside it
  (`vault_cli … import-gds` fills one from the user's export), launch
  `HOME=<fake> target/release/grimvault-gui`, wait ~40 s for the load,
  and capture the window by id (`screencapture -x -o -l<id>`, the id
  from `CGWindowListCopyWindowInfo` filtered by pid — a ten-line Swift
  script did it this session). No synthetic input; look at what
  renders. The user's own instance is usually running — never kill a
  `grimvault-gui` you did not start.
- **Gap found 2026-09-06:** the shared files come in an
  expansion-level family — `.gst` / `.gsh` (softcore / hardcore),
  `.dst` / `.dsh` (Forgotten Gods level), `.cst`, `.bst` … — and the
  user's save root holds `transfer.dst`, `formulas.dst`,
  `transmutes.dst` beside the `.gst` set. The app opens only `.gst`;
  whether the `.dst` set is live for any character is unknown.
- **Save location (user, 2026-09-06):** Steam Cloud is *disabled* for
  Grim Dawn because cloud sync fights local save editing; with it
  disabled the saves live in the local layout, here
  `/Volumes/scott-games/Grim Dawn Saves/save`. The
  `userdata/<id>/219990/remote/save` path in older notes is the
  cloud-enabled location and is stale for this machine.
- **The game may be running:** the live `user/_Zark/player.gdc`
  changed under a copy during the session. Never read live files —
  `cp` to the scratchpad first. Scratch copies are gone with the
  session.
- **Mod layout, confirmed on disk 2026-09-06:** `main/_<Name>/` and
  `user/_<Name>/` hold `player.gdc` of the same format (both user/
  characters parse fully typed and round-trip); the file never names
  a mod, and the game lists every `user/` character under every mod.
  Each mod keeps its own `save/<Mod>/{transfer,reagents,formulas,
  transmutes}.gst` with the mod name inside (LootAscension: 10-tab
  stash, 82 reagent entries) and its own `mods/<Mod>/database/
  <Mod>.arz`. The database is a fill layer and the stash folder is a
  selectable campaign (both user decisions 2026-09-06, ARCHITECTURE
  "Source of truth" / "ARZ/ARC archives"). `formulas.gst` (plaintext
  blueprints) and `transmutes.gst` (illusions, parsed) exist per
  campaign too and are still not shown anywhere.
- **Why the mod Zark's sacks refused placements, resolved:** the
  "unknown records" (`quest_areah_woodchip.dbr` etc.) were **Fangs of
  Asterkarn records** — `gdx3` is installed and the loaders had
  stopped at `gdx2`; the same ids also exist in LootAscension's
  database. With `gdx3` and the mod fill layers read, `--check` shows
  no unknown record and vault → place back into the mod Zark's own
  sack ends byte-identical. The install has two mods: `survivalmode`
  (the Crucible, with its own `Text_EN.arc` and `Items.arc`) and
  `LootAscension` (database only).
- **Known nits, unfiled:** single-mastery characters show the raw
  class tag; the store pane is a tile flow, not a grid; the illusion
  collection is parsed but not shown; equipped items are display-only;
  a copy keeps the original's seed.
- **Unverified in the window:** drag-and-drop, autosave, the copy
  modifier, the iron-bits field, the Reload/Keep-mine modal, the
  realm-labelled picker, and the campaign switch (flush → reopen →
  rewatch; its pure parts — the newest-stash default, the folder
  listing, the `Campaign` type — are unit-tested) are covered by unit
  tests only (synthetic input is banned); whether the game keeps a
  zero-count reagent entry and whether it accepts any file this app
  wrote are unknown. The mod fill rule is proven by a fixture test,
  not yet by a mod-only item on a real character (every record on
  the mod Zark resolves from the shipped layers once `gdx3` is read).
- **Environment left behind:** no app process is running;
  `target/release/grimvault-gui` and `examples/vault_cli` are built
  from the same code `main` now carries. Settings are seeded in
  `~/Library/Application Support/grim-vault/settings.json` with the
  save dir `/Volumes/scott-games/Grim Dawn Saves/save`; no real
  `vault-store.json` exists yet.
- **Skill note:** `/checkpoint` and `/wrap-up` assume a remote and a
  PR; both run local equivalents here (a docs commit on a
  `docs/state-post-*` branch fast-forwarded into `main`, feature
  branches fast-forwarded instead of merged).

## Active workstream

Fast track to a usable Grim Dawn tool (user decision 2026-09-03; the
separate-repo extraction of the shared engine is deferred,
ARCHITECTURE.md "Crate layering"). On `main` (landed by local
fast-forwards on 2026-09-03 and 2026-09-06; no remote yet) **M1–M5
are done and the app runs**: a five-crate workspace —
`univault-engine` (tq-univault's parsers vendored, GD LZ4 dialect),
`univault-io` (safe-io with post-write re-read), `univault-ui`
(art-free egui kit), `grimvault-core` (rolling-XOR codec, **every
`player.gdc` block typed** so edits re-key correctly, `*.gst`, layered
game-data facade, `vault-store.json`, buckets, stash *and* sack
transfer ops, the `Loaded` lossless gate, and — after the user's
first live run — the account-wide component / crafting-material
storage `reagents.gst` typed and writable), and `grimvault-gui` (the
egui shell: setup with dir candidates, background load with progress,
transfer-stash grids with icons, store by Group/Bucket, characters
with editable sacks and own-stash tabs when every block is typed
(read-only badge otherwise), listed `main/` then `user/` with a
`Realm` carried by every vaulted item's origin, a campaign selector
opening one campaign's stash and component storage at a time (main
or any `save/<Mod>/`, the newest one by default, the campaign in
every origin), installed mods' databases as fill layers under the
shipped ones (`gdx3` included), one generic move — lift into a scratch
store, place out of it, restore the source on refusal — for every
pairing of transfer stash / sacks / own stash / store / component
storage, copy by holding Alt or ⌘/Ctrl on the drop, an iron-bits
field, autosave 600 ms quiet with backup-first once per load across
every document, external-change guard with a Reload/Keep-mine modal
that now watches every `player.gdc`, `--check` headless mode), and — landed the afternoon of 2026-09-06 —
`.gds` import into the store, attribute and mastery respecs,
blueprints and illusions per campaign (adds only), item facets with
the game's own tile symbols and a first store search, the item stat
engine behind the tooltip, and the badge-size fix — 417 tests, clippy
pedantic clean. Verified on scratch copies of the
user's install and saves. **Not yet verified: the game reading any
file this app wrote** — the next step is the user's acceptance run
on the real install with the game closed; the user chose to land on
`main` before it so parallel tracks share a base. The user's
`FEATURES.md` (2026-09-06) queues blueprints, illusions, `.gds`
import, and the two respecs as those tracks; GD Stash is an eyes-only
reference for all of them. grim-vault is the Grim
Dawn sibling of tq-univault; PROJECT.md is bound with `tracker:
none`.

## Branches in flight

| Branch | Purpose | Status |
|---|---|---|
| `main` | trunk | at `f1f10f6` — everything through FEATURES.md item 9 plus the item stat engine; 417 tests green; no remote and no GitHub repo yet |
| `feat/search-view` | FEATURES.md 10: tq-univault's query model and search view | in flight in an agent worktree; rebase onto `main` when it reports |
| `feat/gds-import`, `feat/respec`, `feat/blueprints-illusions`, `feat/item-facets`, `feat/item-stats`, `fix/tile-badges`, `docs/gdstash-reference`, `docs/state-post-tracks` | landed tracks | fast-forwarded into `main`; safe to delete with the user's say-so, worktrees under `.claude/worktrees/agent-*` too |

## Next up

1. **User acceptance on the real install** (game closed): launch
   `cargo run --release -p grimvault-gui`, vault an item out of the
   transfer stash, a main-campaign sack, and the custom-game Zark,
   place them back, Alt-drop a copy, set the iron bits; switch the
   campaign selector between LootAscension and main and vault from the
   mod stash and component storage; import a `.gds` into the store;
   add a blueprint and an illusion; reset a spare character's
   attributes and masteries; then confirm every one of those in-game.
   Nothing this app writes has been read by the game yet — this run is
   what makes `main` trusted.
2. **Search view** (FEATURES.md 10): `feat/search-view` in flight —
   integrate as above; verify on a captured window with the scratch
   store (3,205 items).
3. **FEATURES.md 11 onward** as the user announces them — one track
   per independent item, worktree + branch from `main`, the eyes-only
   GD Stash rule, no STATE.md edits by agents.
4. **Stash file family:** decide how the `.dst` / `.gsh` twins are
   shown (further campaigns? a mode selector?) — design dialog first.
5. **M4 follow-ups:** equipment slots as drag ends (slot order and
   the `ItemSlots` rule are recorded); level / XP edits need an
   evaluator for `experienceLevelEquation` (its text is recorded in
   `docs/format-references.md`); a fresh seed on copy.
6. **Game-data cache** under the config dir (names, rarity, class,
   footprint, icon RGBA, stat lines per referenced record),
   stamp-keyed to the archives, so launches over the network mount
   stop re-reading ~870 MB.
7. Deferred: extract the `univault-*` crates to their own repo and
   re-point tq-univault (R1–R5 done on the vendored copies).
8. Create the GitHub repo (`scottweaver/grim-vault`, PROJECT.md's
   commented `github:` block is pre-filled) and push when ready; the
   wrap-up routine's PR steps stay inert until then.
9. **Mod support leftovers:** refine the default campaign from
   `playmenu.cpn` (GD Stash does not read it either) or remember the
   last selection in settings if the newest-stash rule picks wrong.

## Most recent meaningful progress

- **2026-09-06 — Six tracks landed on `main` (fast-forwards to
  `f1f10f6`).** Built in parallel by agents in worktrees from the
  user's FEATURES.md, rebased onto the moving `main` one by one, gates
  run after each rebase: `.gds` import (read-only boundary, GD Stash's
  duplicate rule, both user exports import with 0 unknown records);
  attribute and mastery respecs under rules read from
  `playerlevels.dbr` and `malepc01.dbr` (block 2's health / energy
  proved to be derived pools; `masteries_allowed` is the level gate
  and is kept); blueprints and illusions per campaign, adds only,
  `formulas.gst` / `transmutes.gst` now writable with JSON interchange
  between campaigns; item facets (monster infrequent, double rare,
  ascended / upgradeable) from the record database with the game's
  eleven tile symbols read by entry out of `UI.arc`, plus a first
  store search; the item stat engine (53,775 lines over 4,114 real
  items, 0 unknown attributes) in the tooltip; and the badge-size fix
  after the user saw no badges — they were 10 px. 417 tests. Why: the
  user asked for the queue to be executed in parallel. Risk: every
  write path is still unverified in-game (acceptance run pending);
  the respec, facet, and stat rules rest on record evidence, not on
  in-game comparison; and the six worktrees still hold build caches.

- **2026-09-06 — GD Stash sanctioned as an eyes-only reference
  (branch `docs/gdstash-reference`, docs only).** The user asked to
  use `/Volumes/scott-games/GDStash_v190a` to accelerate the work;
  the jar decompiles cleanly with CFR and ships its author's format
  notes. ARCHITECTURE "Parser provenance" records the rule (facts
  only, verified against real files, nothing transcribed, decompiled
  output never in the repo) and `docs/format-references.md` the
  findings: the `.gds` layout verified on the user's export, the
  illusion slot ids corroborated, names for every `player.gdc` field
  the typing pass left unnamed, the equipment slot order, the
  `playerlevels.dbr` sources for level and respec math, and the
  `.dst` / `.gsh` stash family the app does not open. Why: the
  user's FEATURES.md queue is exactly what GD Stash implements.
  Risk: a license-unknown reference — the eyes-only rule is what
  keeps the codebase clean, and every fact still needs its real-file
  check before code depends on it.

- **2026-09-06 — Landed on `main` (local fast-forward to `a224d9c`).**
  `feat/character-editing` (M4) and `feat/mod-characters` (M5 mod
  characters, mod + gdx3 game-data layers, campaign selector) merged
  linearly and were deleted; no PR, no remote, `tracker: none`. Why:
  the user chose to land before the in-game acceptance run so the
  parallel tracks in Next up share one base. Risk: `main` now writes
  `player.gdc` in both realms and `transfer.gst` / `reagents.gst` in
  any campaign folder, none of which the game has read yet — the
  acceptance run (next-up item 1) is the gate before trusting it on
  real saves beyond the automatic backups.

- **2026-09-06 — Campaign selector (branch `feat/mod-characters`, not
  merged).** Core: `campaign::{Campaign, ModName}` — `Main` or
  `Mod(name)`, folder `save/` or `save/<Mod>/`, wire name `""` or the
  name, serialized `"main"` / the name — carried by
  `ItemOrigin::{TransferStash, ReagentStorage}` (default main for
  older origins) and taken by `vault_from_stash` /
  `vault_from_reagents`. GUI: `SaveDir::campaigns()` lists the main
  campaign plus every folder with a `transfer.gst`; the loader opens
  the campaign whose stash was written last and cross-checks each
  file's `mod_name` against its folder (a mismatch is a toast, not a
  refusal); a ComboBox beside the stash heading switches — unsaved
  edits flushed first, the guard re-pointed, backup re-armed. CLI:
  `--mod NAME`. 305 tests. Why: the user plays LootAscension, whose
  stash and component storage the app never showed; the character
  file cannot name its mod, so a selector with a newest-file default
  is the honest design. Risk: the switch path itself is shell glue
  without a test; the newest-stash rule follows the game's last
  write, which is right after play but wrong if the user edits the
  other campaign's files by hand in between.

- **2026-09-06 — Mod databases and `gdx3` as game-data layers
  (branch `feat/mod-characters`, not merged).** Core:
  `gamedata::shipped_layers()` names base / `gdx1` / `gdx2` / `gdx3`,
  `mod_layers()` turns a shell's listing of `mods/<Mod>/` into fill
  layers (case-insensitive names, first `.arz` by name, folder
  order), and `GameData::layered(shipped, mods)` composes them with
  mods *below* everything shipped; both loaders read through it, the
  `--check` transcript and the status bar count mod layers. Why: the
  user authorised reading the mod databases, and the "unknown
  records" that made the mod Zark's sacks refuse placements turned
  out to be `gdx3` records the old three-entry constant lists never
  read. Risk: a mod that *only* overrides shipped records changes
  nothing here by design — if a mod character's items should show the
  mod's names, that rule needs renegotiating; load time grows with
  each mod's archives (Crucible's `Items.arc` is 4.6 MB).

- **2026-09-06 — M5 custom-game (mod) characters (branch
  `feat/mod-characters`, stacked on M4, not merged).** Core:
  `gdc::Realm { Main, Custom }` names the `main/` / `user/` folder a
  `player.gdc` came from — the file never says — and is a field of
  `ItemOrigin::{Character, CharacterStash}` (serde default `main` for
  origins written before it), so a vaulted item records *which* Zark;
  `vault_from_sack` / `vault_from_player_stash` take it. GUI: the
  shell lists `main/` then `user/`, `CharacterDoc` / `CharacterEntry`
  carry the realm, drag containers hold `OpenCharacter { realm,
  file }`, the picker suffixes "· custom game", `--check` prints the
  realm. CLI: `<character>` accepts `user/Name`; the smoke examples
  and the real-save gate scan both folders. 297 tests. Why: the
  user's played character is the level-91 LootAscension Zark under
  `user/`, invisible until now. Risk: a sack holding a mod-only record
  refuses every placement (unknown footprint) until the mod database
  is layered in — recorded as TBD in ARCHITECTURE; and, as with M4,
  the game has read nothing this app wrote.

- **2026-09-03 — M4 characters editable (branch
  `feat/character-editing`, not merged).** Core: `ItemOrigin` now
  records the sack (`Character { name, sack }`) or own-stash tab
  (`CharacterStash { name, tab }`); `transfer` gained
  `vault_from_player_stash` / `place_in_player_stash(_at)` over the
  same tab helpers as the transfer stash and lost the three direct
  storage↔stash ops; `PlayerFile::character_info_mut` exposes the
  iron bits; `vault_cli` gained `characters`, `vault-sack`,
  `place-sack`, `money`; the fixture gate now also edits the iron
  bits and duplicates an own-stash item, on the fixture and on the
  three real saves. GUI: one `Tracking` (stamp / edits / backup)
  shared by every document; `CharacterDoc` is writable behind
  `Writable::{Yes, OpaqueBlock}`; `drag` is one shape — `Move {
  source, target, mode }` applied as lift-into-scratch-store then
  place, source restored on refusal, `Mode::Copy` seeding the
  scratch with a clone instead of lifting — so every container
  pairing works without pairwise code; `Doc::Character(slot)` joins
  autosave, the write order (now a `Vec`), the watcher, and the
  conflict modal; the character pane's tabs are drop targets and its
  header carries a `DragValue` for the iron bits reported through
  `DragFrame`. 293 tests. Why: the user asked for moving and copying
  items to and from characters and for gold edits; the read-only
  limit was the last thing making characters second-class. Risk: no
  `player.gdc` this app wrote has been loaded by the game; the copy
  keeps the seed; GD's own duplicate detection (if any) is unknown.

- **2026-09-03 — Landed on `main` (local fast-forward to `54d6042`).**
  `feat/gd-read-stack` and `design/shared-engine-split` merged linearly
  and were deleted; no PR, no remote, `tracker: none`. Six feature
  commits since the rules layer: engine split decided, read stack,
  vault loop, GUI shell + typed `player.gdc`, reagent storage, CLI
  settings fallback. Why: the work is usable and the branches were
  pure history at this point; landing it makes `main` the thing to
  build on. Risk: nothing is off this machine yet — a GitHub push is
  next-up item 5.

- **2026-09-03 — Component and crafting-material storage (branch
  `feat/gd-read-stack`, not merged).** User feedback from the first
  live run: the app lacked the game's dedicated component and
  crafting-material storage. Decoded `reagents.gst` (block 20 v1:
  zero marker, empty mod name, count, then `record + u32 count`
  entries) and `transmutes.gst` (block 19 v2: the per-slot illusion
  collection, typed read-only). Classification comes from the
  database's own `craftingMaterial` flag (125 records: 107
  `ItemRelic` components, 18 quest-class materials). Core gained
  `ReagentStorage`, `Illusions`, `reagents::ReagentKind`, four
  transfer ops, `ItemOrigin::ReagentStorage`, and `relicBitmap` as
  an icon source (components had no icon before). GUI: Components /
  Crafting materials tabs beside the stash with the full drag
  matrix, a third document under autosave / backup / guard,
  `WriteOrder` promoting the destination. `reagents.gst` is now a
  writable game-owned file in ARCHITECTURE. CLI vault → place ends
  byte-identical. 288 tests. Why: a vault that ignores the storage
  the game itself uses for components is not usable. Risk: the game
  has not yet read a rewritten `reagents.gst`; whether it tolerates
  a removed (zero-count) entry versus expecting it kept is unknown.

- **2026-09-03 — M3 egui shell + full `player.gdc` typing (branch
  `feat/gd-read-stack`, not merged).** `grimvault-gui`: phases
  Setup/Loading/Failed/Ready, pure `autosave` / `watch` / `drag` /
  `grid` state machines with unit tests, every move through
  `transfer`, write order destination-before-source, backup-first
  once per load, stamp re-check before every save, Reload/Keep-mine
  modal, `--check` headless transcript, no `player.gdc` writes. Core:
  blocks 2, 5–8, 10, 12–17 typed from yagde (MIT) with skills v8 and
  stats v12 established from real saves; remove/add/move edits
  re-parse correctly on the fixture and all three real characters;
  sack ops with 12×8 / 8×8 dims from `gameengine.dbr`. 261 tests.
  Why: this is the "usable" bar the user asked for — the loop runs
  without a terminal, and the read-only limit on characters is gone
  at the core level. Risk: drag-and-drop, autosave, and the conflict
  modal are covered by unit tests, not by input into the live window
  (synthetic input is banned); and no rewritten file has been loaded
  by the game yet.

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
