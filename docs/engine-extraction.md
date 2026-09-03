# Shared engine extraction — proposal

Status: **DECIDED 2026-09-03** in the design dialog recorded under
"Decisions" at the end. Source: a read-only survey of tq-univault
(`crates/univault-core` 15,631 LOC / 25 files, `crates/univault-gui`
10,307 LOC / 13 files, at main `36e7774`) plus the Grim Dawn format
research recorded in `format-references.md`. Line references below
are into tq-univault at that revision.

## Verdict

Roughly 3.9k lines of tq-univault core are game-agnostic today: the
little-endian reader and writer, the ARC and ARZ container parsers,
localization text tables, `.tex` decode, first-fit grid placement, and
the stat-format language. Another ~2.3k lines (store envelope, cache
framing, the game-data facade) become shareable behind one trait and
one generic. Everything save-related, the item shape, the rarity /
gear / stat vocabularies, and the TQVaultAE interop are TQ-specific
and get Grim Dawn siblings in `grimvault-core`.

On the shell side, `safe_io`, the file watcher and refresh tracker,
the debounced JSON state file, the theme, and the chrome components
lift with no game types at all. Grids, drag-and-drop, and autosave
need a `Document` / `GridItem` trait first because tq-univault's app
holds its documents in a hard-coded enum.

The two container formats differ only in compression codec (TQ zlib,
GD raw LZ4 block) and one 4-byte field in the ARZ record entry, so
ARC and ARZ are **one parser each, parametrized** — not forks. The
save formats share nothing beyond the byte reader: TQ is plaintext
with block magics, GD is a rolling-XOR stream with id/length/checksum
blocks.

## Proposed repository layout

One new repository (working name `univault-engine`), dual-licensed
MIT OR Apache-2.0 like both consumers, holding three crates:

| Crate | Role | May depend on | Must not depend on |
|---|---|---|---|
| `univault-engine` | Formats, ids, store and cache envelopes, game-data facade. Pure: takes bytes, returns bytes or typed models. | `serde`, `serde_json`, `thiserror`, `flate2`, `lz4_flex` | `std::fs` (falsifiable: no `fs::` use outside tests), egui, tokio |
| `univault-io` | `safe_io` (uncached verified reads, backup-first writes), file watcher + refresh tracker, backup policy, `DebouncedJsonFile<T>`, recents | `serde_json`, `libc` (macOS `F_NOCACHE`) | egui, any game type |
| `univault-ui` | Theme, chrome components, one slicing helper, grid geometry, sort direction, search widgets, review overlay + preview harness behind a `dev` feature. **Art-free:** textures and fonts are injected by the app. | `egui`, `eframe`, `egui_extras`, `image` (png) | game types (phase 1); a `GridItem` trait in phase 3; bundled PNGs or fonts |

Why three crates and not one:

- tq-univault's core is **IO-free by design** and its STATE.md
  explicitly forbids moving `safe_io` into core "on the quiet"
  (next-up item 4). A separate io crate honours that rule, and gives
  `univault-mcp` the uncached reads it currently lacks.
- The ui kit is separate so the engine compiles headless, which is
  the falsifiable check in both projects' ARCHITECTURE.md.

Consumers after the split:

```
tq-univault:  univault-core → engine
              univault-gui  → io + ui (+ core)
              univault-mcp  → engine + io (+ core)
grim-vault:   grimvault-core → engine
              grimvault-gui  → io + ui (+ core)
```

## Module dispositions — `univault-core`

| Module | LOC | Disposition | What has to change |
|---|---|---|---|
| `reader.rs` | 333 | **engine** as-is | GD wraps it with a rolling-XOR cursor; API unchanged |
| `writer.rs` | 80 | **engine** | make `pub` (currently `pub(crate)`) |
| `arc.rs` | 474 | **engine**, codec-parametrized | zlib hard-wired at `:170`; TQ skips 2 bytes per part, GD is raw LZ4 block and stored-raw when sizes match |
| `arz.rs` | 863 | **engine**, codec + dialect | zlib at `:248`/`:455`; header check `0x0003_0004` at `:488`; GD entry adds a `decompressed_size: u32` where TQ has two timestamp i32s; depends on `chr::RecordId` (`:25`) |
| `text.rs` | 220 | **engine** as-is | none |
| `tex.rs` | 249 | **engine** | DXT decode (`:99-101` unsupported) matters more for GD |
| `grid.rs` | 133 | **engine** | depends on `chr::GridPos` |
| `stats/format.rs` | 462 | **engine** | make `pub`; same `{%.0f0}` language and colour tags in GD |
| `style.rs::palette_color` | ~20 | **engine** (with `text`) | colour-letter map is engine-wide; rest of `style.rs` stays TQ |
| new `ids.rs` | — | **engine** | `RecordId`, `normalize`, `ItemSeed`, `GridPos` extracted from `chr.rs:24-80` and `arz.rs` |
| `platform.rs` | 111 | **engine** `config_dir(app_name)` | hard-codes `tq-univault` (`:17,21,28`); stash helpers (`winsys.dxb` etc.) move to TQ core |
| `store.rs` | 852 | **engine** envelope, phase 2 | `VaultStore<I>` generic over the item DTO; TQ DTO, `Bucket`/`Family`, `export_to_vault` stay |
| `cache.rs` | 661 | **engine** framing, phase 2 | `UVC8` framing + `CacheEntry<P>`; entry payload codec (`:475-546`) is TQ enums |
| `gamedata.rs` | 829 | **engine** facade, phase 2 | behind a `GameProfile` trait; class prefixes, `XPACK*`, bitmap/name variable dispatch, `build_cache` → `stats::Renderer` are TQ |
| `transfer.rs` placement (`:145-270`) | ~120 | **engine** with `grid`, phase 2 | `place_in_*`, `fits_at`, `occupancy` are generic; everything else TQ |
| `query.rs` filter machinery | ~200 | **defer** (phase 3) | generic in shape, every enum it filters is TQ |
| `chr.rs` parsers/encoders | ~1300 | TQ | landmark keys, 12-slot equipment, Atlantis triple, splice writers |
| `stash.rs`, `vault.rs`, `dllpatch.rs`, `respec.rs`, `skilltree.rs`, `style.rs`, `stats/{mod,dictionary,render}.rs`, rest of `transfer.rs` | ~7000 | TQ | GD writes its own siblings (see below) |

## Module dispositions — `univault-gui`

| Module | LOC | Disposition | What has to change |
|---|---|---|---|
| `safe_io.rs` | 329 | **io** | parametrize the `univault-bak` suffix (`:144,:208`); `libc` gate travels with it |
| `main.rs` watcher, `RefreshTracker`, `WatchHealth`, `write_through`, autosave timing, `Recents`, `Toast` (`:436-613`, `:756-816`, `:2022-2086`, `:2401-2426`, `:2585-2597`) | ~600 | **io**, phase 3 | need a `Document` trait; `SourceStamp` (`univault_core::cache`) becomes an io-local `(size, mtime)` type |
| `ui_state.rs` | 257 | **io** `DebouncedJsonFile<T>`, phase 2 | payload (`LeftTab` etc.) stays in-app |
| `theme.rs` | 192 | **ui** | `Theme { palette, fonts }` built from app-supplied font bytes; the two `include_bytes!` fonts (`:29-30`) stay in tq-univault; rename `"tq-heading"` (`:70`, `chrome.rs:179`) |
| `components/{gilded_border,tabbed_panel}.rs` | 697 | **ui**, textures injected | the PNGs are original art (commits `8a4c37e`, `13c00ce`) but stay in tq-univault; components take an image source instead of `include_bytes!`; unify slicing first |
| `chrome.rs` mechanics (`Src`, `blit`, `three_slice`, `light_stone`, `button`, `tooltip_frame`) | ~250 | **ui** | texture keys, slice rects, `SlotPlate` stay in-app |
| `sort.rs` | 78 | **ui** | none |
| `search.rs` widgets (`filter_field`, `suggesting_field`, sort-header button, `row_height`) | ~150 | **ui** | `FilterDraft`, rarity list, expansions stay in-app |
| `main.rs` grid geometry (`fit_cell_size`, `cells_at`, `point_to_cell`, `paint_grid_background`, `paint_fit_preview`, `shelve_items`) | ~200 | **ui** | pure geometry |
| `main.rs` `grid_view` / gestures / drop preview (`:4457-4818`, `:5943-6042`) | ~900 | **ui**, phase 3 | `GridItem` trait + `DropRules` callback; dedupe the two copies first |
| `review.rs`, `bin/preview.rs` | 658 | **ui** `dev` feature | export dir hard-wired to repo layout (`review.rs:16`) |
| `main.rs` paper doll, toolbar, help, modals, import archive names, DLL patch, respec, bonus picker | ~3500 | app | TQ throughout |

## The seams (parametrization points)

1. **Codec.** `trait BlockCodec { decompress(src, expected_len:
   Option<usize>); compress(src) }` with `Zlib` (TQ; ARC parts skip
   2 bytes) and `Lz4Block` (GD; raw block, size supplied from the
   entry, stored-raw when compressed == decompressed).
2. **ARZ dialect.** `ArzDialect { codec, entry_has_decompressed_size,
   first_dword }`. Header, string table, and record payload are
   byte-identical across games.
3. **Ids.** `RecordId`, `normalize`, `ItemSeed`, `GridPos` move to
   `engine::ids`. This is the prerequisite for everything else.
4. **`GameProfile` trait** for the game-data facade: `is_item_class`,
   `bitmap_variable`, `name_tag`, `class_footprint_bound`,
   `text_file_order`, `resource_archive_for`, `build_cache_entry`.
5. **Store envelope.** `VaultStore<I: StoredItem>`. The per-app
   format tag (`univault-store` / `grimvault-store`) *is* the
   identity: the loader refuses any other tag, so a GD store can
   never be opened as a TQ one. The survey flagged that the envelope
   carries no game field; the tag serves that role and the engine
   must make it a required parameter rather than a constant.
6. **Cache.** Shared `UVC8` framing, `CacheEntry<P>` with a per-game
   payload codec and payload version; the format tag rides in the
   header for the same reason as the store.
7. **Platform.** `config_dir(app_name)`; save and game discovery stay
   per game.
8. **safe_io.** Backup suffix parametrized.
9. **Theme and art.** `Theme { palette, fonts }` and every component take
    textures and fonts from the app; the kit bundles no PNGs or fonts.
10. **Later:** `Document` (path, stamp, dirty, reload, save →
    `SaveOutcome`, item_count, mid_save_evidence) for autosave and
    refresh; `GridItem` (position, footprint, icon) + `DropRules` for
    the grid view.

## What Grim Dawn writes itself (`grimvault-core`)

- **Save codec:** a rolling-XOR cursor over the engine reader/writer
  with id / length / checksum block framing (`format-references.md`
  §5c), `.gdc` and `.gst` parsers, a versioned item DTO (v8 and v11
  add fields), and the full re-encode + lossless-model gate from
  ARCHITECTURE.md.
- Item model, rarity ladder (Common → Legendary), gear classes, a GD
  stats dictionary and renderer (aether, chaos, vitality,
  retaliation, conversion), category vocabulary, and expansion origin
  (`gdx1` / `gdx2` / `gdx3`).
- Save-directory discovery for the local and Steam-cloud layouts.

## Prerequisite refactors in tq-univault, in order

Each is a refactor commit answering METHODOLOGIES.md's four
questions; tq-univault's 274 tests are the guard.

| # | Refactor | Unblocks |
|---|---|---|
| R1 | `ids` module: `RecordId`, `normalize`, `ItemSeed`, `GridPos` out of `chr.rs` / `arz.rs` | arc, arz, grid, store |
| R2 | `BlockCodec` + `ArzDialect` in `arc.rs`, `arz.rs`, `arz::compose` | GD ARZ/ARC on the same parser |
| R3 | Visibility: `writer`, `stats::format`, block constants → `pub` | the crate boundary |
| R4 | One nine/three-slice helper replacing the three copies in `chrome.rs`, `tabbed_panel.rs`, `gilded_border.rs` | ui kit without shipping three slicers |
| R5 | `platform::config_dir(app_name)`; stash-path helpers out of `platform` | engine `platform` |
| R6 | `GameProfile` trait; `gamedata::build_cache` stops calling `stats::Renderer` / `style::*` directly | phase 2 facade |
| R7 | `VaultStore<I>` with a required format tag | phase 2 store |
| R8 | `CacheEntry<P>` with per-game payload codec | phase 2 cache |
| R9 | `DebouncedJsonFile<T>` out of `ui_state.rs` | phase 2 io |
| R10 | Dedupe `grid_view` / `grid_view_store` | phase 3 |
| R11 | `Document` trait under autosave / refresh / conflict guard | phase 3 io |
| R12 | `GridItem` + `DropRules` under the grid view | phase 3 ui |

## Phases

**Phase 1 — read-only foundation.** Unblocks grim-vault's next-up
item 2 (decode real GD files). Contents: engine `{ids, reader,
writer, arc, arz, text, tex, grid, format, palette}`; io `{safe_io}`;
ui `{theme, components, slicing, sort, review + preview}`. Needs
R1–R5. Exit criteria: tq-univault builds and passes its 274 tests
against the git dependency; grim-vault decodes `database.arz`,
`GDX1.arz`, `GDX2.arz`, and an `Items.arc` with the engine parsers.

**Phase 2 — persistence and facade.** Store and cache envelopes,
`GameProfile`, `platform`, `DebouncedJsonFile`, placement helpers.
Needs R6–R9. Exit: grim-vault's store and cache use the shared
envelopes with the `grimvault-store` tag.

**Phase 3 — shell engine.** Watcher / refresh / autosave over
`Document`; generic grid view with `DropRules`; search widgets.
Needs R10–R12. This touches tq-univault's 6.5k-line `main.rs` and is
the largest and least urgent step; grim-vault can ship a first GUI
with an app-local copy of the autosave loop if phase 3 lags.

## Extraction mechanics

- Refactor **in place in tq-univault first** (that is where the
  tests live), then extract. Extracting a raw copy and refactoring in
  the new repo would strand the tests.
- Copy the moved files into the new repo with one provenance commit
  (`extracted from tq-univault @ <sha>`); history stays in
  tq-univault (decided over `git filter-repo`, which is installed
  but was judged not worth a rewritten first history).
- tq-univault then swaps to
  `univault-engine = { git = "…", rev = "…" }`; local development
  uses a `[patch]` in an uncommitted `.cargo/config.toml` (already
  the ARCHITECTURE.md rule for grim-vault).
- The engine repo's CI mirrors tq-univault's `ci.yml`: rustfmt,
  clippy pedantic `-D warnings` + tests on Linux / macOS / Windows,
  llvm-cov. The ui crate needs the same Linux xcb/xkbcommon packages.
- Docs travel with code: the ARC/ARZ/`.tex` sections of tq-univault's
  `docs/format-references.md`, and the WORKING_NOTES section
  "Reading a save the game is still writing" (the `safe_io`
  mechanism), move to the engine repo.
- Test fixtures (`chr::fixture`, `arz::fixture::ArzBuilder`,
  `arc::fixture::build_arc`, `tex::fixture`) move with their module
  behind a `test-util` feature so consumers can build synthetic
  archives.

## Decisions (2026-09-03 design dialog)

1. **Names:** repo `univault-engine`; crates `univault-engine`,
   `univault-io`, `univault-ui`.
2. **Phase 1 scope:** pure modules only (R1–R5); the store and cache
   envelopes wait for phase 2.
3. **History:** plain copy with a provenance commit; tq-univault
   keeps the history.
4. **Art:** the ui kit is art-free. Components and the theme take
   textures and fonts from the app; tq-univault keeps its gold and
   bronze PNGs and OFL fonts, grim-vault bundles its own.

## TBDs in ARCHITECTURE.md this survey resolves

- **Save/stash encoding reference:** dandels/gdlc (Rust, MIT) covers
  the rolling-XOR cursor, `player.gdc`, and `transfer.gst` end to
  end; corroborated by three independent implementations.
- **GD Item Assistant** is `marius00/iagd` (C#, MIT): usable for
  ARZ / ARC / `.tex`; it has **no save decoder** (it hooks the game
  DLL instead).
- **GD Stash** is closed source with no published license; eyes-only
  is moot because there is nothing to read.
- **Compression dependency:** `lz4_flex` block API (raw block,
  decompressed size supplied), pure Rust — no reason to record a
  native dep.
