# Project State

The rehydration document. Any agent starting a session reads this
first to learn where the project stands right now. It answers "where
are we" — never "how does this work" (that's ARCHITECTURE.md and the
code) and never "how should we work" (that's METHODOLOGIES.md).

Last updated: 2026-09-06 (wrap-up: M4 characters editable, M5 mod
characters, mod + gdx3 game-data layers, and the campaign selector
landed on `main` at `a224d9c` by local fast-forward; the in-game
acceptance run is still pending)

## Session handoff
<!-- transient; owned by the checkpoint skill -->
**Resume here:** `main` at `a224d9c` holds everything; no other branch
exists and there is no remote (the user declined creating the GitHub
repo for now — do not push unprompted, next-up item 6). Start every
new track on its own branch from `main`; the user wants several run
in parallel, and next-up items 2–5 are independent of each other.
The **user's acceptance run** (next-up item 1) is still open: nothing
the app writes has been read by the game yet, though the user ran the
app this session and saw the external-change reload work.

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
that now watches every `player.gdc`, `--check` headless mode) — 305
tests, clippy pedantic clean. Verified on scratch copies of the
user's install and saves. **Not yet verified: the game reading any
file this app wrote** — the next step is the user's acceptance run
on the real install with the game closed; the user chose to land on
`main` before it so parallel tracks share a base. grim-vault is the Grim
Dawn sibling of tq-univault; PROJECT.md is bound with `tracker:
none`.

## Branches in flight

| Branch | Purpose | Status |
|---|---|---|
| `main` | trunk | at `a224d9c` — rules layer, read stack, vault loop, GUI shell, typed `player.gdc`, reagent storage, settings fallback, M4 character editing, M5 mod characters, mod + gdx3 game-data layers, campaign selector; 305 tests green; no remote and no GitHub repo yet |

## Next up

1. **User acceptance on the real install** (game closed): launch
   `cargo run --release -p grimvault-gui` from `main`, point Setup at `/Volumes/scott-games/steamapps/common/Grim Dawn`
   and `/Volumes/scott-games/Grim Dawn Saves/save`, vault an item out
   of the transfer stash, one out of a main-campaign sack, and one out
   of the custom-game Zark (picker entry "Zark · custom game"), place
   them back, Alt-drop one to copy it, set the iron bits, then
   confirm in-game. The stash pane should open on LootAscension (its
   stash is the newest); switch the campaign selector to the main
   campaign and back, vault from the mod stash and its component
   storage too. Load time now includes `gdx3` and the two mods (14 s
   over SMB in the headless check). Everything is already on `main`
   (user decision 2026-09-06); this run is what makes it trusted.
2. **Mod support, phase 3:** show each campaign's blueprints
   (`formulas.gst`, plaintext key-value) and illusions
   (`transmutes.gst`, already parsed); refine the default campaign
   from `playmenu.cpn` (it names the last character *and* mod
   selected, format unspecified) or remember the last selection in
   settings if the newest-stash rule ever picks wrong.
3. **M4 follow-ups:** equipment slots as drag ends (unequip to a
   sack / the store, equip from one — needs the slot-to-class rule);
   further block-1 / block-2 edits (level, attributes, skill points)
   through the same `CharacterDoc` path; a fresh seed on copy as an
   option.
4. **Game-data cache** under the config dir (names, rarity, class,
   footprint, icon RGBA per referenced record), stamp-keyed to the
   archives, so launches over the network mount stop re-reading
   ~870 MB.
5. Deferred: extract the `univault-*` crates to their own repo and
   re-point tq-univault (R1–R5 done on the vendored copies).
6. Create the GitHub repo (`scottweaver/grim-vault`, PROJECT.md's
   commented `github:` block is pre-filled) and push when ready; the
   wrap-up routine's PR steps stay inert until then.

## Most recent meaningful progress

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
