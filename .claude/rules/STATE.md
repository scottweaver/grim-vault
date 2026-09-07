# Project State

The rehydration document. Any agent starting a session reads this
first to learn where the project stands right now. It answers "where
are we" — never "how does this work" (that's ARCHITECTURE.md and the
code) and never "how should we work" (that's METHODOLOGIES.md).

Last updated: 2026-09-07 (FEATURES.md 31 landed on `main` at
`6d51441` — the learned-blueprint sync, a fourth standing order that
turns every blueprint learned in the open campaign the vault has no
item of into a blueprint item in the store; 505 tests; fast-forwarded
and pushed; 30 at `0d4efe2`, 28–29 at `f400222`, 25–27 at `a22ac10`
earlier the same evening)

## Session handoff
<!-- transient; owned by the checkpoint skill -->
**Resume here:** `main` at the `docs/state-post-features-31`
fast-forward (feature tip `6d51441`) holds everything landed — every
FEATURES.md item through 31 (there is no item 17) — 505 tests, clippy
pedantic clean, fmt clean, headless `--check` clean from the release
build on a scratch copy of the saves. **Why 25–31:** the user asked
in chat whether the vault is portable so a Linux binary on Bazzite
can use the Mac's vault; it is (no path or platform fact in it), and
they asked for the shared-location route to be first-class, plus
export / import and a settings modal (25–27), then corrected what
they saw in the window — crafting materials filed under Quest Items,
stackables held as several stacks (28–29), group tabs that did not
say what they held (30) — and asked for learned blueprints to sync
into the vault (31) — all recorded in FEATURES.md by the agent, not
the user, so the queue stays the one record. **The first launch of
`6d51441` on the real files adds 143 blueprint items** to the store
from LootAscension's `formulas.gst` (the main campaign's 217 are all
held already), on top of the 85-stack fold from 28–29 — both
expected, both autosaved backup-first. **Nine
components stay in two stacks after the fold** (Aether Soul,
Whetstone, Bloody Whetstone, Aether Shard …): one stack came out of
the reagent storage bare, the other out of a GD Stash export carrying
a completion-bonus `modifierName` (`lootaffixes/crafting/…`), and the
stack key is the whole item less seed and count, so they are
different items to the fold. Whether to fold across the modifier is
the user's call (Blocked / waiting 7). **The first launch of `f400222` on the real store folds it:**
the user's real store (3,450 entries at 16:05) holds 85 split stacks
that the window will fold into 70 on load and autosave, backup-first
— expected, not a fault; the CLI dry run preserved every record's
unit total. **The user's next step is the Linux build:**
`target/release/grimvault-gui` is built from `f400222`; on Bazzite,
`~/.config/grim-vault/settings.json` with `storeFile` naming the NAS
vault under the Linux mount point is the whole setup, and the ⚙ modal
writes it. The settings modal has only been exercised by unit tests
and the CLI twins — nobody has clicked the gear yet — so the first
thing to confirm in the window is that the modal opens, Apply on a
store change swaps the pane, and Export / Import toast sensibly.
**The remote exists now:** the user created
`github.com/scottweaver/grim-vault` (public, Issues enabled) on
2026-09-07 at 14:54 local and pushed; `origin/main` is in sync at
`0e3f39d`, so next-up item 7 is done — push after every landing from
now on (`git push`, never force). PROJECT.md still binds
`tracker: none` and the skills still run their local equivalents;
switching to `tracker: github` (`/bootstrap-project` edit mode, the
commented block is pre-filled) and to PR-based landing per
METHODOLOGIES is the user's call (Blocked / waiting). No track is in
flight, and `main` is the only branch: the three landed branches,
their `worktree-agent-*` refs, and the `.claude/worktrees/agent-*`
checkouts were deleted on the user's say-so once everything was
fast-forwarded. **The acceptance run is under way, results unknown
here:** the user relaunched the `f6bd3d0` build at 14:49 (pid 26200
at checkpoint) and at 14:50 nominated LootAscension stash tabs 2 and
3 for *purge* as well as auto-move under `bulkDuplicates: skip`; the
real store (2.8 MB) was rewritten at 14:50 with a backup beside it,
so the standing orders of the new build have already run on the real
files. Ask the user what the game did with them before assuming any
write path works or fails.
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
  (untracked, user-authored, growing): items 1–31 are landed on
  `main` (there is no item 17) and each line carries a
  `[landed <date>: <rule>]` tag (20–24 tagged 2026-09-07 following
  the pass the user agreed to for 1–19; 25–31 were *written* by the
  agent from chat requests the same day, marked "asked in chat").
  They announce additions with "new items in FEATURES.md" / "new
  features are ready for review", and nothing is read until then.
  Items are executed on the fly, in parallel where independent — one
  fork agent per track in its own worktree, briefed with the design
  decisions up front; three tracks at once worked today, six on
  2026-09-07's first pass; 25–27, 28–29, 30 and 31 were single
  tracks in the main checkout.
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
  windows, the real one is the ~1380×952 entry). Over the SMB
  mount `cp -Rp` copies everything but exits non-zero on `chflags`,
  so never chain it with `&&`. The harness refuses
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
- **Skill note:** the remote exists since 2026-09-07 evening, but
  until PROJECT.md is rebound to `tracker: github` and the user opts
  into PRs, `/checkpoint` and `/wrap-up` keep running their local
  equivalents (a docs commit on a `docs/state-post-*` branch
  fast-forwarded into `main`, feature branches fast-forwarded instead
  of merged) followed by `git push`.

## Active workstream

Fast track to a usable Grim Dawn tool (user decision 2026-09-03; the
separate-repo extraction of the shared engine is deferred,
ARCHITECTURE.md "Crate layering"). On `main` (landed by local
fast-forwards on 2026-09-03, 2026-09-06 and 2026-09-07; pushed to
`origin/main` since 2026-09-07 evening)
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
a confirmation, and one `bulkDuplicates` rule in `settings.json`, and
— landed 2026-09-07 night — the vault store at any path via
`storeFile`, vault export / import as a store merge, a settings
modal behind ⚙, a Crafting Materials bucket, one consolidated stack
per stackable record kept by the window, the store's seven groups —
Weapons, Armor, Accessories with Relics, Item Upgrades, Crafting,
Consumables, Other — and the learned-blueprint sync as a fourth
standing order) — 505 tests, clippy pedantic clean. Verified on
scratch copies of the user's install and saves. **Not yet verified:
the game reading any file this app wrote** — the next step is the
user's acceptance run on the real install with the game closed, and
now the Linux build on Bazzite sharing the NAS vault. The user's
`FEATURES.md` is landed through item 31. grim-vault is the Grim Dawn
sibling of tq-univault; PROJECT.md is bound with `tracker: none`.

## Branches in flight

| Branch | Purpose | Status |
|---|---|---|
| `main` | trunk | at the `docs/state-post-features-31` fast-forward — every FEATURES.md item through 31; 505 tests green; pushed, `origin/main` in sync |
| `feat/blueprint-sync`, `docs/state-post-features-31` | FEATURES.md 31 | landed by fast-forward at `6d51441`; fully merged, awaiting the user's say-so to delete (no worktrees) |
| `feat/store-groups`, `docs/state-post-features-30` | FEATURES.md 30 | landed by fast-forward at `0d4efe2`; fully merged, awaiting the user's say-so to delete (no worktrees) |
| `feat/store-location-and-settings`, `docs/state-post-features-25-27` | FEATURES.md 25–27 | landed by fast-forward at `a22ac10`; fully merged, awaiting the user's say-so to delete (no worktrees) |
| `feat/material-bucket-and-stacks`, `docs/state-post-features-28-29` | FEATURES.md 28–29 | landed by fast-forward at `f400222`; fully merged, awaiting the user's say-so to delete (no worktrees) |

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
   relaunch to see the remembered campaign; **new:** click ⚙, point
   the store file at a copy on the NAS and Apply, export a copy,
   import it back (expect "nothing new"); see the first load fold the
   real store's split stacks (a toast, "store taken" in backup-first),
   Crafting > Crafting Materials populated and Other > Quest Items
   down to the true quest items, the blueprint sync toast adding
   LootAscension's 143 unheld blueprints as items, and — after
   learning a blueprint in-game — the guard's reload of `formulas.gst`
   adding one more; then confirm every one of those
   in-game — **in particular whether the game accepts a stack placed
   from the vault that exceeds its own max stack size** (a
   consolidated stack of components can be thousands; the app never
   splits on placement, next-up 5). Nothing this app writes has been read by the game
   yet — this run is what makes `main` trusted.
1b. **The Linux build for Bazzite** (user intent, 2026-09-07): build
   `grimvault-gui` on or for Linux (pure-Rust deps, no native ones
   recorded; `rfd` uses GTK/XDG portals on Linux — check it links on
   Bazzite's immutable base), then on that machine write
   `~/.config/grim-vault/settings.json` — or run setup and use ⚙ —
   with `storeFile` naming the NAS vault under the Linux mount point.
   One app on the vault at a time.
2. **FEATURES.md 28 onward** as the user announces them — one fork
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
   extract gesture on search rows like tq-univault's; **splitting a
   consolidated stack on placement** to the record's `maxStackSize`
   (engine defaults in `docs/format-references.md`) or a "take N"
   gesture, now that a vault stack can exceed what one cell holds.
6. **Launch time** is bytes, not sequencing: ~1.06 GB of the 1.26 GB
   read per launch is `Items.arc` textures read whole only to be
   indexed. Read them by entry as `UI.arc` already is, or build the
   game-data cache under the config dir, stamp-keyed to the archives.
7. Deferred: extract the `univault-*` crates to their own repo and
   re-point tq-univault (R1–R5 done on the vendored copies).
8. ~~Create the GitHub repo and push~~ — done by the user on
   2026-09-07 (`scottweaver/grim-vault`, public, Issues enabled).
   Left: decide whether to rebind PROJECT.md to `tracker: github`
   and land future tracks through PRs (wrap-up's PR steps are live
   once that happens).
9. **Mod support leftovers:** `playmenu.cpn` remains unread (GD
   Stash does not read it either).

## Most recent meaningful progress

- **2026-09-07 (latest) — FEATURES.md 31 landed on `main`
  (fast-forward to `6d51441`).** The user asked for blueprints to
  auto-sync to the vault when found in-game. Modelled as the
  component sync's twin over `formulas.gst`: `bulk::blueprint_shortfall`
  lists the learned records the store holds no item of (any origin —
  a blueprint vaulted from a stash is the blueprint; record paths
  compared with `ids::normalize`), `bulk::sync_blueprints` adds one
  blueprint item each under `ItemOrigin::LearnedBlueprint
  { campaign }`; `settings.json` `syncBlueprints` (default on);
  `World::sync_blueprints` runs in `carry_out_orders` for
  `Doc::Blueprints`, after an in-app add or import to the list, and
  when the toggle turns on — from the list's toolbar or the ⚙ modal;
  `--check` prints the plan; `vault_cli sync-blueprints`. 505 tests.
  Why: the user thinks of the vault as the union of everything found;
  the sync pattern was already accepted for reagents. Risk: a learned
  blueprint becomes a *placeable item* in the vault — dropping it into
  another campaign's stash conjures a blueprint drop from knowledge,
  which is the point but is a duplication the game never offers
  (Blocked / waiting 8); the first launch adds 143 items from
  LootAscension's list.

- **2026-09-07 (last) — FEATURES.md 30 landed on `main`
  (fast-forward to `0d4efe2`).** The user read the group tabs after
  28–29 and asked that they say what they hold: `Group` gains
  `Upgrades` ("Item Upgrades": Components, Augments — what goes into
  worn items) and `Consumables` (Potions & Oils — the OneShot
  classes, usable skills, reset tonics — and the new `Bucket::Writ`,
  "Writs & Merits": faction boosters and warrants, difficulty
  unlocks); Relics move to Accessories as equipped items; Crafting
  keeps Materials, Blueprints, Transmuters; Other is Quest Items,
  Notes, and the unmapped fallback. `Bucket::ALL` follows the new
  order; an older `ui-state.json` naming "other" still parses. 504
  tests. Why: the user's reading of the game — relics are worn,
  components and augments upgrade, only the materials tab is
  crafting. Risk: none to data (buckets are computed views); nine
  components remain in two stacks because a GD Stash export's copy
  carries a completion-bonus modifier the storage's copy lacks.

- **2026-09-07 (late) — FEATURES.md 28–29 landed on `main`
  (fast-forward to `f400222`, one track in the main checkout).** The
  user saw crafting materials under Other > Quest Items and the same
  component held as several stacks. Core: `Bucket::of(class,
  reagent)` — a record the game flags `craftingMaterial` (they are
  `QuestItem`s by class; `ItemInfo.reagent` already knew) is the new
  `Bucket::Material` under Crafting, a stack for `Identity::of`;
  `VaultStore::consolidate_stacks` folds a stackable record's later
  entries into its first (key: the item less seed and count; units
  summed, ids retired, first origin and moment kept) and
  `VaultStore::merge` treats stacks as a high-water mark per record
  so a repeated import cannot double a folded stack; `Item::units`
  names the count-at-least-one rule. GUI: `World::settle_store` runs
  the fold at the end of any frame whose store revision changed
  (never mid-drag), marks the store edited, toasts; `--check` prints
  what the window would fold; `vault_cli consolidate-stacks`. 503
  tests. Why: the user's rule — one stack per item — and the game's
  own filing of materials. Risk: a folded stack can exceed the
  game's max stack size and the app never splits on placement
  (next-up 5); the fold drops the folded entries' provenance; the
  real store folds 85 entries on the next launch, backup-first.

- **2026-09-07 (night) — FEATURES.md 25–27 landed on `main`
  (fast-forward to `a22ac10`, one track in the main checkout).** The
  user asked whether the vault is portable enough for a Bazzite Linux
  build to share it; it is, and they asked for the shared-location
  route to be first-class. Core: `settings.json` `storeFile`
  (absolute, or relative to the config dir; absent = beside the
  settings) resolved by `Settings::store_file`, and
  `VaultStore::merge` — every entry of another store whose vaulting
  event (origin, moment, item) this one lacks is added under a fresh
  id, nothing removed, so a store merged into itself adds nothing
  (`ItemOrigin` now derives `Hash`). GUI: `settings_dialog` — a ⚙ at
  the left of the status bar opens a modal with the directories, the
  store file (Browse… / New… / Default and a verdict that refuses a
  directory or an unmounted parent), the sync and bulk-duplicates
  rules, "Export a copy…" and "Import from a copy…"; the draft is
  applied as one and `Change::between` decides its cost — rules in
  place, a store swap (edits flushed first, guard re-pointed, standing
  orders run), or a full reload via the loader. `--check` and the CLI
  examples honour `storeFile`; `vault_cli export-store` (never
  overwrites) / `import-store`. 500 tests. Why: one vault for two
  machines with machine-local settings is exactly the split the
  self-describing store was built for. Risk: the modal has never been
  clicked (unit tests and the CLI twins only); two apps on one NAS
  store at once rely on the polling guard, which catches a write
  between saves but not two edits in flight; a store switch runs the
  standing orders into the newly opened store, so switching to an
  empty file auto-moves nominated tabs into it at once.

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
  alike, and the store's checkbox is where it lives; (7) *new:* the
  stack fold keys on the whole item less seed and count, so a
  component carrying a completion-bonus `modifierName` (from a GD
  Stash export) stays a separate stack from the bare one out of the
  storage — nine records in the real store; folding across the
  modifier is a one-line change to `StackKey::of` if the user says
  those are one item; (8) *new:* the blueprint sync represents a
  learned blueprint as a blueprint *item* in the store (bucket
  Blueprints, placeable back into any campaign's stash) rather than
  as a separate knowledge list — the store stays one file of items
  and the whole store UI applies; if the user wants a list that can
  be pushed into a campaign's `formulas.gst` instead, that is a
  different feature.
- **Waiting on the user — the acceptance run** (next-up item 1) is
  under way in the relaunched build as of this checkpoint; its
  results are what make `main` trustworthy on real saves, and this
  session knows none of them.
- **Waiting on the user — tracker and landing flow:** with the repo
  on GitHub, rebind PROJECT.md to `tracker: github` and switch to
  PR-based landing, or keep the local fast-forward flow plus `git
  push`.
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
