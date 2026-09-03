# Grim Dawn file-format references

Outcome of the 2026-09-03 format survey run while planning the shared
engine extraction (see `engine-extraction.md`). Verdict: **no
crates.io crate covers any GD format**; parsers are hand-written,
with the ARC/ARZ containers shared with tq-univault through the
engine crate and the save codec written here. Binding rules about
provenance and dependencies live in `.claude/rules/ARCHITECTURE.md`
("Parser provenance and dependencies"); this file is the working
reference map. ARCHITECTURE.md requires this file to exist before the
first parser PR merges.

## Provenance rules (summary)

- **Primary reference for GD formats:**
  [dandels/gdlc](https://github.com/dandels/gdlc) (Rust, **MIT**,
  v0.3.4, last push 2026-08-11). Covers ARZ, ARC (both LZ4 block),
  `player.gdc`, `transfer.gst`, and the rolling-XOR decoder
  (`src/decrypt.rs`). No `.tex` support. Line-by-line porting is
  allowed; ported code preserves the MIT notice (dandels, 2025).
- **Secondary MIT references:**
  [marius00/iagd](https://github.com/marius00/iagd) (GD Item
  Assistant, C#, MIT) for ARZ (`Parser/Arz/ArzParser.cs`), ARC
  (`Parser/Arc/ARCHeader.cs`, `Decompress.cs`), and `.tex` → DDS
  (`Parser/Arc/DDSImageReader.cs`). It has **no save decoder** — it
  hooks the game DLL and reads CSV drops.
  [ChrisElison/GDParser](https://github.com/ChrisElison/GDParser)
  (C#, MIT, derived from IAGD) for `CryptoDataBuffer.cs`.
  [nbak/grim-save-parser](https://github.com/nbak/grim-save-parser)
  (Rust, MIT) and [wr8fdy/yagde](https://github.com/wr8fdy/yagde)
  (Rust, MIT) as save-side second opinions.
- **Eyes-only (never transcribed):**
  [gregates/lib-gddb](https://github.com/gregates/lib-gddb) (Rust,
  **GPL-3.0**, ARZ/ARC only);
  [Krauzy/dreeg](https://github.com/Krauzy/dreeg) (`gd-db`,
  `gd-save` crates declare **EPL-1.0** alongside an MIT notice —
  ambiguous, treat as eyes-only until clarified);
  [Odie/gd-edit](https://github.com/Odie/gd-edit) (Clojure, license
  not checked) and
  [AaronHutchinson/Grim-Dawn-Save-Decryption](https://github.com/AaronHutchinson/Grim-Dawn-Save-Decryption)
  (C++, license not checked) as cross-checks of the cipher.
- **GD Stash** (mamba, Java) is closed source with no published
  license. Nothing to read.
- **Cross-game prior art:** tq-univault's own parsers (MIT OR
  Apache-2.0, same author) and their TQVaultAE lineage; see
  tq-univault `docs/format-references.md`.

## Per-format map

### ARZ (record database, read-only) — shared container

- **Header: identical to TQ**, 24 bytes: `0x00` first dword, `0x04`
  record-table offset, `0x08` record-table size, `0x0C` record count,
  `0x10` string-table offset, `0x14` string-table size. GD readers
  split the first dword as `u16 unknown (=2), u16 version (=3)`
  (lib-gddb, IAGD `GRIMDAWN_ARZ_V3_HEADER`) — GD reads as
  `0x0003_0002` and TQ AE as `0x0003_0004`, both verified on real
  files 2026-09-03. Modelled as `ArzDialect.first_dword`.
- **String table: identical** — `u32 count`, then `u32 len + bytes`
  per string.
- **Record-table entry: differs by one field.** TQ: `i32 id-string
  idx; len-prefixed record type; i32 offset; i32 compressed size; i32
  ts; i32 ts`. GD: `u32 idx; len-prefixed type; u32 offset; u32
  compressed; u32 decompressed; 8 bytes (FILETIME / padding)`. Neither
  is fixed-size (variable type string).
- **Compression:** TQ zlib per record; GD **raw LZ4 block, no frame
  header**, decompressed size supplied from the entry
  (`lz4_flex::block::decompress` with the size, never
  `decompress_size_prepended`). Records are **never stored raw**: 63
  of 69,089 real GD records are genuine LZ4 blocks exactly as long
  as their payload, so an equal-size heuristic corrupts them
  (verified 2026-09-03, engine sweep). The engine also checks the
  inflated length exactly, since the container size is the block's
  only integrity check.
- **Decompressed record payload: identical** — `u16 type (0 int, 1
  float, 2 string-idx, 3 bool), u16 count, u32 key-string-idx`, then
  `count` × 4-byte values.
- **Files:** `database/database.arz`, `gdx1/database/GDX1.arz`,
  `gdx2/database/GDX2.arz` — layered in that order.

### ARC (resource archives, read-only) — shared container

- **Header identical to TQ** (7 × u32 = 28 bytes): magic `"ARC\0"`,
  version (3 in both GD and TQ AE, verified 2026-09-03), numFiles, numParts, partTableSize (= parts × 12),
  stringTableSize, partTableOffset.
- **Part entries (12 bytes) and TOC entries (44 bytes) identical:**
  TOC = `type (1 uncompressed, 3 compressed), offset, compSize,
  decompSize, 12 bytes (unknown + FILETIME), numParts, firstPart,
  nameLen, nameOffset`.
- **Compression differs:** TQ zlib per part with a 2-byte skip
  (`offset + 2, size − 2`); GD LZ4 block per part, stored raw when
  `compressed == decompressed` (94 of Items.arc's 6,729 entries, all
  beginning `TEX\x02`). The raw rule is an ARC rule only — see ARZ.
- **Files:** `database/templates.arc`, `resources/*.arc`
  (`Items.arc`, `Text_EN.arc`, `UI.arc`, …), expansions under
  `gdx1/resources/`, `gdx2/resources/`.

### Save encoding (`player.gdc`, `*.gst`) — GD-only codec

Four independent implementations agree (gdlc `decrypt.rs`,
AaronHutchinson `decrypt-player.cpp`, gd-edit `gdc.clj`, dreeg
`gd-save`):

- **Key seed:** `key = first_u32_le ^ 0x55555555`.
- **Table:** `k = key; for i in 0..256 { k = k.rotate_right(1)
  .wrapping_mul(39916801); table[i] = k }`.
- **Byte:** `plain = c ^ (key as u8); key ^= table[c]` — the
  *ciphertext* byte indexes the table, and the key updates **after**
  the XOR. (gdlc's single-byte `read_byte` updates first; that is a
  gdlc quirk used only by `read_bool`, not the format.)
- **u32:** `plain = c ^ key`, then `key ^= table[b]` for each of the
  four little-endian ciphertext bytes. `f32` is a u32 read
  reinterpreted. String: decrypted u32 length + decrypted bytes (wide
  strings: length in u16 units, ×2 bytes).
- **Block:** `id = read_u32()` (advances the key); `len = raw_u32 ^
  key` **without** advancing; body; end checksum = raw u32 that must
  equal the current key (gdlc checks `next_int() == 0`).
- **Consequence (binding in ARCHITECTURE.md):** every prior byte
  feeds the key, so a byte-level splice is impossible. Writes are
  full re-encode under the lossless-model gate.
- **Opaque blocks are not re-keyable** (found 2026-09-03 while
  building the parser): a u32 is XORed against the whole 32-bit key
  at once while a byte is XORed against the key's low byte *after*
  the previous byte advanced it, so the same ciphertext decodes to
  different "plaintext" depending on the field layout. A block
  decoded byte-wise re-encodes exactly under the same key and
  **wrongly under any other key**. Hence an edit to block N can only
  be written if every block after N is fully typed; `grimvault-core`
  records the key each opaque block was read under and refuses to
  encode it under a different one. Nested (id 0) blocks and the
  zero markers after a version word do not feed the key.
- Single-byte order verified: XOR then update. gdlc's update-first
  `read_byte` decodes the header's sex/hardcore bytes to values like
  150/223 on real files; XOR-first gives 0/1.

### `player.gdc`

- Header per gdlc `player.rs`: `"GDCX"`, `2`, version `8`, 16-byte
  UID; then blocks 1 (CharacterInfo v5), 2 (bio), 3 (inventory
  v4..=11), 4 (stash v6..=11). Fixture: gdlc `test/v11_player.gdc`.
- Real block sequence (identical in the fixture and all three real
  saves, 2026-09-03): `1 2 3 4 5 6 7 17 8 12 13 14 15 16 10`.
  **Every block is typed** (2026-09-03, `grimvault-core::blocks`,
  one module per id), so the whole file re-encodes from the model
  and an inventory or stash edit — which re-keys every later block —
  is writable. Proof: removing, adding, and moving a sack item, then
  encode → parse, reproduces the edited model exactly on the fixture
  and on all three real saves (`tests/fixture_round_trip.rs`,
  `examples/save_smoke.rs`). A block at a version outside the table
  below falls back to opaque, which makes that file read-only again
  (the re-key rule above). Layouts ported from wr8fdy/yagde (Rust,
  MIT) and cross-checked field by field against nbak/grim-save-parser
  (Rust, MIT); Odie/gd-edit (Clojure) stays eyes-only. Block 8 v8 and
  block 16 v12 are what the current game writes and are unknown to
  both references; they were established here from the fixture and
  the real saves, and every sample closes on its block checksum
  under them.

  | Block | Meaning | Versions typed | Notes |
  |---|---|---|---|
  | 1 | character info | 5 | gdlc |
  | 2 | bio: level, experience, unspent attribute / skill / devotion points, total devotion unlocked, physique, cunning, spirit, health, energy | 8 | yagde `Bio`, grim-save-parser `CharacterBio` agree |
  | 3 | inventory (sacks, equipment) | 4–11 | gdlc |
  | 4 | per-character stash | 6–11 | gdlc |
  | 5 | respawn points: three 16-byte-uid lists (per difficulty) then three current uids | 1 | `RespawnList` in both |
  | 6 | rift gates: three uid lists per difficulty | 1 | `TeleportList` in both |
  | 7 | map markers: three uid lists per difficulty | 1 | `MarkerList` in both |
  | 17 | devotion shrines: six uid lists, unnamed by either reference (plausibly 3 difficulties × 2 states) | 2 | `ShrineList` in both |
  | 8 | skills: skill list (name, level, enabled byte, devotion level, experience, active, two unknown bytes, auto-cast skill / controller), masteries allowed, skill and devotion reclamation points, item skills (name, auto-cast pair, item slot, item), v6+ a counted sub-skill list | 5, 6, 8 | yagde reads the v6 word as a sub-skill list, grim-save-parser as one `u32` — identical while the count is 0, as in every sample. **v8** (current game): one extra byte per skill after `enabled`, 1 on the `itemskillsgdx3/potionmodifiers/healthpotion_*` entries and 0 elsewhere, meaning unknown; v7 has no sample |
  | 12 | lore notes collected, as record paths | 1 | |
  | 13 | factions: one leading word (both call it `faction`) then per faction `modified`, `unlocked` bytes, value, positive and negative boost | 5 | |
  | 14 | UI: three unknown scalars (byte, word, byte), five unknown string / string / byte entries, hotbar sets, camera distance. v4 / v5 / v6: one set of 36 / 46 / 47 slots; v7: set count, slots per set, an id per set. Slot types 0 skill (skill, is-item-skill byte, item, equip location), 4 item (item, two bitmaps, wide label), 2 / 3 health / energy potion, `0xFFFFFFFF` empty | 4–7 | slot meanings from grim-save-parser |
  | 15 | tutorial pages shown | 1 | |
  | 16 | play stats: counters, greatest damage, per-difficulty monster records, champion / hero kills, crafting and exploration counters, nemesis kills per difficulty; v9+ survival-mode quartet; v11+ skill map, Shattered Realm souls / essence, difficulty-skip byte; two trailing unknown words | 7, 9, 11, 12 | **v12** (current game): two more words before the trailing pair (28 / 0 in the fixture, 0 / 0 in the real saves); v8 and v10 have no sample |
  | 10 | quest trigger tokens: three string lists per difficulty | 2 | yagde calls it `Crucible` and its reader drops the tokens; grim-save-parser (`TriggerTokens`) keeps them. Written only when the header data version ≥ 7 |

- Block 3 (inventory) has a `flag` byte (1 in every file) and a
  never-entered state with no sacks; block 1's `difficulty` byte
  reads 50/0/64/16 across four files, so it is not a difficulty
  index despite gdlc's name.
- Sack grids are **not** in the save. Main bag 12 × 8 cells,
  every additional bag 8 × 8: `records/game/gameengine.dbr` gives
  `UICharWindowInventorySack0DimsX/Y` = 384 × 256 px and
  `UICharWindowInventorySack1DimsX/Y` = 256 × 256 px (the same sizes
  as `records/ui/character/characterinventory/inventory_grid0.dbr` /
  `inventory_grid1.dbr` `inventoryXSize/YSize`) at 32 px per cell
  (the item-bitmap footprint rule). Corroborated 2026-09-03 by item
  extents: the fixture's packed sacks reach exactly x+w = 12 / 8 and
  y+h = 8, the real saves stay within. `transfer::SackDimensions`.
- Location: `save/main/_<Name>/player.gdc`; the game's own rotation
  is `player.g00` / `player.g01` (never ours to touch).

### `transfer.gst` (shared stash), `formulas.gst`

- `transfer.gst` per gdlc `stash.rs`: `2`, block `18`, stash version
  5..=11; gd-edit `stash.clj` corroborates Block 18. Stash items
  append `X, Y` floats.
- Game rotation: `transfer.t00` – `.t09` (never ours to touch).
- `formulas.gst` (blueprints) is **not obfuscated at all**: plaintext
  Titan-Quest-style `begin_block` 0xB01DFACE / `end_block`
  0xDEADC0DE with length-prefixed keys (`formulasVersion` = 3,
  `numEntries`, `expansionStatus`, repeated `itemName` /
  `formulaRead`). Detected and refused with a matchable error by the
  `.gst` parser; the engine's key/value reader would parse it.
  Read-only per ARCHITECTURE.md.

### `reagents.gst` (component and crafting-material storage)

The account-wide storage the game shows as its Components and
Crafting Materials tabs is one file. No reference (gdlc, yagde,
grim-save-parser, gd-edit) types it; the layout below was established
2026-09-03 with `grimvault-core`'s `Decoder` on the user's own file
(62 entries, 4,191 bytes) and closes on every checksum;
`gst::ReagentStorage`, writable (ARCHITECTURE.md "Source of truth").

- Leading word `1`, then block **20** version **1**, then — in
  block 18's shape — a static zero marker that does not feed the key.
- Header words after the marker, in order: a `u32` **0**, modelled as
  the mod name string (block 18's slot; an empty string encodes as
  exactly one zero word, so the two readings are byte-identical on
  the base game and the string reading is the one that matches
  blocks 18 and 19); then the **entry count** (62). There is no
  expansion-status byte: the count decodes as a `u32` directly after
  the zero word and the 62 nested blocks then parse cleanly, which a
  stray byte would break.
- Then one nested id-0 block per entry: `string` record path (u32
  length + bytes) and `u32` count, e.g.
  `records/items/crafting/materials/craft_aetherialmissive.dbr` × 8
  (59-character path, 67-byte body), `records/items/materia/
  compb_unholyinscription.dbr` × 20, `records/items/questitems/
  scrapmetal.dbr` × 80. An entry keeps nothing but record and count —
  no seed, no affixes.
- Which records the game admits (surveyed across all three database
  layers, `reagents::ReagentKind`): exactly the 125 records whose
  `craftingMaterial` flag is set — all 107 `ItemRelic` components
  (`records/items/materia/`) and 18 `QuestItem`s (the 15 under
  `records/items/crafting/materials/`, Scrap and Dynamite under
  `records/items/questitems/`, one more under `materia/`). The
  Components tab is the `ItemRelic`s; every other flagged record is a
  crafting material. An entry whose record cannot be classified is
  shown under Crafting Materials and may be vaulted but not merged
  into.
- Any other version of block 20 stays opaque, which keeps the file
  read-only (the re-key rule above).

### `transmutes.gst` (illusion collection)

Block **19** version **2**, typed read-only 2026-09-03 from the user's
file (`gst::Illusions`); ARCHITECTURE.md keeps the file read-only.

- Leading word `1`; version; static zero marker; mod name string
  (empty); expansion-status byte (7, as block 18's); `u32` slot count
  (9).
- Then one nested id-0 block per equipment slot: `u32` slot id, `u32`
  record count, then that many record-path strings. Slot ids observed
  1, 3, 4, 5, 7, 8, 9, 14, 15 holding head, torso, legs, feet, hands,
  off-hand (foci and shields), weapons (one nested list for every
  weapon class), shoulders, and medals respectively; the id ↔ slot
  mapping beyond that observation is unverified.

### Item serialization (inside saves and stash)

Confirmed by gd-edit's `Item` spec and gdlc `inventory_item.rs`:
`baseName, prefixName, suffixName, modifierName, transmuteName,
seed (i32), relicName, relicBonus, relicSeed, augmentName, unknown
(i32, gdlc asserts 0), augmentSeed, relicCompletionLevel ("var1" /
materia combines), stackCount`. **Versioned additions:**
`ascendantRecord` + `ascendantRecord2h` (strings) between
`augmentSeed` and `relicCompletionLevel` when inventory version ≥ 8;
`seedRerolls` after `stackCount` (≥ 8); `affixRerolls` (≥ 11). IAGD's
export format carries the same fields
(`AscendantAffixNameRecord`, `RerollsUsed`, `AffixRerollsUsed`).

### `.tex` (item bitmaps)

- GD `TEX\x02` places the `DDSR` header at offset 12 with **no pad
  byte** (TQ's Atlantis-era `TEX\x02` has one); the engine's
  `tex::locate_dds` tries both. GD item bitmaps are `DDPF_RGB` 32-bit
  with zero masks; all 5,134 `.tex` headers in Items.arc read cleanly
  (2026-09-03). Footprint = pixels ÷ 32 per axis, as in TQ. Pixel
  decode was not exercised on GD textures, and DXT-compressed
  textures still report `UnsupportedFormat`; IAGD `DDSImageReader`
  and dreeg `gd-db` (`image_dds`) are the references beyond headers.
- `bitmap` record values carry an `items/` prefix naming the archive;
  Items.arc entry names omit it.

## Rust prior art (2026-09-03)

- crates.io: nothing for Grim Dawn or Titan Quest ARZ/ARC/save.
- GitHub: gdlc (MIT), lib-gddb (GPL-3.0), dreeg (EPL-1.0 / MIT
  ambiguity), yagde (MIT save editor), grim-save-parser (MIT),
  grim-dawn-item-helper (no license). For TQ: only
  Tamschi/serde_titan-quest (unlicensed, dead since 2021) and
  tq-univault.

## Verified vs unverified

**Verified from primary sources:** gdlc license, modules, formats,
and decoder; lib-gddb GPL-3.0 and ARZ/ARC-only scope; marius00/iagd
MIT and the absence of a C# save decoder; GD Stash closed source; GD
ARZ/ARC header and entry layouts, LZ4-block compression, the extra
decompressed-size field; TQ zlib + 2-byte ARC skip; the full XOR
scheme and block/checksum semantics; GD item field order including
the v8/v11 additions; no relevant crates.io crates.

**Unverified:** whether the 8-byte ARZ entry trailer is a FILETIME or
padding (preserved verbatim either way); whether block 20's zero word
is a mod name or a bare `u32` (identical bytes on the base game; only
a mod's `reagents.gst` would tell); whether the game writes a
zero-count entry or drops it (this app drops it); block 19's slot-id
mapping; whether IAGD ever shipped
`GDCryptoDataBuffer.cs` itself (inferred from GDParser's derived
file); GD Stash's Nexus/ModDB permission text (HTTP 403); whether the "multiple count
groups" string-table loop in gdlc/lib-gddb reflects real files or
defensive coding; the fields the block-typing pass could not name
(the v8 per-skill byte, the two v12 stats words, the six shrine
lists, `Factions::faction`); skills v7 and stats v8/v10 layouts (no
sample); the `Sex` mapping (0 female / 1 male, inferred from
character names only).
