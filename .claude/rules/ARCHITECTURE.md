# Architecture constraints

Decided, binding architecture facts. STATE.md answers "what is in
flight"; this file answers "what must remain true." Every constraint
here was decided in a design dialog or by a hard external fact — a
change to any of them is a design decision requiring its own dialog
and a PR that updates this file in the same change (see
METHODOLOGIES.md "Refactors that change documented architecture").

The intuition: STATE.md is a working artifact you update aggressively;
this file is a contract you update deliberately.

Established 2026-09-03 (bootstrap dialog, pre-code — the repo held
only CLAUDE.md; constraints derived from the tq-univault sibling's
contract, the Grim Dawn install's on-disk layout, and the bootstrap
Q&A). Items marked TBD are open questions, not decisions.

## Purpose and scope

- grim-vault is the Grim Dawn sibling of tq-univault
  (`~/Projects/tq-univault`, GitHub `scottweaver/TQ-AE-Univault`): a
  platform-independent item-vault / inventory manager, filling the
  role GD Stash and GD Item Assistant fill on Windows. (2026-09-03)
- Game scope is Grim Dawn plus the installed expansions — Ashes of
  Malmouth (`gdx1`) and Forgotten Gods (`gdx2`). Fangs of Asterkarn
  (`gdx3`) is a **sanctioned, planned** overlay: adding it when
  installed is not a structural change, provided it slots into the
  overlay order recorded under "Source of truth". (2026-09-03)
- v1 is core + GUI only. A read-only MCP server and a mod forge are
  **sanctioned surfaces** (see "Planned surfaces") — building them
  needs no renegotiation here, but their recorded contracts bind
  from the first line. (2026-09-03, bootstrap dialog)
- This is **not** a security-conscious application: vault contents
  are game data stored unencrypted. The only "crypto" anywhere is the
  game's own save obfuscation, which the app decodes and re-encodes;
  no crypto layer of our own exists or is planned. (2026-09-03)

## Source of truth

- The game's own files are the authoritative store for character
  data: `save/main/_<Name>/player.gdc` (main-campaign character) and
  `save/user/_<Name>/player.gdc` (custom-game character — every mod
  shares `user/`, and the file does not name a mod; added 2026-09-06),
  `save/transfer.gst` (shared stash), `save/reagents.gst` (the
  account-wide component and crafting-material storage), and the
  crafting files `save/formulas.gst` (blueprints) and
  `save/transmutes.gst` (illusions). The game owns them; this app is
  a guest editor. `player.gdc` (in either realm), `transfer.gst`,
  `reagents.gst`, `formulas.gst`, and `transmutes.gst` (the four
  shared files in the save root or any mod's folder, see the
  campaign rule below) are the only game-owned files the app writes
  (`reagents.gst` added 2026-09-03 when block 20 was typed from the
  user's file — it is item storage, the same role as the transfer
  stash; `formulas.gst` and `transmutes.gst` added 2026-09-06 on the
  user's FEATURES.md request for blueprints and illusions, **adds
  only**: the app appends entries the record database vouches for —
  a blueprint is a record of class `ItemArtifactFormula`, an
  illusion a record whose class maps to one of the nine slot ids —
  and never removes, reorders, or re-homes one; `crafting_cli` and
  the crafting pane expose no removal, and an import refuses rather
  than moves an entry whose slot disagrees with the database);
  `playmenu.cpn` and the per-character `levels_world001.map/` trees
  are read-only until renegotiated here. (2026-09-03, extended
  2026-09-06) A character's realm is part of its
  identity only through its folder, so every store record naming a
  character carries the realm explicitly (`ItemOrigin`); a record
  without one predates realms and can only be `main/`. (2026-09-06)
  **Campaigns.** Each mod keeps its own shared files under
  `save/<Mod>/` (`transfer.gst`, `reagents.gst`, `formulas.gst`,
  `transmutes.gst`, the mod name inside each; its record database is
  a fill layer, see "ARZ/ARC archives" below). The shell opens **one
  campaign's** shared files at a time — the main campaign's beside
  `main/`, or one mod folder's — chosen by a selector whose default
  is the campaign whose `transfer.gst` the game wrote most recently:
  the best available witness of what is being played, because
  neither `player.gdc` nor the game names a character's mod (the game
  lists every `user/` character under every mod). The characters
  shown never depend on the selection. A `.gst`'s own `mod_name` is
  cross-checked against its folder and a mismatch is surfaced, never
  silently reinterpreted. Every store origin from a shared file names
  its campaign (`"campaign": "main"` or the mod name; absent means
  main, written before campaigns were recorded). Switching writes
  unsaved edits first and re-arms backup-first for the files opened.
  (2026-09-06, user request)
- The game keeps its own backup rotation (`transfer.t00`–`.t09`,
  `player.g00`/`.g01`). Those slots are the game's: the app never
  writes, renames, or deletes them, and never treats one as a write
  target. (2026-09-03)
- **One unified store file** in this app's own native format is the
  authoritative store for vaulted items: a flat, normalized set of
  items, each carrying a stable id and its full identity, in one
  versioned self-describing JSON file (`vault-store.json`, format
  tag `grimvault-store`) under the platform config directory by
  default — **or wherever `settings.json`'s `storeFile` points**
  (2026-09-07, user request: one vault shared by the Mac and a Linux
  machine over the NAS). The settings are machine-local, the store is
  portable: nothing in it is a path or a platform fact, so each
  machine's settings name the same file under its own mount point.
  One app on a store at a time is the user's job; the store rides the
  same polling guard as the game files (below), so another machine's
  write between two saves is noticed, not clobbered. Type
  buckets are **computed views** derived from each item's own base
  record, never stored membership — so an item cannot be misfiled,
  and moving or copying its bytes cannot change what it is. Buckets
  are unbounded; no capacity and no grid positions are persisted.
  The store is separate from tq-univault's (different game,
  different item identity) even though the envelope machinery is
  shared. Carried over from tq-univault's 2026-08-29 virtual-tabs
  decision. (2026-09-03, bootstrap dialog) Beside it the desktop
  shell keeps **`ui-state.json`** (format tag `grimvault-ui-state`,
  version 1): the store pane's mode, bucket, search bar and sort —
  view state, never data. Self-describing like the store (unknown
  top-level fields preserved), ignored when foreign or newer, written
  after a second of quiet and on exit; losing it costs nothing but a
  restored filter. (2026-09-06, search-view track)
- The game's ARZ/ARC archives (record database, textures, strings)
  are read-only reference data. This app never writes them. The
  record database is **layered**: `database/database.arz`, then
  `gdx1/database/GDX1.arz`, then `gdx2/database/GDX2.arz` (then
  `gdx3` when present), later layers overriding earlier records by
  record path. `database/templates.arc` and `resources/*.arc` follow
  the same base-then-expansion order. (2026-09-03) **Installed mods
  are fill layers** (user decision 2026-09-06): every
  `mods/<Mod>/database/*.arz` with the `Text_EN.arc` and `Items.arc`
  beside it is read, in folder-name order, *below* the shipped
  layers — a record, tag, or bitmap resolves from a mod only when no
  shipped layer defines it, so a mod's override of a shipped record
  never changes how the app shows a base-game item. Nothing selects
  a mod: a character file does not name one, and the game shows
  every `user/` character under every mod. The derived cache, when
  built, fingerprints the mod layers with the shipped ones.
  (2026-09-06)
- Nothing held in memory is authoritative: a mutation exists only
  once explicitly serialized to disk. (2026-09-03)
- A derived local cache of item reference data (names, footprints,
  icons) lives under the platform config directory: regenerable at
  any time, fingerprint-keyed to the archive layers it was built
  from, never authoritative, and never distributed (it embeds
  extracted game assets for local personal use only). (2026-09-03)

## Crate layering

- Shared code is **vendored into this workspace** as
  `crates/univault-engine` (copied from tq-univault with a
  provenance line per file), behind the same crate boundary the
  eventual separate repository will have: `univault-engine`
  (headless formats, ids, store and cache envelopes, game-data
  facade; never touches the filesystem) and — when they exist —
  `univault-io` (safe-io, file watcher, backup policy, debounced
  state file) and `univault-ui` (egui theme and chrome components,
  **art-free**: textures and fonts are supplied by the app).
  Extracting them to their own repository and re-pointing
  tq-univault is **deferred, not abandoned**: the boundary is what
  makes that a move rather than a rewrite. Committed manifests in
  this repo never reference a path outside it. Renegotiated
  2026-09-03 (user: fast-track a usable Grim Dawn tool over
  migrating tq-univault) from the bootstrap-dialog decision of
  "separate repo first, consumed as a git dependency";
  `docs/engine-extraction.md` records the deferred plan.
  (2026-09-03)
- This repo is a Cargo workspace with `crates/univault-engine` (the
  vendored shared engine above), `crates/grimvault-core` (GD file
  formats, store and vault logic, in-memory model — GUI-agnostic)
  and `crates/grimvault-gui` (egui/eframe front-end).
  `crates/grimvault-mcp` is added if and when the MCP surface is
  built. (2026-09-03)
- Dependencies flow shell → core → engine (`grimvault-gui` →
  `grimvault-core` → `univault-engine`; `grimvault-mcp` →
  `grimvault-core`; shells additionally use `univault-io` and
  `univault-ui`), never the reverse, never shell → shell, and
  `grimvault-core` never depends on `univault-io` or `univault-ui`.
  Falsifiable check: `grimvault-core` compiles headless with no egui,
  eframe, winit, rmcp, or tokio anywhere in its dependency tree, and
  no `std::fs` use outside tests. (2026-09-03)
- The GUI framework is egui/eframe; the core/gui split exists
  precisely so this remains swappable without touching core.
  (2026-09-03)
- Async is confined to shell crates; core and the engine crate stay
  sync and pure. (2026-09-03)

## Data flow

- Load: core parses game files and the store into a typed model.
  Edit: the UI mutates the model only. Save: core serializes and the
  shell writes **automatically** once an edit has been quiet briefly
  (autosave); there are no manual save buttons. All format knowledge
  lives in core; the shell only decides *when* to write.
  (2026-09-03, carried over from tq-univault's 2026-08-25 autosave
  decision)
- **Full re-encode, not splice** — the one departure from
  tq-univault's write rule. GD saves (`player.gdc`, `*.gst`) are
  XOR-obfuscated with a rolling key that every ciphertext byte feeds
  (cipher confirmed 2026-09-03 across four implementations and
  verified byte-for-byte against real saves the same day,
  `docs/format-references.md`), so a byte-level splice cannot exist.
  Writes are decode → typed model → full re-encode under one rule,
  **lossless model**, enforced at two points: (1) every file the
  parser accepts must re-encode byte-for-byte when unmodified — a
  test gate run against real saves; (2) before any write, core
  re-encodes the unmodified baseline and compares it to the bytes
  read at load; a mismatch aborts the save with a visible error.
  **Opaque blocks are not re-keyable** (found 2026-09-03): a u32
  field and four single bytes decode differently under the rolling
  key, so a block whose field layout is unknown can only be
  re-encoded under the exact key it was read with. An unrecognised
  block is carried opaquely for reading and for writes that leave
  everything before it untouched, and **every block after an edited
  one must be fully typed** before that edit may be written;
  `encode` refuses otherwise. Every `player.gdc` block in the
  observed sequence is typed (ported from yagde, MIT; skills v8 and
  stats v12 established from real saves), so `transfer.gst` and
  `player.gdc` are both writable; an unfamiliar block id or version
  still parses opaquely and keeps that file read-only. (2026-09-03,
  bootstrap dialog; opaque rule and full typing added the same day
  from the parser work)
- Every write to a game-owned file goes through a backup-first
  write path: this app's own backup of the file exists on disk
  before the original is touched, **one backup per load** — the
  first write since the file was last loaded takes the backup;
  subsequent autosaves of the same loaded baseline reuse it, so
  per-edit writes cannot churn the pre-session state away.
  (Re)loading a file re-arms the backup. Writes are synced in place after the backup (tq-univault's proven
  path; staging-then-rename was declined) and re-read to verify
  before the save is reported done. This app
  mutates people's save files; this constraint is non-negotiable.
  (2026-09-03)
- The shell watches the open files by polling and keeps panes
  current: an external change reloads a clean pane automatically,
  but only once the change is *believed* — the file's stamp must
  hold stable across two polls, and a read that comes back empty
  must be corroborated before it clears a pane that held items.
  **The app never knowingly overwrites an externally-changed file
  without the user choosing to**: every save re-checks the file
  against the stamp taken at load/last write, and a mismatch — or an
  external change to a dirty pane — suspends autosave and prompts.
  "Keep mine" re-arms backup-first so the external version is backed
  up before being overwritten. (2026-09-03) TBD (2026-09-03): the
  corroboration witness for an empty stash read — tq-univault uses
  the game's `.dxg` twin; GD's nearest analogue is the
  `transfer.t00` rotation slot, unverified.
- All game-file reads and writes go through the shared safe-io
  path: uncached reads, length-checked against the file's own
  metadata, so a stale or mid-write read over a network mount is a
  retryable error rather than a silently wrong pane. The user's
  install and saves live on a NAS written by a separate machine;
  tq-univault paid for this rule on 2026-08-31. (2026-09-03)
- **Standing orders write on the app's own initiative (2026-09-07,
  FEATURES.md 14–15; the purge added the same day, FEATURES.md 23).**
  A transfer-stash or character-stash tab the user has nominated in
  `settings.json` (`autoMove`, each entry carrying the tab's whole
  identity — campaign, or realm and character name — plus the index)
  is emptied into the store whenever the app loads it, the user
  nominates it, the campaign selector opens it, or the guard reloads
  it after a *believed* external change (the two-poll rule above).
  That is a write to `transfer.gst` or `player.gdc` that no drag
  caused: it still rides the ordinary autosave path — backup-first
  once per load, the stamp re-check, the external-change guard — and
  never runs on a dirty pane (a reload only ever replaces a clean
  one) or while the conflict modal is up. An item the store already
  holds under the same record and roll seed is left in the tab by the
  auto-move under the default `bulkDuplicates: skip` and moved under
  `allow` — one rule in `settings.json` for every bulk move or copy
  into the store, a tab's "Move all" / "Copy all" buttons and the
  standing order alike, while single drags, double-clicks and
  right-clicks always land (2026-09-07, FEATURES.md 24); the
  auto-move never destroys one (`grimvault-core::bulk`; the identity
  rule is in `docs/format-references.md`). **The purge
  (`purgeDuplicates`, the same entry shape) is the one order that
  destroys items:** a nominated tab loses every item whose record and
  roll seed the store already holds — membership is the store's
  alone, so two in-tab copies of a seed the store lacks both stay,
  and stacks and unrolled (zero-seed) items are never duplicates, and
  `bulkDuplicates` has no say — at the same moments, under the same
  gate and autosave path, with the store never changed; the auto-move
  runs first, so a tab under both orders ends empty. The
  component-storage sync (`syncReagents`, default on) runs at the
  same moments but writes only the store: it adds one stack of the
  shortfall per record and never removes, reduces, or touches
  `reagents.gst`. A headless `--check` run from the saved settings
  prints the rule and what all three orders would do without
  writing.

## External boundaries

- External boundaries are the file formats — the GD character/stash
  encoding (`.gdc`/`.gst`, rolling XOR), ARZ (LZ4-compressed
  records) and ARC archives, and this app's store format — each
  guarded by its own module with a typed read/write surface. Format
  modules that are engine-generic (ARC/ARZ container parsing, the LE
  reader) live in the engine crate; GD-specific record and save
  shapes live in `grimvault-core`. ARC and ARZ are **one parser
  each**, shared with tq-univault and parametrized by codec (zlib vs
  raw LZ4 block) and ARZ dialect; a forked GD copy is structural.
  (2026-09-03, engine-extraction dialog)
- **GD Stash's export file (`.gds`) is a read-only import boundary**
  (2026-09-06, resolving the 2026-09-03 TBD; FEATURES.md item 3):
  versions 1–3 as laid out in `docs/format-references.md`, guarded
  by `grimvault-core::gds` — `parse` turns the bytes into a typed
  `GdsExport` once, refusing the whole file on any malformed byte,
  and `import` adds its entries to the store under
  `ItemOrigin::GdStashExport { file, mode, owner }`, the export's
  file name, the softcore/hardcore mode, and the soulbound owner —
  the facts only that file records, carried by every imported entry
  so it never depends on which file it came from. This app **never
  writes `.gds`**: the store stays the authority, facts flow in and
  nothing flows back. Duplicate identity is the whole exported fact
  (every item field, stack count included, plus mode and owner), so
  importing a file twice adds nothing and a changed stack is a new
  entry, never a silent reconciliation; an entry whose base record
  no database layer defines is imported and named in the report,
  never dropped. GD Item Assistant's `.ias` is recorded in
  `docs/format-references.md` but not implemented; reading it is a
  boundary of the same kind and gets its own entry here first.
  Falsifiable: no `.gds` writer exists anywhere in the workspace,
  and `gds::import` only ever calls `VaultStore::add`.
- This app's own interchange documents — `grimvault-blueprints` and
  `grimvault-illusions`, one JSON file each with the vault store's
  envelope (`format` tag, `version`, unknown fields preserved) plus
  the `campaign` and `exportedAt` they were taken from — carry a
  blueprint list or an illusion collection between campaigns. Read
  and written only by `grimvault-core::{blueprint, illusion}`; a
  document of any other tag is refused by name, and an import adds
  what the record database vouches for and reports the rest — never
  removes. (2026-09-06) **A vault export is the store format itself**
  (2026-09-07): "Export a copy…" writes the open store as it is to a
  file of the user's choosing, and "Import from a copy…" is
  `VaultStore::merge` — every entry of another store file whose
  vaulting event (origin, moment, item) this store lacks is added
  under a fresh id of this store's own; nothing is removed, changed,
  or re-identified, so importing a store into itself or the same copy
  twice adds nothing. No new format, no new boundary: both files are
  `grimvault-store` documents.
- No network services, no telemetry, no online features. stdio IPC
  for the planned MCP surface is not a network service.
  (2026-09-03)

## Platform independence

- Must build and run on Windows, macOS, and Linux. (2026-09-03)
- OS-specific logic — game-directory and save-directory discovery,
  covering both the local `Documents/My Games/Grim Dawn/save/`
  layout and the Steam-cloud `userdata/<id>/219990/remote/save/`
  layout, plus path conventions — is confined to a single platform
  module; no `cfg(target_os)` sprawl elsewhere. (2026-09-03)
- Pure-Rust dependencies preferred; a native/C dependency needs a
  reason recorded here. (2026-09-03)

## Parser provenance and dependencies

- The parsing foundation is the shared hand-rolled typed
  little-endian reader plus a pure-Rust LZ4 decoder (`lz4_flex`
  unless a reason is recorded here) for ARZ records. No parser-derive
  proc-macros (binrw was declined in tq-univault's 2026-08-24 survey;
  the decision carries). (2026-09-03)
- tq-univault's own code (MIT OR Apache-2.0, same author) is freely
  reused. External porting reference for GD formats:
  [dandels/gdlc](https://github.com/dandels/gdlc) (Rust, MIT).
  [gregates/lib-gddb](https://github.com/gregates/lib-gddb) is GPL —
  eyes-only, never transcribed. GD Item Assistant
  (`marius00/iagd`, MIT) is a sanctioned secondary reference for
  ARZ, ARC, and `.tex` and contains no save decoder. **GD Stash**
  (mamba, Java, closed source, no published license) is an
  **eyes-only** reference (user decision 2026-09-06): the user's
  copy at `/Volumes/scott-games/GDStash_v190a` decompiles cleanly
  (CFR, `brew install cfr-decompiler`) into the session scratchpad,
  and the jar ships its author's format notes as plain text. It is
  read for **format facts** — field order, block ids, enum values,
  which game record a number comes from — each verified against
  real files before use; no line of its code and none of its
  hard-coded data tables (shrine and rift-gate UIDs, tag-to-text
  maps) is transcribed, and decompiled output never enters this
  repository. The reference map is `docs/format-references.md`
  (written 2026-09-03). gdlc covers the save/stash encoding end to end,
  corroborated by three independent implementations (resolved
  2026-09-03, engine-extraction survey).
- The project is dual-licensed MIT OR Apache-2.0. (2026-09-03)

## Planned surfaces (sanctioned, not v1)

Building any of these needs no renegotiation of this doc; the
contract recorded here binds from the first commit.

- **Read-only MCP server** (`crates/grimvault-mcp`): JSON-RPC over
  **stdio only** — the client spawns it as a child process; no
  listening sockets, ever. Exposes characters, the stash, the store
  and its buckets, and the layered record database; never writes any
  file. Adding write tools or a network transport is structural.
  (2026-09-03)
- **Mod forge**: serializes this app's own record edits into new
  bundles under the game's `mods/` directory, optionally merged onto
  a base mod. Shipped databases and third-party mod files are never
  modified; a composed bundle is always a new folder, deletable
  without trace. (2026-09-03)
- **Fangs of Asterkarn (`gdx3`)**: a further overlay layer in the
  order recorded under "Source of truth". (2026-09-03) Read since
  2026-09-06 whenever `gdx3/database/GDX3.arz` exists — the user's
  install has it, and its records (the Asterkarn quest items, for
  one) were the "unknown records" seen on the mod character.

## Audit triggers

Files whose changes warrant re-checking this doc during post-merge
cleanup:

- `Cargo.toml` (workspace root), `crates/*/Cargo.toml`, `Cargo.lock`
  entries for the engine crates, and `.cargo/config.toml` —
  dependency-direction, git-vs-path, and native-dep constraints
- `crates/grimvault-core/src/*.rs` format modules (save/stash codec,
  ARZ/ARC layering, store, platform) — boundary contracts, the
  lossless-model rule, and the platform-confinement rule
- `crates/grimvault-core/src/gds.rs` — the `.gds` import stays
  read-only (a writer or an `.ias` reader is a new boundary entry)
- Any module implementing the save/write-back path — backup-first,
  refuse-on-mismatch, and the game's-rotation-is-untouchable rules
- `crates/grimvault-core/src/{formulas,blueprint,illusion}.rs` and
  `crates/grimvault-gui/src/crafting.rs` — the adds-only rule for
  `formulas.gst` / `transmutes.gst` and the database gate on what
  they admit
- `crates/grimvault-gui/src/main.rs` — entry point / framework choice
- `crates/grimvault-mcp/src/*.rs` (when it exists) — read-only and
  stdio-only
- `docs/format-references.md` — provenance and license records
- `docs/engine-extraction.md` — the shared-crate boundary and phase
  plan

## Structural criteria

Structural (this doc must change in the same PR): a GUI or async
dependency appearing in core or any reversal of the crate DAG; a
committed path dependency outside this repo, or game-specific code
leaking into `univault-engine`; a new external boundary (network access, a
new file format, an interchange format, telemetry); a change to who
holds authoritative state; writing to any game-owned file not listed
as writable above, to the game's backup rotation, or to ARZ/ARC files
(composed mod bundles are the sanctioned exception); a change to the
store format's identity rules (a breaking schema version, stored
bucket membership, splitting the store); weakening or bypassing
lossless-model, backup-first, or the external-change guard;
reintroducing byte splicing of game-owned files; adding write tools
or a network transport to the MCP surface; adopting a parser-derive
dependency; transcribing from a GPL or license-unknown reference; a
license change; replacing egui/eframe; dropping a supported platform.

Not structural (no update needed): new UI panels or item operations
behind unchanged file boundaries; parser internals behind an
unchanged typed surface; adding the `gdx3` overlay or standing up the
MCP / mod-forge surfaces within their recorded contracts; test
changes; new pure-Rust dependencies that respect the layering; engine
crate version bumps.

## Maintenance

Update when a constraint above is deliberately renegotiated (design
dialog + PR updating this file), or when a recorded TBD is resolved.
Never for in-flight status — that's STATE.md. Keep constraints
falsifiable and dated. Secrets never enter this doc.
