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
- **GD Stash** (mamba, Java, closed source, no published license) —
  **eyes-only** since 2026-09-06 (user decision; ARCHITECTURE.md
  "Parser provenance"). The user's copy is
  `/Volumes/scott-games/GDStash_v190a` (v1.90a; `GDStash.jar`, 656
  unobfuscated classes under `org.gdstash`). It decompiles cleanly
  with CFR (`brew install cfr-decompiler`, binary `cfr-decompiler
  GDStash.jar --outputdir <scratchpad>`) — decompile into the
  session scratchpad only, never into this repository — and the jar
  embeds the author's own notes as plain text (`_info.txt` …
  `_info5_gameengine.txt`, `info5_dmgtypes.txt`,
  `org/gdstash/file/GDByteBuffer.txt`) plus HTML user docs under
  `doc/`. Read it for format facts, verify them against real files,
  never transcribe code or its hard-coded data tables. What it has
  told us so far is under "GD Stash (eyes-only) findings" below.
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
- Location: `save/main/_<Name>/player.gdc` (main campaign) or
  `save/user/_<Name>/player.gdc` (custom games — every mod shares the
  folder, and the header carries no mod name; confirmed 2026-09-06 on
  the user's LootAscension character, which parses fully typed and
  round-trips like a main one); the game's own rotation is
  `player.g00`… (`.g06` observed; never ours to touch). A mod's own
  stash and storage live beside them under `save/<Mod>/` with the mod
  name inside each `.gst`.

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

## GD Stash (eyes-only) findings — 2026-09-06

Facts read from the decompiled v1.90a sources and cross-checked
against the user's files where a file exists. Class names are given
so the next reader can find the spot; nothing below is a transcription.

- **Package map:** `org.gdstash.file` — ARC/ARZ/DDS/TEX readers and
  `GDReader` / `GDWriter` (the rolling-XOR codec; same scheme as
  ours). `org.gdstash.character` — one class per `player.gdc` block:
  `GDCharHeader`, `GDCharBio` (2), `GDCharInventory` +
  `GDCharEquippedContainer` + `GDCharInventorySack` (3),
  `GDCharStash` (4), `GDCharRespawnList` (5), `GDCharTeleportList`
  (6), `GDCharMarkerList` (7), `GDCharSkillList` / `GDCharSkill` (8),
  `GDCharCrucible` (10), `GDCharNoteList` (12), `GDCharFactionList`
  (13), `GDCharUISettings` (14), `GDCharTutorialList` (15),
  `GDCharStats` (16), `GDCharShrineList` (17). `org.gdstash.item` —
  `GDStash` / `GDStashPage` (block 18), `GDTransmute` /
  `GDTransmuteType` (19), `GDReagent` / `GDReagentItem` (20),
  `GDItem`, `GDRandomUniform`. `org.gdstash.formula` —
  `formulas.gst`. `org.gdstash.quest` — the per-character quest
  files. `org.gdstash.db` — its Derby mirror of the record database
  (`DBItem`, `DBAffix`, `DBEnginePlayer`, `DBEngineLevel`,
  `DBSkillTree`, `ItemClass`, `ItemSlots`, `DBStashItem` with the
  `.gds` / `.ias` readers). `org.gdstash.description` — item stat
  text composition. It does **not** read `playmenu.cpn`.
- **Stash file family.** Every shared file exists per expansion
  level and per mode, distinguished by suffix: `transfer.gst` /
  `.gsh` (softcore / hardcore, all expansions; UI labels "Softcore
  (.gst)", "Hardcore (.gsh)"), `.dst` / `.dsh` ("SC FG", Forgotten
  Gods level), `.cst` / `.csh` ("SC AoM"), `.bst` / `.bsh` ("SC
  Vanilla"); `formulas.*`, `transmutes.*`, `reagents.*` take the
  same suffixes, and the game's rotation follows (`.t00` for `.gst`,
  `.h00` for `.gsh`, `.dt` / `.dh`, `.ct` / `.ch`, `.bt` / `.bh`).
  Confirmed on disk 2026-09-06: the user's save root holds
  `transfer.dst`, `formulas.dst`, `transmutes.dst` beside the `.gst`
  set. **This app opens only the `.gst` files today.** The header's
  `expansionStatus` byte is read as 1 = Ashes of Malmouth, 3 =
  Forgotten Gods, 7 = Fangs of Asterkarn (by the look of it a mask:
  bit 0 AoM, bit 1 FG, bit 2 FoA); on block 18 only `== 1` is tested.
- **Block 19 (`transmutes.gst`) slot ids — now corroborated:** 1
  head, 3 torso, 4 legs, 5 feet, 7 hands, 8 off-hand (foci *and*
  shields), 9 weapon (every weapon class, one- and two-handed), 14
  shoulders, 15 medal — `GDTransmute` constants, and
  `ItemClass.getTransmuteType` maps a record's `Class` onto them.
  Matches the nine ids observed in the user's file exactly. Its
  "add all illusions" treats two items as the same illusion when
  mesh, base / bump / glow texture, and shader all match, and skips
  enemy-only items and `medal_visible.msh`. It writes the file with
  the version and `expansionStatus` it read.
- **`formulas.gst`:** read and written by `GDFormulaList` in the
  plaintext `begin_block` / `end_block` shape recorded above
  (`formulasVersion` 3, `numEntries`, `expansionStatus`, then
  `itemName` + `formulaRead` per entry). "Enable all blueprints"
  appends every blueprint record not already listed with
  `formulaRead = 1`.
- **`reagents.gst`:** block 20 v1, `(record, count)` entries, as
  ours. Entries it cannot resolve are carried through unchanged
  (`removedItems`); it does not settle whether the game keeps a
  zero-count entry.
- **`player.gdc` names for fields we left unnamed** (bytes
  identical; only the labels differ):
  - Block 8 skills, v7+: `name, level, enabled, locked,
    devotionLevel, experience, subLevel (u32), active (byte),
    transition (byte), autoCastSkill, autoCastController` — so our
    `unknown_u8_v8` is its `locked`, our "active" word its
    `subLevel`, our two trailing bytes its `active` / `transition`.
    (v ≤ 6: `name, level, enabled, devotionLevel, experience,
    active (u32), locked, transition, autoCast pair`.)
  - Block 16 stats, v12: it reads the two v7+ trailing words first
    (`unknown1`, `unknown2`) and then `ascendantBossMonstersKilled`,
    `hiddenChestsOpened`; ours reads the v12 pair first, then the
    trailing pair. Same four words — which pair is the new one is
    still open (the fixture's `28` sits in its `unknown1` and our
    `unknown_u32_v12_a`).
  - Block 17 shrines: the six lists are, per difficulty *d* (0
    normal, 1 elite, 2 ultimate), list `2d` = restored, list `2d+1`
    = discovered. It also carries hard-coded UID → name tables for
    shrines and rift gates (block 6); we would derive those from the
    game's level files, not copy them.
  - Block 13 factions: the leading word is unnamed there too
    (`faction`); per faction `modified`, `unlocked` bytes and
    `value`, `positiveBoost`, `negativeBoost` floats, as ours.
  - Block 14 UI: `equipmentSelection` (byte), `skillWindowSelection`
    (u32), `skillSettingValid` (byte), five × (`primarySkill`,
    `secondarySkill`, `skillActive` byte), the skill sets,
    `cameraDistance`.
  - Block 3 equipment order: 0 head, 1 amulet, 2 chest, 3 legs, 4
    feet, 5 hands, 6 ring left, 7 ring right, 8 belt, 9 shoulders,
    10 medal, 11 artifact (relic); `useAlternate` byte before the
    twelve, then `alternate1` byte + main hand / off hand, then
    `alternate2` byte + the second pair. Item field `unknown (i32,
    gdlc asserts 0)` between `augmentName` and `augmentSeed` is its
    `enchantmentLevel` — the **augment level**.
  - Slot rule for equipping: `ItemSlots` is 25 booleans per record
    class (axe / mace / sword / dagger / scepter / spear / staff /
    ranged, one- and two-handed; shield, off-hand; amulet, belt,
    medal, ring; head, shoulders, chest, hands, legs, feet);
    `ItemClass.getClassInt` enumerates the record `Class` values
    (`ArmorProtective_Head` … `WeaponHunting_Ranged2h`,
    `ItemArtifact`, `ItemRelic`, …).
- **Level, XP, and respec math is data-driven** — all from
  `records/creatures/pc/playerlevels.dbr`: `experienceLevelEquation`
  (an expression in `playerLevel`, evaluated with exp4j; a small
  evaluator is needed on our side), `characterModifierPoints` /
  `skillModifierPoints` (attribute and skill points per level;
  `DBEngineLevel` sums them over a level range), `strengthIncrement`
  / `dexterityIncrement` / `intelligenceIncrement` / `lifeIncrement`
  / `manaIncrement` (what one point buys; the attribute buttons
  also move health and energy by them), `characterStrength` /
  `characterDexterity` / `characterIntelligence` / `characterLife` /
  `characterMana` (base values), `maxDevotionPoints`. Mastery reset
  (`GDCharSkillList.refundMastery`): every skill of the mastery's
  `DBSkillTree` is removed from block 8, the levels of the
  non-granted ones summed and returned to block 2's skill points.
- **Seeds:** `GDRandomUniform` is the Park–Miller minimal-standard
  generator (16807, 2³¹ − 1, Schrage's split) used to mint new item
  seeds; its own docs say the seed → stat-roll mapping is unknown to
  it. A fresh seed on copy can be any `i32`.
- **`.gds` (GD Stash export) — a read-only import boundary since
  2026-09-06** (ARCHITECTURE.md "External boundaries";
  `grimvault-core::gds`, never written by this app). Plain
  little-endian, unobfuscated: `u32 version` (v1.90a writes 3; 1
  and 2 are read), `u32 count`, then per item — strings are `u8
  length + UTF-8 bytes`, 0 = absent — `itemID, prefixID, suffixID,
  modifierID, transmuteID, i32 seed, relicID, relicBonusID, i32
  relicSeed, enchantmentID (augment), i32 enchantmentLevel, i32
  enchantmentSeed, [v ≥ 2: ascendantID, ascendant2hID], i32 var1
  (relic completion level), i32 stackCount, [v ≥ 2: i32
  rerollsUsed], [v ≥ 3: i32 affixRerollsUsed], u8 hardcore,
  charname` (the soulbound owner, absent otherwise). The item
  fields are the game's own record in wire order (`Item`), the
  `i32`s being Java's reading of the same four bytes this crate
  keeps as `u32`; `enchantmentLevel` lands in `Item::unknown`,
  `var1` in `relic_completion_level`. `hardcore` is written as 0 or
  1 (GD Stash reads any non-zero as true; this crate refuses other
  values, since one means the entry was not read where it starts).
  Verified 2026-09-06 end to end on the user's own exports — every
  byte consumed, nothing trailing: `gd-stash-export.gds` (version 3,
  3,205 entries; first `records/items/enchants/a07a_enchant.dbr` ×
  40, seed `0x1c4c23f4`, softcore, owner "Zark"; 300 entries with
  seed 0, 392 stacks above 1, 79 soulbound to Zark, every one
  softcore, every one distinct as a whole record; `enchantmentLevel`,
  `var1`, both reroll counts, and both ascendant records are 0 /
  empty throughout) and `reagent-export.gds` (version 3, 589
  entries; the same first augment as a stack of 5; only 149
  distinct by base record + seed, so the same seed recurs with
  different stack counts; 22 entries identical to ones in the
  collection export). Against the layered database (base, `gdx1`,
  `gdx2`, `gdx3`, two mods) every record in both files resolves.
  **GD Stash's own duplicate rule** (`DBStashItem.storeItem` →
  `isStored`): a non-stackable item is "already in the stash" when
  every field — base, prefix, suffix, modifier, transmute, seed,
  relic, relic bonus, relic seed, augment, augment level, augment
  seed, both ascendant records, `var1`, stack count, hardcore, and
  owner — matches; a stackable one (`isStackable`: a complete
  component, or the record's own stackable flag) is never a
  duplicate and instead merges its count into the row with the same
  base, `var1`, hardcore, and owner, so importing its own export
  twice doubles every stack there. This crate adopts the
  non-stackable identity for every entry — stack count included —
  because it is the only rule under which the reagent export's 589
  entries survive (149 would under any count-free key) and a
  re-import adds nothing. The `.ias` (Item Assistant) reader beside
  it takes versions 1–7 with the same string encoding, `var1` as
  `i16`, then `hardcore`, an expansion byte, a mod name, and
  version-gated extras; recorded, not implemented.

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
MIT and the absence of a C# save decoder; GD Stash closed source
(its jar and docs carry no license text; decompiles cleanly, read
eyes-only); block 19's slot ids (GD Stash's constants agree with the
nine observed); GD
ARZ/ARC header and entry layouts, LZ4-block compression, the extra
decompressed-size field; TQ zlib + 2-byte ARC skip; the full XOR
scheme and block/checksum semantics; GD item field order including
the v8/v11 additions; the `.gds` version-3 layout end to end on both
of the user's exports; no relevant crates.io crates.

**Unverified:** whether the 8-byte ARZ entry trailer is a FILETIME or
padding (preserved verbatim either way); whether block 20's zero word
is a mod name or a bare `u32` (identical bytes on the base game; only
a mod's `reagents.gst` would tell); whether the game writes a
zero-count entry or drops it (this app drops it; GD Stash does not
say); which of block 16's two word pairs is the v12 addition (GD
Stash and this crate name them the other way round); the shrine
lists' restored / discovered split (GD Stash's reading, not yet seen
in-game); whether IAGD ever shipped
`GDCryptoDataBuffer.cs` itself (inferred from GDParser's derived
file); GD Stash's Nexus/ModDB permission text (HTTP 403); whether the "multiple count
groups" string-table loop in gdlc/lib-gddb reflects real files or
defensive coding; the fields the block-typing pass could not name
(the v8 per-skill byte, the two v12 stats words, the six shrine
lists, `Factions::faction`); skills v7 and stats v8/v10 layouts (no
sample); the `.gds` version-1 and version-2 layouts (read by GD
Stash's version gates; no sample — v1.90a writes only 3); the `Sex`
mapping (0 female / 1 male, inferred from character names only).
