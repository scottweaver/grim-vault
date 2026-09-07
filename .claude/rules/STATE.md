# Project State

The rehydration document. Any agent starting a session reads this
first to learn where the project stands right now. It answers "where
are we" — never "how does this work" (that's ARCHITECTURE.md and the
code) and never "how should we work" (that's METHODOLOGIES.md).

Last updated: 2026-09-07 (FEATURES.md 20–24 landed on `main` at
`f6bd3d0` by three parallel tracks — the scrolling tab strip with
storage tabs first, the purge-duplicates standing order, and the
bulk move / copy / delete buttons under a `bulkDuplicates` rule;
496 tests; the three landed branches and worktrees deleted on the
user's say-so; the in-game acceptance run is still pending)

## Session handoff
**Resume here:** `main` at the `docs/state-post-features-20-24`
fast-forward (feature tip `f6bd3d0`) holds everything landed — every
FEATURES.md item through 24 (there is no item 17) — 496 tests, clippy
pedantic clean, fmt clean, headless `--check` clean from the release
build on a scratch copy of the saves; there is no remote (the user
declined creating the GitHub repo for now — do not push unprompted,
next-up item 7). No track is in flight, and `main` is the only
branch: the three
landed branches, their `worktree-agent-*` refs, and the
`.claude/worktrees/agent-*` checkouts were deleted on the user's
say-so once everything was fast-forwarded.
Parallel tracks are integrated by rebasing each onto the moving
`main` — ask the agent to do it, it knows its own conflicts — and
fast-forwarding; today's conflicts sat in `bulk.rs`, `settings.rs`,
`panes/mod.rs`, the tab-header rows of `panes/stash.rs` and
`panes/character.rs`, `check.rs`, `app.rs`, and the ARCHITECTURE
standing-orders paragraph, all resolved by the last agent to rebase.
**Worktree agents share one `git stash` list** — brief them never to
bare-stash. The **user's acceptance run** (next-up item 1) is still
open: nothing the app writes has been read by the game yet. The user
runs `target/release/grimvault-gui` and rebuilds with `cargo run
--release -p grimvault-gui`; `target/release/grimvault-gui` and
`examples/vault_cli` are built from `f6bd3d0`. The user's own
instance (pid 14872 at the last checkpoint) was **not running** by
the end of this session — no agent killed it (each quit only the
instance it launched) — so the next launch is the new build, and its
first load runs the component sync (default on) plus the standing
orders in `settings.json` — the real file already nominates
LootAscension stash tabs 2 and 3 for auto-move, now under the
default `bulkDuplicates: skip`.

- **GD Stash is an eyes-only reference (user, 2026-09-06):** the
  user's copy at `/Volumes/scott-games/GDStash_v190a` decompiles
  cleanly — `cfr-decompiler GDStash.jar --outputdir <scratchpad>`
  (`brew install cfr-decompiler`, already done on this machine) —
  and the jar embeds the author's plain-text format notes. Read it
  for format facts, verify them against real files, never transcribe
  code or its data tables, never let decompiled output into the repo
  (ARCHITECTURE "Parser provenance"). Everything it has told us so
  far is in `docs/format-references.md` "GD Stash (eyes-only)
  findings".
- **The user's feature queue is `FEATURES.md`** in the repo root
  (untracked, user-authored, growing): items 1–24 are landed on
  `main` (there is no item 17) and each line carries a
  `[landed <date>: <rule>]` tag (20–24 tagged 2026-09-07 following
  the pass the user agreed to for 1–19). They announce additions
  with "new items in FEATURES.md" / "new features are ready for
  review", and nothing is read until then. Items are executed on
  the fly, in parallel where independent — one fork agent per track
  in its own worktree, briefed with the design decisions up front;
  three tracks at once worked today, six on 2026-09-07's first pass.
- **Verifying the GUI without touching the user's setup:** point
  `GRIMVAULT_CONFIG_DIR` at a scratch directory holding a
  `settings.json` (`gameDir` = the real install, `saveDir` = a scratch
  copy of the save tree made with `cp -Rp`, so the newest-file rules
  see real timestamps), optionally a `vault-store.json` beside it
  (`vault_cli … import-gds` fills one from the user's export), launch
  `target/release/grimvault-gui` by absolute path (a relative path
  from a worktree runs the main checkout's binary), wait ~15 s for
  the load, and capture the window by id (`screencapture -x -o
  -l<id>`, the id from `CGWindowListCopyWindowInfo` filtered by pid —
  in JXA wrap it in `ObjC.castRefToObject`; winit lists five helper
  windows, the real one is the ~1380×952 entry). The harness refuses
  a fake `HOME` for worktree agents; the config dir variable is
  enough. No synthetic input; look at what renders — modals and
  hover-scrolling are covered by unit tests only. The user's own
  instance is usually running — never kill a `grimvault-gui` you did
  not start.
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
  changed under a copy during a session. Never read live files —
  `cp` to the scratchpad first. Scratch copies are gone with the
  session.
- **Mod layout, confirmed on disk 2026-09-06:** `main/_<Name>/` and
  `user/_<Name>/` hold `player.gdc` of the same format; the file
  never names a mod, and the game lists every `user/` character under
  every mod. Each mod keeps its own `save/<Mod>/{transfer,reagents,
  formulas,transmutes}.gst` with the mod name inside (LootAscension:
  10-tab stash, 82 reagent entries) and its own `mods/<Mod>/database/
  <Mod>.arz`. The database is a fill layer and the stash folder is a
  selectable campaign (ARCHITECTURE "Source of truth" / "ARZ/ARC
  archives").
- **Known nits, unfiled:** `AutoMoveTab` / `AutoMoveTarget` now name
  the tab identity for both standing orders and want an
  order-neutral name (deferred today to keep three tracks from
  colliding); `vault_cli vault-tab` / `vault-stash-tab` pass
  `BulkDuplicates::Skip` rather than reading the setting; a
  hand-edited settings file listing one tab twice runs its order
  twice (the UI dedups); the scroll strip is not in the `dev`
  preview harness; single-mastery characters show the raw class tag;
  equipped items are display-only; a copy keeps the original's seed;
  double-click still targets the stash tab showing while right-click
  targets the last-touched game grid; the search view has no
  expansion-origin filter; `vault_cli` prints refusals in Debug form;
  `a1670af` carries an early uncompiled copy of `gui/src/automove.rs`
  superseded by the next commit.
- **Unverified in the window:** drag-and-drop, autosave, the copy
  modifier, the iron-bits field, the Reload/Keep-mine modal, the
  realm-labelled picker, the campaign switch, right-click moves, the
  item inspector, the standing-order toggles' effects, the search
  table's gestures, and — new today — the Delete-all confirmation
  modal, a Move all / Copy all click, the store's duplicates
  checkbox, and hover-scrolling or wheel-scrolling the tab strip are
  covered by unit tests, headless CLI / `--check` runs, and (for the
  purge) one live debug-GUI run against scratch files only; the new
  tab order, the right chevron with a clipped tab behind it, the
  header buttons, and the checkbox were seen rendering in captures.
  Whether the game keeps a zero-count reagent entry, accepts a
  socket this app filled, or accepts any file this app wrote is
  unknown.
- **Skill note:** `/checkpoint` and `/wrap-up` assume a remote and a
  PR; both run local equivalents here (a docs commit on a
  `docs/state-post-*` branch fast-forwarded into `main`, feature
  branches fast-forwarded instead of merged).

## Active workstream

Fast track to a usable Grim Dawn tool (user decision 2026-09-03; the
separate-repo extraction of the shared engine is deferred,
ARCHITECTURE.md "Crate layering"). On `main` (landed by local
fast-forwards on 2026-09-03, 2026-09-06 and 2026-09-07; no remote yet)
**M1–M5 are done and the app runs**: a five-crate workspace —
`univault-engine` (tq-univault's parsers vendored, GD LZ4 dialect),
`univault-io` (safe-io with post-write re-read), `univault-ui`
(art-free egui kit, now with the chevron scroll strip), `grimvault-core`
(rolling-XOR codec, every `player.gdc` block typed, `*.gst`, layered
game-data facade, `vault-store.json`, buckets, stash *and* sack
transfer ops, the `Loaded` lossless gate, the component storage
`reagents.gst`, the `bulk` module with the seed-duplicate rule, the
purge, and the tab / sack move, copy and clear ops), and
`grimvault-gui` (the egui shell: setup, background load, transfer-stash
grids with icons, store by Group/Bucket with tq-univault's search
view, characters with editable sacks and own-stash tabs, a campaign
selector, mod databases as fill layers, one generic move for every
container pairing, copy by modifier, right-click moves, an item
inspector with sockets, iron bits and respecs, blueprints and
illusions per campaign, `.gds` import, autosave 600 ms quiet with
backup-first once per load, the external-change guard with a
Reload/Keep-mine modal, `--check` headless mode, standing orders —
auto-move, purge, the additive component sync — and, landed
2026-09-07 evening, a scrolling tab strip with the storage tabs
first, Move all / Copy all on every tab and sack, Delete all… behind
a confirmation, and one `bulkDuplicates` rule in `settings.json`) —
496 tests, clippy pedantic clean. Verified on scratch copies of the
user's install and saves. **Not yet verified: the game reading any
file this app wrote** — the next step is the user's acceptance run on
the real install with the game closed. The user's `FEATURES.md` is
landed through item 24. grim-vault is the Grim Dawn sibling of
tq-univault; PROJECT.md is bound with `tracker: none`.

## Branches in flight

| Branch | Purpose | Status |
|---|---|---|
| `main` | trunk | at the `docs/state-post-features-20-24` fast-forward — every FEATURES.md item through 24; 496 tests green; no remote and no GitHub repo yet |
| *(none)* | — | the three track branches of 2026-09-07 evening (`feat/scrolling-tab-strip`, `feat/purge-duplicates`, `feat/bulk-tab-ops`), their `worktree-agent-*` refs, and their worktrees were deleted on the user's say-so; no worktrees remain |

## Next up

1. **User acceptance on the real install** (game closed): launch
   `cargo run --release -p grimvault-gui`, vault an item out of the
   transfer stash, a main-campaign sack, and the custom-game Zark,
   place them back, Alt-drop a copy, set the iron bits; switch the
   campaign selector between LootAscension and main and vault from the
   mod stash and component storage; import a `.gds` into the store;
   add a blueprint and an illusion; reset a spare character's
   attributes and masteries; search the store (⌘F), right-click an
   item each way, click an item and free / fill a socket in the
   inspector, nominate a spare stash tab as auto-move and another as
   purge, Move all / Copy all a spare tab, Delete all… a spare tab,
   flip the store's "Skip duplicates in bulk moves" box, hover a strip
   chevron while dragging, watch the component sync fill the store,
   relaunch to see the remembered campaign; then confirm every one of
   those in-game. Nothing this app writes has been read by the game
   yet — this run is what makes `main` trusted.
2. **FEATURES.md 25 onward** as the user announces them — one fork
   agent per independent item in its own worktree, branch from
   `main`, decisions fixed in the brief, the eyes-only GD Stash rule,
   no bare `git stash`, no STATE.md edits by agents; the integrator
   asks each agent to rebase onto the moving `main` before landing.
3. **Rename `AutoMoveTab` / `AutoMoveTarget`** to an order-neutral
   name now that both standing orders share them (deferred on
   2026-09-07 to keep three parallel tracks from colliding).
4. **Stash file family:** decide how the `.dst` / `.gsh` twins are
   shown (further campaigns? a mode selector?) — design dialog first.
5. **Follow-ups:** equipment slots as drag ends; level / XP edits need
   an evaluator for `experienceLevelEquation` (its text is in
   `docs/format-references.md`); a fresh seed on copy; drop-to-socket
   through `drag::Move`; unify the double-click and right-click
   targets; `vault_cli` reading `bulkDuplicates`; a duplicate /
   extract gesture on search rows like tq-univault's.
6. **Launch time** is bytes, not sequencing: ~1.06 GB of the 1.26 GB
   read per launch is `Items.arc` textures read whole only to be
   indexed. Read them by entry as `UI.arc` already is, or build the
   game-data cache under the config dir, stamp-keyed to the archives.
7. Deferred: extract the `univault-*` crates to their own repo and
   re-point tq-univault (R1–R5 done on the vendored copies).
8. Create the GitHub repo (`scottweaver/grim-vault`, PROJECT.md's
   commented `github:` block is pre-filled) and push when ready; the
   wrap-up routine's PR steps stay inert until then.
9. **Mod support leftovers:** `playmenu.cpn` remains unread (GD
   Stash does not read it either).

## Most recent meaningful progress

- **2026-09-07 (evening) — FEATURES.md 20–24 landed on `main` (three
  fast-forwards to `f6bd3d0`).** The user announced the five items;
  they were split into three independent tracks and built by fork
  agents in worktrees with the decisions fixed in the brief, each
  rebased onto the moving `main` by its own agent: (C) an art-free
  `univault-ui::components::scroll_strip` — a hidden-bar horizontal
  scroll area with pointer-position chevrons shared with the vendored
  `tabbed_panel` through a new `chevron` module, so a drag scrolls it
  — replacing both wrapping strips, with Components and Crafting
  materials first; (B) `purgeDuplicates` in `settings.json` as a
  second standing order under `StandingOrder { AutoMove,
  PurgeDuplicates }`, `bulk::purge_duplicates` deleting only what the
  store already holds by record and seed, run after every auto-move
  at the same moments and gate, `--check` printing its plan, recorded
  in ARCHITECTURE "Data flow" as the one order that destroys items;
  (A) `bulkDuplicates: skip | allow` (default skip) governing every
  bulk move or copy into the store including auto-move, `TabPlan`
  generalized over any item iterator, `copy_tab` / `copy_player_tab`
  / `vault_sack` / `copy_sack` / `clear_tab`, Move all / Copy all on
  every tab and sack, Delete all… on transfer and own-stash tabs
  behind a modal, the store's "Skip duplicates in bulk moves" box,
  `BulkOp::Transfer(Mode) | Clear` so a clear can never reach the
  copy path. 496 tests. Why: the user asked for the queue to keep
  landing on the fly. Risk: two more writes the app makes that
  destroy items (purge on the app's initiative, delete-all on a
  click) rest on the once-per-load backup and are unverified in-game;
  the purge has no confirmation beyond its checkbox; the duplicates
  rule now changes what auto-move leaves behind.

- **2026-09-07 — Six more tracks landed on `main` (fast-forwards and
  two cherry-picks to `666f592`).** Resumed after a usage-limit
  cut-off: the search-view agent's ~2,900 uncommitted lines were
  finished and landed (typed query model, three-way verdicts, one
  sortable `egui_extras` table, `ui-state.json` for view state, a
  `search_cli` example; parity with tq-univault except the
  expansion-origin filter), then five fresh tracks from FEATURES.md
  11–19 ran as fork agents in worktrees with the design decisions
  fixed in the brief: startup on the remembered campaign
  (`settings.json`) and the newest `player.gdc`; right-click moves
  through the one `drag::Move` path with a `LastActive` grid;
  parallel archive reads with `std::thread::scope`, assembled in
  layer order, transcript byte-identical, NAS load ~12 s; sockets
  (`core::socket`, allow flags from `itemrelic.tpl` /
  `itemenchantment.tpl`, components complete at level 0, the game no
  longer rolls a completion bonus) behind a left-click inspector,
  CLI detach → attach byte-identical on a real save; and standing
  orders — auto-move tabs and the additive component-storage sync —
  under a seed-duplicate rule (`maxStackSize` from `itembase.tpl`),
  recorded in ARCHITECTURE "Data flow" as a write the app makes on
  its own initiative. 483 tests. Why: the user asked for the queue in
  parallel; six agents at once with the integrator rebasing each onto
  the moving `main` worked. Risk: nothing verified in-game still; the
  sync is on by default and fills the store at first launch; a
  shared-stash collision between two worktrees cost a detour and is
  now a briefing rule.

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
  next-up item 8.

## Blocked / waiting

- **Waiting on the user — rules to confirm or overrule** (built under
  stated assumptions, each a small change if reversed): (1) *resolved
  today by design:* a nominated auto-move tab keeps its seed
  duplicates under the default `bulkDuplicates: skip` and moves them
  under `allow`; (2) the component-storage sync is a high-water mark
  that refills the vault after the user moves reagents out of it, and
  is on by default; (3) double-click targets the stash tab showing
  while right-click targets the last-touched game grid; (4) *new:*
  Delete all… covers transfer-stash tabs and a character's own-stash
  tabs, never sacks; (5) *new:* the purge runs with no confirmation
  beyond its checkbox and its hover text; (6) *new:* one global
  duplicates rule governs the manual bulk buttons and auto-move
  alike, and the store's checkbox is where it lives.
- **Waiting on the user — the acceptance run** (next-up item 1) is
  what makes `main` trustworthy on real saves; until it runs, every
  write path rests on byte-identical re-encodes and CLI / headless
  round trips only.
- Environment note: the game install and saves live on
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
