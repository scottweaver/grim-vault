# Project State

The rehydration document. Any agent starting a session reads this
first to learn where the project stands right now. It answers "where
are we" — never "how does this work" (that's ARCHITECTURE.md and the
code) and never "how should we work" (that's METHODOLOGIES.md).

Last updated: 2026-09-08 evening (after the foreign-icon fix landed on
`main` at `183dc22` — nothing left unlanded; all pushed; 534 tests)

## Session handoff
<!-- transient; owned by the checkpoint skill -->
**Resume here:** `main` at `183dc22` plus this refresh (pushed,
`origin/main` in sync) holds everything landed — every FEATURES.md
item through 33 (there is no item 17), the lost-component fix and
the foreign-icon fix below — 534 tests, clippy pedantic clean, fmt
clean, headless `--check` clean from the release build on the real
install with a scratch copy of the saves. **Landed 2026-09-08
evening (`183dc22`, `fix/icons-outside-items-arc`, branch deleted by the wrap-up):** the user hit
"item 21 (records/storyelements/signs/signh.dbr) has no known
footprint" a second time, copying a relic from the vault into
LootAscension transfer tab 0 — item 21 there is Lokarr's Gaze, whose
icon sits in `gdx1`'s `Level Art.arc`, so the tab's occupancy could
not be computed and every placement into it was refused. The fix
was built and verified on 2026-09-07 evening (the first report, an
amulet into the same tab) but never fast-forwarded into `main`, and
the 2026-09-08 release rebuild from `main` dropped it from the
user's binary. Rebased onto `main` (one conflict in
`gui/src/loader.rs`, both sides kept — the icon step runs before the
affix build), 534 tests, clippy and fmt clean, `--check` on the real
install: 26 of 26 icons outside `Items.arc` found, Lokarr's Gaze
listed at 2x2 in tab 0, no problems. `target/release/grimvault-gui`
is built from `183dc22` (18:28); the user has to relaunch it.
**Bug fixed 2026-09-08 (`42291a0`, `fix/linked-conflicts`, reported by
the user in chat):** a Black Tallow applied from the vault to a worn
medal was lost — the game file changed on disk, "Reload from disk"
rolled back the game half, and the vault half (stack 3 → 2) stood.
Cause: a two-document edit had no link between its halves, and
`flush` kept writing past a conflicting document (the store was saved
with the decrement before the modal), while the guard-first path left
the store half to be autosaved once the gate reopened. Fix:
`gui/src/links.rs` `Links`, bound by `World::edited_together` at every
two-document site (moves, socket fill and free, bulk move, auto-move)
and settled when a side is written or reloaded; `flush` stops at the
first conflict; the modal names the bound documents and Reload / Keep
mine apply to all of them (ARCHITECTURE "Data flow", 2026-09-08).
Residual: a partner already written before the conflict leaves a
duplicate, never a loss (next-up 5). Exercised by three `Links` tests
and review only, not live. The user's store reads Black Tallow 1 (was
3 at the 15:21 load): one unit re-applied to the medal, one lost; the
user chose not to restore it. **Landed 2026-09-08, both tracks:** the
affix card (32, built in the main checkout) and the gear tab (33, `feat/equipment-tiles`, built by
a fork agent in its own worktree from the user's decision "gear tiles,
first in the strip, unequip by drag": every slot an item tile with
tooltip and inspector, gear dragged, right-clicked or double-clicked
*off* the character into the vault, a sack or a stash tab through the
one `drag::Move` path — `DragSource::Equipped` is a source only, no
`DropTarget` can name a slot, so equipping stays unrepresentable —
the slot left as the game leaves a never-used one: `attached 0`,
empty base name, `stackCount 1`, everything else blank, probed on all
five real characters; `ItemOrigin::Equipped { realm, name, slot }`;
`vault_from_equipment`; the agent rebased onto the moved `main`
itself and the integrator re-ran the gates in its worktree before the
fast-forward). **Nothing is left unlanded.** The agent worktree
`.claude/worktrees/agent-aaed563a4259e1782` still has
`feat/equipment-tiles` checked out with its build cache; removing it
and the two landed feature branches is the user's call. **Why 32–33:** the user asked in
chat for reference cards (first: a searchable list of the "proper"
affix names with what each grants — design dialog: floating card
windows, one row per name with value ranges, expandable to the
records) and, mid-session, for a tab showing a character's equipped
gear — the Equipped tab existed since M3 as a text list between the
sacks and the stash tabs, easy to scroll out of view, hence the
redesign; both recorded in FEATURES.md by the agent. **Still open
from 2026-09-07:** the user's acceptance run on the real install
(nothing the app writes has been read by the game yet — ask what the
game did before assuming any write path works); the Linux build for
Bazzite sharing the NAS vault via `storeFile`; the first launch of a
post-`35301b0` build records 337 learned blueprints and folds 85 split
stacks, both expected and backup-first; nine components stay in two
stacks (a completion-bonus `modifierName` on the GD Stash export's
copy) pending the user's call (Blocked / waiting 7); PROJECT.md still
binds `tracker: none` and the local fast-forward flow plus `git push`
is what lands work. The user's own `grimvault-gui` was running during
the 2026-09-08 session (its `vault-store.json` was written at 18:20;
none was running when the binary was rebuilt at 18:28) — never kill
an instance you did not start.
Parallel tracks are integrated by rebasing each onto the moving
`main` — ask the agent to do it, it knows its own conflicts — and
fast-forwarding; past conflicts sat in `bulk.rs`, `settings.rs`,
`panes/mod.rs`, the tab-header rows of `panes/stash.rs` and
`panes/character.rs`, `check.rs`, `app.rs`, and the ARCHITECTURE
standing-orders paragraph. **Worktree agents share one `git stash`
list** — brief them never to bare-stash. The user runs
`target/release/grimvault-gui` and rebuilds with `cargo run --release
-p grimvault-gui`; `target/release/grimvault-gui` is built from
`183dc22` (2026-09-08 18:28), the release `grimvault-core` examples
still from `cd4ed5b`. The
real `settings.json` nominates LootAscension stash tabs 1–6 for both
auto-move and purge under `bulkDuplicates: skip`, with the reagent
and blueprint syncs on — every launch on the real files runs all
four standing orders.

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
  (untracked, user-authored, growing): items 1–33 are landed on
  `main` (there is no item 17) and each line carries a
  `[landed <date>: <rule>]` tag (20–24 tagged 2026-09-07 following
  the pass the user agreed to for 1–19; 25–33 were *written* by the
  agent from chat requests, marked "asked in chat").
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
  hover-scrolling are covered by unit tests only. A `ui-state.json`
  seeded beside the scratch `settings.json` opens what a click would
  (`{"reference":{"affixes":{"open":true,"query":{"text":"cleric"}}}}`
  put the affix card up at launch on 2026-09-08). egui's default
  fonts lack the small triangles ▸ ▾ (they draw as boxes); the media
  symbols ⏵ ⏷ render. The user's own
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
  preview harness; single-mastery characters show the raw class tag; worn gear comes
  off but nothing can be equipped from the app (no slot rule yet); a
  copy keeps the original's seed;
  double-click still targets the stash tab showing while right-click
  targets the last-touched game grid; the search view has no
  expansion-origin filter; `vault_cli` prints refusals in Debug form;
  `a1670af` carries an early uncompiled copy of `gui/src/automove.rs`
  superseded by the next commit; the affix card lists an entry's
  partial grants in first-seen order rather than grouped by item form
  (Thunderstruck's armor, shield and weapon lines interleave — the
  unfolded records tell them apart).
- **Unverified in the window:** drag-and-drop, autosave, the copy
  modifier, the iron-bits field, the Reload/Keep-mine modal, the
  realm-labelled picker, the campaign switch, right-click moves, the
  item inspector, the standing-order toggles' effects, the search
  table's gestures, and — new today — the Delete-all confirmation
  modal, a Move all / Copy all click, the store's duplicates
  checkbox, and hover-scrolling or wheel-scrolling the tab strip, and — new
  2026-09-08 — the affix card's row click, position toggles and rarity combo, and
  the gear tiles' drag, right-click and double-click are covered by
  unit tests (the tiles, both weapon sets and the "in hand" caption
  were seen rendering in the agent's captures), headless CLI / `--check` runs, and (for the
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
standing order, recording knowledge in the store's `blueprints` list;
and — landed 2026-09-08 — the affix reference card behind a Reference
menu in the status bar, `grimvault-core::reference` building one
entry per named prefix and suffix with value ranges across its tiers,
and the gear tab as tiles first in the character strip with worn gear
taken off by drag, right-click or double-click; and — landed
2026-09-08 evening — item icons read by entry from the archive their
bitmap path names, so Lokarr's set, the `gdx2` potion formulas and
Iron Bits have footprints and a tab holding one accepts placements)
— 534 tests, clippy pedantic clean. Verified on
scratch copies of the user's install and saves. **Not yet verified:
the game reading any file this app wrote** — the next step is the
user's acceptance run on the real install with the game closed, and
now the Linux build on Bazzite sharing the NAS vault. The user's
`FEATURES.md` is landed through item 33. grim-vault is the Grim Dawn
sibling of tq-univault; PROJECT.md is bound with `tracker: none`.

## Branches in flight

| Branch | Purpose | Status |
|---|---|---|
| `main` | trunk | at `183dc22` (the foreign-icon fix) plus this refresh — every FEATURES.md item through 33, both 2026-09-08 fixes; 534 tests green; pushed, `origin/main` in sync |
| `feat/reference-cards` | FEATURES.md 32, the affix card | landed by fast-forward 2026-09-08; deletable (`git branch -d`) — the user confirms deletions |
| `feat/equipment-tiles` | FEATURES.md 33, the gear tab as tiles with unequip-by-drag | landed by fast-forward 2026-09-08; still checked out in the agent worktree `.claude/worktrees/agent-aaed563a4259e1782` — remove the worktree, then `git branch -d`, on the user's word |
| `fix/linked-conflicts` | the lost-component fix: linked documents share one external-change decision | landed by fast-forward 2026-09-08 (`42291a0`); deletable on the user's word |

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
   down to the true quest items, the blueprint sync toast recording
   LootAscension's 337 learned blueprints (the store header's
   "blueprints known" count), and — after learning a blueprint
   in-game — the guard's reload of `formulas.gst` recording one more;
   then confirm every one of those
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
1c. **Rehydrate known blueprints into the game** (user intent,
   2026-09-07, deliberately not built yet): from the store's
   `blueprints` list, (a) conjure a blueprint *item* into a stash tab
   or sack — an `Item { base_name: record, stack_count: 1 }` through
   the ordinary place path, so the game can learn it in another
   campaign; (b) bulk-add every known blueprint the open campaign's
   `formulas.gst` lacks, through the existing adds-only crafting path
   (`Formulas::add` with `FormulaRead::Unread`, database-vouched).
   Both need a UI on the list (a "Blueprints known" view in the store
   pane is the natural home) — design dialog first.
1d. **More reference cards** on `grimvault-core::reference` and
   `gui/src/reference.rs` (a `ReferenceView` field and a menu line per
   card): components and augments by the slots they fit, item sets,
   …; and an affix name in a tooltip or the inspector opening the
   affix card on that name. The partial grants of a multi-form affix
   could group by form once a witness for the form exists (the loot
   tables, not the file name).
2. **FEATURES.md 34 onward** as the user announces them — one fork
   agent per independent item in its own worktree, branch from
   `main`, decisions fixed in the brief, the eyes-only GD Stash rule,
   no bare `git stash`, no STATE.md edits by agents; the integrator
   asks each agent to rebase onto the moving `main` before landing.
3. **Rename `AutoMoveTab` / `AutoMoveTarget`** to an order-neutral
   name now that both standing orders share them (deferred on
   2026-09-07 to keep three parallel tracks from colliding).
4. **Stash file family:** decide how the `.dst` / `.gsh` twins are
   shown (further campaigns? a mode selector?) — design dialog first.
5. **Follow-ups:** a journal-based undo of a two-document edit whose
   partner half was already written when the other half was rolled
   back (today's fix reloads or keeps linked documents together; a
   written partner leaves a duplicate — `Applied` already names the
   stored id and landing); equipping by drop onto a gear tile (the `ItemSlots`
   rule per record class in `docs/format-references.md`, two-handers
   clearing the off hand, the active-set rule); level / XP edits need
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

- **2026-09-08 (evening) — foreign-icon fix landed on `main`
  (fast-forward to `183dc22`).** The user hit "item 21
  (`records/storyelements/signs/signh.dbr`) has no known footprint"
  again, copying a relic into LootAscension transfer tab 0; item 21
  is Lokarr's Gaze, whose icon lives in `gdx1`'s `Level Art.arc`, and
  a tab with one unknown footprint refuses every placement. The fix
  (`fix/icons-outside-items-arc`, built 2026-09-07 evening for the
  same tab: `GameData::foreign_bitmaps` surveys the item tables for
  bitmaps outside `Items.arc`, the loader reads each by entry from
  the archive its path names, mods first so a shipped layer wins)
  had never been fast-forwarded, and the 2026-09-08 release rebuild
  dropped it from the binary. Rebased (one additive conflict in
  `loader.rs`), gates green, `--check` on the real install shows 26
  of 26 icons and Lokarr's Gaze at 2x2. 534 tests. Why: a whole
  stash tab was unusable from the app. Risk: none to data (reads
  only); ~0.45 s more per launch for the survey; the lesson is
  procedural — a fix verified but not fast-forwarded is lost on the
  next rebuild.

- **2026-09-08 (afternoon) — lost-component fix landed on `main`
  (fast-forward to `42291a0`).** The user reported a Black Tallow lost
  after applying it from the vault to a worn medal and choosing
  "Reload from disk" on an external change: the game half was rolled
  back, the vault half stood. `links::Links` binds the two documents
  of every move, socket fill or free, bulk move and auto-move while
  either side is unsaved (`World::edited_together`), settling on save
  or reload; `flush` stops at the first document that changed on disk;
  the conflict modal names the bound documents and Reload / Keep mine
  apply to all of them. ARCHITECTURE "Data flow" records the rule. 532
  tests. Why: a vault manager must never lose an item to its own
  guard. Risk: not exercised live; a partner written before the
  conflict (game → vault, store first) still leaves a duplicate.

- **2026-09-08 (later) — FEATURES.md 33 landed on `main` (fast-forward
  to `9802232`, a fork agent in its own worktree).** The user said the
  app was missing a tab with the gear a character has equipped; the
  Equipped tab existed since M3 as a text list between the sacks and
  the stash tabs, and the dialog settled "gear tiles, first in the
  strip, unequip by drag". Core: `gdc::EquipSlot` (the twelve worn
  slots in block 3's order plus both weapon sets' hands, serde by
  name), `InventoryContents::{slot, slot_mut, slots,
  active_weapon_set}`, `EquippedItem::empty()` in the game's own
  never-used shape, `transfer::vault_from_equipment`,
  `ItemOrigin::Equipped { realm, name, slot }`. GUI:
  `DragSource::Equipped` as a source only through the one `drag::Move`
  path (autosave, backup-first and the guard unchanged), the tab first
  in the strip drawing two rows — armour + weapon set 1, accessories +
  weapon set 2, the set in hand captioned — each slot a tile with
  tooltip, inspector (socket edits on worn gear work) and the grid
  gestures; `check.rs` names slots. 529 tests; the take-off edit is in
  the lossless round-trip gate and ran over the five real characters.
  Why: a vault manager that cannot reach worn gear misses half the
  loot. Risk: whether the game accepts a slot emptied in the clean
  form is unverified in-game (it is the shape of every never-used
  slot in real files); a two-hander's ghost off hand is left as is;
  equipping by drop is not built.

- **2026-09-08 — FEATURES.md 32 landed on `main` (fast-forward to
  `cd4ed5b`, one track in the main checkout).** The user asked in chat
  for reference cards, the first a searchable list of proper affix
  names with what each grants; the design dialog settled floating
  card windows, one row per name with value ranges, expandable to the
  records, branched from `main`. Core: `reference::AffixTable` —
  every named `LootRandomizer` under `lootaffixes/{prefix,suffix}/`
  (the ascendant, completion, unique and crafting folders are
  nameless), one `AffixEntry` per name, position and rarity, its
  records as `Tier`s, collapsed into `Grant`s by stat shape with each
  varying number written as its range ("10–36% Pierce Resistance",
  "1-15 to 15-45 Lightning Damage"), a `Coverage` mark when only some
  records carry a line, and the per-skill variants of a mastery
  prefix folded into "+2 to one of 28 skills"; `AffixQuery` over
  name, grants and tier lines; `search::number_spans` made public;
  `reference_cli affixes`. GUI: a "Reference" menu in the status bar,
  the "Affix names" window with bar, filters and a four-column table
  whose rows unfold; built in the loader (`LoadStep::Reference`,
  ~0.9 s); `ui-state.json` gains `reference`; `--check` prints the
  size. Real install: 386 names over 6,191 records. 520 tests. Why:
  the user looks affixes up while sorting loot. Risk: nominal values
  only (the roll jitter is not public); the multi-form entries
  interleave their partial lines; the card was seen rendering once,
  its gestures are unit-tested only.

- **2026-09-07 (latest) — FEATURES.md 31 landed on `main` twice:
  items at `6d51441`, then knowledge at `35301b0`.** The user asked
  for blueprints to auto-sync to the vault when found in-game; the
  first cut modelled a learned blueprint as a blueprint *item* in
  the store, and the user narrowed it the same hour to "just detect
  and store", with rehydration (as an item, or bulk into a
  campaign's learned file) kept for later. Final shape: the store
  document gains a `blueprints` list — `LearnedBlueprint { record,
  campaign, learnedAt }`, one per record, absent when empty so older
  files read unchanged; `VaultStore::learn_blueprint` /
  `knows_blueprint` / `blueprints`, `merge` unions the lists;
  `bulk::blueprint_shortfall` and `bulk::sync_blueprints` record what
  the open campaign's `formulas.gst` lists and the vault does not
  know; `settings.json` `syncBlueprints` (default on);
  `World::sync_blueprints` runs in `carry_out_orders` for
  `Doc::Blueprints`, after an in-app add or import to the list, and
  when the toggle turns on — from the list's toolbar or the ⚙ modal;
  the store header shows "N blueprints known"; `--check` prints the
  plan; `vault_cli sync-blueprints`. 506 tests. Why: the user thinks
  of the vault as the record of everything found; knowledge, not
  items, so nothing can be conjured by accident. Risk: no real store
  ever carried the retired `learnedBlueprint` origin (checked before
  removing it); the first launch records 337 from LootAscension.

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
  those are one item; (8) *resolved by the user the same hour:* the blueprint sync records knowledge (the store's `blueprints` list),
  never items; rehydration into the game is next-up 1c; (9) *narrowed 2026-09-08 evening:* the icon fix is landed and its
  branch deleted by the wrap-up; still open are whether to delete
  the landed `feat/reference-cards`, `feat/equipment-tiles` and
  `fix/linked-conflicts` (and the three older `docs/state-post-*`
  branches), and whether to remove the agent worktree that holds
  `feat/equipment-tiles`; (10) *new
  2026-09-08:* a two-hander taken off leaves the game's ghost of it in
  the off-hand slot (base name blank, other fields as the weapon
  left them) — the game reads emptiness from the base name alone, so
  it was left as the game leaves it; clearing it fully is a one-line
  change if the user prefers.
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
