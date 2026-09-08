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
- **Two ways to read one:** whole (`ArcFile`, as `Items.arc` and
  `Text_EN.arc` are read), or by entry (`ArcHeader` → the table
  region → `ArcIndex`, whose `locate` yields the absolute byte ranges
  of an entry's parts and whose `assemble` rebuilds the file from
  them). The shell reads `UI.arc` the second way for the eleven tile
  symbols (2026-09-06): the parts lie between the 28-byte header and
  the table offset, the tables run from the table offset to the end
  of the file.

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
  `.gst` parser; `grimvault-core::formulas` reads and writes it
  (2026-09-06, writable adds-only per ARCHITECTURE.md). Layout
  established byte by byte on the user's files: every key is a `u32`
  length + ASCII (`begin_block` 11, `formulasVersion` 15, …), the two
  markers are the raw little-endian words `CE FA 1D B0` /
  `DE C0 AD DE`, the version word is 3, the count a `u32`,
  `expansionStatus` **one byte** (7 in the `.gst` files, 3 in
  `formulas.dst`), each entry `itemName` + `u32`-length record path
  then `formulaRead` + `u32`, and nothing follows `end_block`. The
  read flag is 0 or 1 in every file (0 = the in-game "new" badge on
  a blueprint not yet viewed); the parser types it as a boolean and
  refuses any other value, refuses version 2 (GD Stash accepts one
  without the expansion byte; no sample), and refuses trailing bytes,
  so an accepted file re-encodes byte-for-byte — verified on
  `formulas.gst` (217 entries, 22,577 bytes), `formulas.dst` (177),
  and `LootAscension/formulas.gst` (288). The game **appends** a
  newly learned blueprint at the end with the flag at 0: the
  LootAscension file and its `.bak` twin differed only in the count
  and one trailing entry (`craft_armord308.dbr`, flag 0), which is
  what this app's add does. A blueprint record is one whose `Class`
  (and record-table type) is `ItemArtifactFormula` — every one of the
  682 entries across the three files, and 991 such records in the
  layered database (base + gdx1–3 + mods); its name is the
  `description` tag, its icon `artifactFormulaBitmapName`, its
  `itemClassification` the rarity.

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

Block **19** version **2**, typed 2026-09-03 from the user's file
(`gst::Illusions`); writable adds-only since 2026-09-06
(ARCHITECTURE.md "Source of truth"; the rule is
`grimvault-core::illusion`).

- Leading word `1`; version; static zero marker; mod name string
  (empty, or the mod's name in `save/<Mod>/transmutes.gst`);
  expansion-status byte (7, as block 18's); `u32` slot count (9).
- Then one nested id-0 block per equipment slot: `u32` slot id, `u32`
  record count, then that many record-path strings. Slot ids 1, 3,
  4, 5, 7, 8, 9, 14, 15 hold head, torso, legs, feet, hands, off-hand
  (foci and shields), weapons (every weapon class in one list),
  shoulders, and medals. **Verified record by record 2026-09-06** on
  the user's main collection (592 records) and LootAscension's
  (2,353): every record's `Class` maps to the id of the list it sits
  in — `ArmorProtective_Head` 1, `_Chest` 3, `_Legs` 4, `_Feet` 5,
  `_Hands` 7, `WeaponArmor_Offhand` and `WeaponArmor_Shield` 8, every
  `WeaponMelee_*` / `WeaponHunting_*` / `WeaponMagical_Staff` class 9
  (19 weapon classes seen, `Spear2h` included), `_Shoulders` 14,
  `ArmorJewelry_Medal` 15 — with zero disagreements
  (`illusion::audit`, run by `--check`). Belts, rings, and amulets
  never appear. A new illusion is appended to its category's list;
  the game rewrites the whole file with a fresh seed on its own
  saves (the `.bak` twin of the main file differs only in the seed),
  so the seed carries no meaning.

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

### Stack size and seed identity — 2026-09-07

What makes two items of one record the same item, for the bulk moves
of `grimvault-core::bulk` (FEATURES.md 13): the roll seed, but only
for the classes the game never stacks. Read from the templates and
the engine records on the user's install (`database/templates.arc`,
`records/game/gameengine.dbr`, `records/game/gameiteminfo.dbr`) and
checked against the user's two GD Stash exports:

- `templatebase/itembase.tpl` declares `maxStackSize` (int, default
  0, "Overrides gameengine default"): a per-record override of the
  engine's per-class stacking. Across every layer only the potions
  (`OneShot_PotionHealth` / `_PotionMana`, 100) and five `QuestItem`
  records (50) set it; components, augments, and every other record
  leave it 0 and take the engine default. `gamedata::ItemInfo`
  carries it as `max_stack_size`.
- The engine defaults: `gameengine.dbr` `potionStackLimit`,
  `questItemStackLimit`, `scrollStackLimit` (all 100) and
  `gameiteminfo.dbr` `itemMaxStackSize` (1000). Which classes those
  apply to is engine code, not data, so the app decides by class
  through the type bucket: the equipment buckets and relics
  (`ItemArtifact`) are one-per-instance and identified by seed; every
  other bucket — components, augments, blueprints, transmuters,
  consumables, quest items, notes, and the unmapped rest — is a
  stack, as is any record with `maxStackSize` above one
  (`bulk::Identity::of`).
- Corroboration (3,794 exported entries): no `Armor*`, `Weapon*`, or
  `ItemArtifact` entry carries a stack above 1, while `ItemRelic`
  reaches 1000, `QuestItem` 1718, `ItemEnchantment` 72,
  `ItemFactionBooster` 80, `OneShot_Scroll` 286, `ItemNote` 4, and
  `ItemArtifactFormula` 2 — the stacks are exactly the non-equipment
  classes. A zero seed appears on 14 equipment entries (GD Stash
  creations) and on every blueprint, so zero is read as "never
  rolled" and identifies nothing.

Not verified: whether the game itself ever merges two equipment items
of one record and seed (the duplicate rule only ever leaves an item
in place, so a wrong call costs a manual drag, never an item).

### Item facets (monster infrequent, double rare, ascension)

Established 2026-09-06 on the user's install (base + `gdx1`–`gdx3` +
the two mods) and the 794 items of the user's transfer stashes and
five characters; `grimvault-core::facets`, gated by
`tests/facets_real.rs` when `$GRIMVAULT_GAME_DIR` / `$GRIMVAULT_SAVE_DIR`
are set. GD Stash (eyes-only) pointed at the records; every fact
below was read from the real database.

- **The game's own indicators.** `records/game/gameiteminfo.dbr`
  (template `gameiteminfo.tpl`, no `Class`) names eleven inventory
  tile symbols: `monsterInfrequentSymbol`, `doubleRareSymbol`,
  `doubleRareMonsterInfrequentSymbol`, `commonAscendedSymbol`,
  `magicalAscendedSymbol`, `rareAscendedSymbol`,
  `doubleRareAscendedSymbol`, `monsterDoubleRareAscendedSymbol`,
  `epicAscendedSymbol`, `legendaryAscendedSymbol`, and
  `awakenedItemSymbol`, each a `ui/character/item_*.tex` path
  (`item_monsterinfrequent.tex`, `item_doublerare.tex`,
  `item_doubleraremonsterinfrequent.tex`, `item_ascended_common.tex`,
  `item_ascended_magic.tex`, `item_ascended_rare.tex`,
  `item_ascended_doublerare.tex`, `item_ascended_doubleraremi.tex`,
  `item_ascended_epic.tex`, `item_ascended_legendary.tex`,
  `item_awakened.tex`). All eleven live in the **base**
  `resources/UI.arc` (entry names without the `ui/` prefix, the same
  convention as `items/` for `Items.arc`), 32 × 32, uncompressed
  32-bit, 4,236 bytes each, decodable by `tex::decode`; the
  expansion and mod `UI.arc` files hold none. The same record also
  carries the rarity colours (`rareItemColor` etc.), the loot beams,
  and `doubleRareItemColor` / `monsterDoubleRareItemColor` (both the
  rare green) for reference. The shell reads `UI.arc` **by entry**
  (`ArcIndex` + ranged reads): the base archive is 243 MB, `gdx3`'s
  125 MB, so reading them whole for 44 KB of textures was declined.
- **Monster infrequent.** No record variable names the notion. A
  monster infrequent's base record has `itemClassification = Rare`
  and an equipment `Class` (armor, jewellery, belt, weapon, shield,
  off-hand); the skill modifiers on many of them point at
  `records/skills/itemskills*/skillmodifiers/monsterinfrequents/` (or
  `.../mi/` for `gdx2`), and their `FileDescription` is the monster
  type ("Wendigo Ancient", "Swamp Golem"). **Faction-vendor gear**
  (`records/items/faction/`, 481 `Rare` records) shares the
  classification but carries `soulbound = true` (598 `faction/`
  records carry the flag, 588 of them `true`; the ten `false` are the
  old unsuffixed `f00x_*` faction weapons and shields) and
  `itemStyleTag = tagStyleFactionTier2`; no `gear*/` record is
  soulbound (one `gearaccessories/` record carries the flag, `false`). `factionSource` exists only on the 338 `ItemEnchantment`
  augments. `Rare` components (`materia/compb_*`, 28), augments
  (151), relics (`gearrelic/b*`, 21), boosters and blueprints are not
  equipment. **Rule adopted:** `Rare` classification ∧ equipment
  class ∧ not `soulbound` ⇒ monster infrequent. Assumption: the game
  excludes faction gear the same way (the 1.2 notes say the icon
  "separates monster infrequents from standard Rare items"; the
  engine's test is not readable from the data). The six quest-reward
  rares under `records/storyelements/rewards/` (Slith ring and
  necklace at three levels) and the ten unbound `f00x` faction
  weapons would be marked under this rule.
- **Double rare.** Affix records (`LootRandomizer`) carry
  `itemClassification` — `Magical` (3,281), `Rare` (3,418, e.g.
  `lootaffixes/suffix/b_ar033_ar_f.dbr` "Scorched Ends",
  `lootRandomizerCost` 29,808), `Epic` (22), `Broken` (5). An item is
  double rare when its prefix **and** suffix records are `Rare`. A
  `Rare` base with two rare affixes is both facets at once (the
  game's `doubleRareMonsterInfrequentSymbol`). Ascendant affixes
  (`records/items/lootaffixes/ascended/`, `lootRandomizerCost`
  12,000, no `itemClassification`, no `lootRandomizerName`) never
  count.
- **Ascension (Fangs of Asterkarn).**
  `records/ui/itemascension/itemascension_table.dbr` (template
  `ingameui/itemascensionwindow.tpl`) names one recipe per base
  rarity — `commonRecipe`, `rareRecipe`, `epicRecipe`,
  `legendaryRecipe` → `records/items/crafting/blueprints/ascension/
  craft_ascended_{common,rare,epic,legendary}.dbr` (`Class`
  `ItemAscensionFormula`; `creationCost` 100,000 / 250,000 / 150,000
  / 250,000; `affixWeight` / `masteryWeight` 400 / 600 or 600 / 400;
  five reagents). There is **no magical recipe** — a Common base with
  magical affixes ascends through the common one — and no item-level
  opt-out: `allowAscension` (609 records) is a
  `LootItemTable_DynWeight` flag for pre-ascended drops. Each recipe
  lists affix and mastery tables per equipment category —
  `{accessory,armor,offhand,oneHandMelee,oneHandRanged,shield,
  twoHandMelee,twoHandRanged}Tables{Affix,Mastery}` — all sixteen
  populated in all four recipes; belts are accessories, the six
  armor slots are armor. **Rule adopted:** an item with
  `ascendantRecord` or `ascendantRecord2h` set is *ascended*;
  otherwise it is *eligible* when the table has a recipe for its base
  rarity whose `<category>TablesAffix` is non-empty; the table absent
  means no ascension in this install. **Symbol rule (corrected
  2026-09-06 from the user's game):** the game marks *eligible*
  items, not only ascended ones — the user's LootAscension stash tab 8
  holds 23 plain epics (`c1xx`–`c3xx` bases, several from
  `records/items/upgraded/`), none with an ascendant affix, and every
  one carries the ascension mark in-game — so this app shows the
  displayed rarity's `*AscendedSymbol` for eligible and ascended items
  alike, while the monster-infrequent and double-rare marks outrank
  it (the user's rares keep their monster-infrequent mark). The
  `magicalAscendedSymbol` fits this reading: a common base with a
  magical affix is eligible through the common recipe and displays as
  Magical. Unverified: whether the game marks eligible commons and
  eligible rares that are not monster infrequents; whether it gates
  ascension on item level; and the symbol of an ascended monster
  infrequent without two rare affixes (no dedicated variable exists;
  this app shows `rareAscendedSymbol`, its displayed rarity).
- **The user's items (2026-09-06):** 794 classified, every record
  resolved: 191 monster infrequents (175 plain, 16 with two rare
  affixes), 42 double rares (26 on common bases), 472 eligible for
  ascension, **no ascended item** — the ascended rules are proven on
  hand-built items only.

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
  Items.arc entry names omit it. **The first segment names the
  archive in general** (2026-09-07, from a survey of every item
  record across the four shipped databases and two mods — 10,007
  records with a bitmap, all in the `Armor*`, `Weapon*`, `Item*`,
  `OneShot_*`, and `QuestItem` tables): 26 bitmaps lie outside
  `Items.arc`. The 20 `gdx2` potion formulas (`OneShot_SkillUnlock`,
  `records/items/crafting/blueprints/potions/potions_modifier_a3*.dbr`)
  point at `ui/cauldron/*.tex` in `UI.arc`; **Lokarr's set** is
  hidden as `records/storyelements/signs/sign{f,h,s,t}.dbr` (boots,
  head, shoulders, chest — the letters are the slots; `signa`–`signe`
  and `signg` do not exist) with its icons at
  `level art/buildings/signs/sign_?01a_dif.tex` in **`gdx1`'s**
  `Level Art.arc` (64 × 64 for three, 64 × 96 for the coat: 2 × 2
  and 2 × 3); the Endless Dungeon test item `z001_test.dbr` and Iron
  Bits (`moneyobject01.dbr`) name `system/textures/*.tex` in
  `System.arc`. `Level Art.arc` is 0.85 GB in the base game and
  1.6 GB in `gdx3`, so the shell reads these by entry like the tile
  symbols (`GameData::foreign_bitmaps` names them, the loader fetches
  them from the archive each path names in every layer, mods first,
  and hands the bytes to `GameData::with_bitmaps`). Before this an
  item whose bitmap was elsewhere had no footprint, and a stash tab
  holding one refused every placement ("item 22 … has no known
  footprint" on the user's LootAscension stash, 2026-09-07).

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
  `formulaRead = 1`; its blueprint query is class
  `ItemArtifactFormula` less a hard-coded list of test / random /
  mod-path exclusions that this app does not carry (the database
  survey found none needing it: every record of that class in the
  user's install resolves to a named, iconed blueprint).
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
    `enchantmentLevel` — the **augment level**. **Empty slot shape
    (verified 2026-09-08** on the gdlc fixture and five real
    characters, v4 and v11): `attached` 0, `baseName` empty,
    `stackCount` 1 — never 0 — and the other fields are whatever the
    slot last held: a slot the game emptied keeps its last occupant's
    seed, affixes, component and augment (a two-hander's off-hand slot
    is a full ghost of the weapon with only `baseName` blanked; a
    never-used slot has seed 0 and every string empty). The game reads
    emptiness from `baseName` alone. This app writes the never-used
    form (`EquippedItem::empty`) when it takes gear off; `useAlternate`
    was 0 and `alternate2` 1 on every file read.
  - Slot rule for equipping: `ItemSlots` is 25 booleans per record
    class (axe / mace / sword / dagger / scepter / spear / staff /
    ranged, one- and two-handed; shield, off-hand; amulet, belt,
    medal, ring; head, shoulders, chest, hands, legs, feet);
    `ItemClass.getClassInt` enumerates the record `Class` values
    (`ArmorProtective_Head` … `WeaponHunting_Ranged2h`,
    `ItemArtifact`, `ItemRelic`, …).
- **Level, XP, and respec math is data-driven** — from **two**
  records (`ARZRecord.isPlayerEngine`):
  `records/creatures/pc/playerlevels.dbr` carries
  `experienceLevelEquation` (an expression in `playerLevel`,
  evaluated with exp4j; a small evaluator is needed on our side),
  `characterModifierPoints` / `skillModifierPoints` (attribute and
  skill points per level; `DBEngineLevel` sums them over a level
  range), `strengthIncrement` / `dexterityIncrement` /
  `intelligenceIncrement` / `lifeIncrement` / `manaIncrement` (what
  one point buys), `maxDevotionPoints`, `maxPlayerLevel`; and
  `records/creatures/pc/malepc01.dbr` carries the base values
  `characterStrength` / `characterDexterity` /
  `characterIntelligence` / `characterLife` / `characterMana` and
  the mastery list `skillTree1..N` (the first draft of this note put
  the base values in `playerlevels.dbr`; they are not there — see
  "Respec" below, verified 2026-09-06). Its attribute buttons move
  health by `lifeIncrement` on physique and energy by
  `manaIncrement` on spirit only — an approximation the real saves
  contradict (below). Mastery reset
  (`GDCharSkillList.refundMastery`): every skill of the mastery's
  `DBSkillTree` is removed from block 8, the levels of the
  non-granted ones summed and returned to block 2's skill points;
  `masteriesAllowed` and the header's class tag are left alone, and
  `GDChar.setLevel` recomputes `masteriesAllowed` from the level
  alone (1 above level 1, 2 above level 9).
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

## Respec: attributes and masteries — 2026-09-06

Established for `grimvault-core::respec` (the plain full refunds)
from the user's install — every value below is identical in
`database.arz`, `GDX1.arz`, `GDX2.arz`, and `GDX3.arz` unless a layer
is named — and from the vendored fixture (Laurana, level 100, classes
05 + 06) plus the five real saves (two fresh level-1 characters; Sif,
level 8, class 10; Zark, level 78, classes 03 + 06; the mod Zark,
level 93, classes 03 + 06). GD Stash's rules were the starting point
and were corrected where the saves disagreed.

- **The two records.** `records/creatures/pc/playerlevels.dbr`
  (template `experiencelevelcontrol.tpl`): `strengthIncrement` =
  `dexterityIncrement` = `intelligenceIncrement` = 8;
  `lifeIncrement` 20, `lifeIncrementDexterity` 8,
  `lifeIncrementIntelligence` 12; `manaIncrement` 16;
  `characterModifierPoints` = [1]; `skillModifierPoints` an array
  whose entry *i* is the skill points granted on reaching level
  *i* + 2 — 3 through level 50, 2 through level 90, 1 after;
  `maxDevotionPoints` 50 in the base game, 55 from `gdx1` on;
  `maxPlayerLevel` 85, then 100; `experienceLevelEquation`
  `(((((playerLevel*playerLevel*playerLevel)^1.16)*32)+((playerLevel*playerLevel)*300))*0.1)+36`.
  It has **no** base attribute and **no** mastery list.
  `records/creatures/pc/malepc01.dbr` (template `player.tpl`;
  `femalepc01.dbr` is identical in every field named here):
  `characterStrength` = `characterDexterity` =
  `characterIntelligence` = 50.0, `characterLife` = `characterMana` =
  250.0, and `skillTree1`–`skillTree6` in the base game, up to
  `skillTree8` in `gdx1`, `skillTree9` in `gdx2`, `skillTree10` in
  `gdx3`, each `records/skills/playerclassNN/_classtree_classNN.dbr`
  (the game's own mastery numbering, which the header class tag
  spells). `respec::RespecRules` reads exactly these two and refuses
  when a record or field is missing.
- **Block 2's health and energy are the base pools the points
  bought, not current values.** On every non-fresh save, with
  *p* = (physique − 50) / 8, *c* = (cunning − 50) / 8, *s* =
  (spirit − 50) / 8: health = 250 + 20*p* + 8*c* + 12*s* and energy =
  250 + 16*s*, exactly — Laurana 906 / 50 / 50 → 2390 / 250; Zark
  522 / 50 / 122 → 1538 / 394; the mod Zark 666 / 50 / 146 → 1934 /
  442; Sif 66 / 50 / 50 → 290 / 250. The spirit term is what GD
  Stash's buttons miss. `reset_attributes` derives the spent points
  from the attributes themselves (the total is not a function of
  level: 108 at level 100, 80 at 78, 95 at 93, 7 at 8 — level − 1
  plus quest rewards), refuses unless each attribute is the base
  plus whole 8-point steps and both pools match the formula, then
  puts all five back to the base and adds the points to
  `attribute_points_unspent`.
- **Mastery membership is the tree.** A tree record (type
  `SkillTree`) lists its skills as `skillName1..N` — with duplicates,
  it is a grid of buttons — including the mastery bar itself
  (`_classtraining_classNN.dbr`, template `skill_mastery.tpl`, max
  level 50). A member's `grantedSkills` are the skills the mastery
  hands out for free: in the shipped trees only the four
  `skill_shapeshift.tpl` skills of class 10 (`werewolf1` grants
  `werewolf1_skill01_claws` and `_skill02_charge`, `wereraven1` its
  icicles and ice ring), and they are listed as members too. No tree
  names a record outside its own `records/skills/playerclassNN/`
  folder, so the folder rule and the tree rule remove the same
  skills and refund the same points on every sample: the fixture 27
  skills / 248 points, Sif 6 / 15 (claws and charge at level 6 each
  leave unrefunded; 15 + 6 unspent = 21 = 7 level-ups × 3), Zark
  27 / 198, the mod Zark 27 / 232. `skillsecondary_petmodifier.tpl`
  members (`totem2_petmodifier.dbr`) cost a point like any other.
  `reset_masteries` removes every member and granted skill of every
  tree, adds the levels of the non-granted ones to
  `skill_points_unspent`, and leaves devotions,
  `records/skills/default/*` (8 per character), the 46
  `itemskillsgdx3/potionmodifiers/*` entries, item skills,
  sub-skills, and both reclamation counters as they are.
- **`masteries_allowed` is the level gate, not the chosen count.**
  Observed 0 on both fresh level-1 characters, 1 on Sif (level 8, one
  mastery), 2 on the three two-mastery characters (levels 78, 93,
  100) — consistent with "1 from level 2, 2 from level 10", which is
  how GD Stash recomputes it on a level edit while its mastery
  refund never touches it. The reset therefore keeps it: with the
  mastery skills gone and the gate still 2, the game has room to
  offer both choices again; zeroing it on a level-93 character would
  leave nothing to raise it. Unverified in-game (below).
- **The header's `class_tag`** is `tagSkillClassName` followed by the
  two-digit indices of the chosen masteries (`10` for Sif's one,
  `0306`, `0506` for two) and empty with none; the game derives it
  from block 8, so the mastery reset clears it. GD Stash writes it
  back unchanged.
- **Left alone on purpose:** block 14 hotbar slots that named a
  removed skill, and block 16's skill map — GD Stash leaves both too,
  and the game clears hotbar slots itself when a skill is refunded
  in-game.

## Item stat lines — 2026-09-06

How a database record becomes the stat text the game shows, as
established from the game's own `tags_ui.txt` (2,459 tags in the base
archive, more per expansion) and a census of every variable on the
26,244 `records/items/` records across base, `gdx1`, `gdx2`, `gdx3`
and the two installed mods. Implemented in `grimvault-core::stats`
(`vocabulary` = these facts; `render` = the engine-generic machinery;
`item` = the per-item assembly). Provenance: the machinery is a port
in shape of tq-univault's `stats` module (MIT OR Apache-2.0, same
author; itself a port of `TQVaultAE`'s `ItemProvider`, MIT), rebuilt
on Grim Dawn's tag grammar; the `{%…}` template parser is the vendored
`univault_engine::format`. GD Stash (eyes-only) and IAGD (MIT) were
read for *which attributes exist and how the game phrases them* and
every phrase below was then found in the game's own tags; no line of
either tool's code or text tables is transcribed.

- **Tag grammar.** Every label is a `{%…}` template. Flat damage
  labels take a pre-formatted amount as **text**: `DamageFire={%t0}
  {^E}Fire Damage`, the amount itself coming from
  `DamageSingleFormat={%.0f0}` or `DamageRangeFormat={%.0f0}-{%.0f1}`
  ("12-28 Fire Damage"). Defense and character labels carry the
  number: `DefenseFire={%.0f0}% {^E}Fire Resistance`,
  `tagCharAttribute04={%+.0f0} {^E}Health`. Duration labels have no
  placeholder and the amount is prepended: `DamageDurationFire=
  {^E}Burn Damage`; a label beginning with `%` is a suffix of the
  number (`DamageDurationRunSpeed=% {^E}Slower target Movement` →
  "20% Slower target Movement"). Prefixes and suffixes are their own
  tags: `tagChanceOf={%.1f0}% {^E}{Chance of {^H}`,
  `DamageSingleFormatTime= {^E}over {^H}{%.1f0} {^E}Seconds`,
  `DamageFixedSingleFormatTime= {^E}for {^H}{%.1f0} {^E}Seconds`,
  `ImprovedTimeFormat= {^E}with {^H}{%+.0f0}% {^E}Increased Duration`,
  `GlobalPercentChanceOfAllTag` / `…OneTag` ("…Chance of:" / "…Chance
  for one of the following:"). A line is the concatenation; the text
  archive loader trims every tag and strips the `{^X}` colour codes,
  so the renderer re-joins pieces with single spaces and takes the
  base/bonus distinction from the vocabulary, not from `{^S}`.
- **The `…R` twins** (`DamageFireR={%t0}-{%t1}…`, `DefenseFireR`,
  `tagCharAttribute01R`) are the game's *roll-range* display (the
  possible affix roll under `lootRandomizerJitter`), not damage
  ranges — resistances have them too. Not used.
- **Variable grammar.** `<family stem><Part>`: parts are `Min`, `Max`,
  `Chance`, `Global`, `XOR`, `DurationMin`, `DurationMax`,
  `DurationChance`, `Modifier`, `ModifierChance`, `DurationModifier`,
  `DurationModifierChance`, `MaxResist`, `DrainMin`/`DrainMax` and
  `DamageRatio` (mana burn), and a bare stem for defense, character
  and skill values; a defense `…Duration` is itself a value
  ("Reduction in Burn Duration"). Matching is case-insensitive: the
  database spells `retaliationPercentcurrentLifeGlobal`.
- **Families and their tags** (stem → value tag / modifier tag):
  `offensive<X>` → `Damage<X>` / `DamageModifier<X>`, with
  `offensivePhysical` → `DamageBasePhysical` on weapons and shields
  (white) and `DamagePhysical` ("+N Physical Damage") elsewhere,
  `offensivePierceRatio` → `DamageBasePierceRatio`,
  `offensiveBase<X>` → `tagDamageBase<X>` (`Life` → `…Vitality`; the
  white base damage of caster weapons and shields),
  `offensiveBonusPhysical` → `DamageBonusPhysical`, `offensiveManaBurn`
  → `DamageManaDrain` + `DamageManaBurnRatio`, `offensiveTotalDamage`
  / `offensiveCritDamage` / `offensiveDamageMult` modifier-only →
  `tagDamageModifierTotalDamage` / `…CritDamage` / `…DamageMult`,
  `offensiveSleep` → `tagDamageSleep`, `offensiveFumble` /
  `offensiveProjectileFumble` → `DamageDurationFumble` /
  `…ProjectileFumble`. `offensiveSlow<X>` (duration damage) →
  `DamageDuration<X>` / `DamageDurationModifier<X>`. `retaliation<X>`
  → `Retaliation<X>` / `RetaliationModifier<X>` (`retaliationTotalDamage`
  → `tagRetaliationModifierTotalDamage`); `retaliationSlow<X>` →
  `RetaliationDuration<X>` / `RetaliationDurationModifier<X>`.
  `defensive<X>` → `Defense<X>`, `Defense<X>Modifier`,
  `Defense<X>Duration`, `Defense<X>DurationModifier`,
  `Defense<X>MaxResist`, with `defensiveProtection` →
  `DefenseAbsorptionProtection` ("520 Armor", white on armor) or
  `DefenseAbsorptionProtectionPlus` elsewhere and for
  `defensiveBonusProtection`, `defensiveSlowLifeLeach` /
  `…ManaLeach` → `DefenseLifeLeach` / `DefenseManaLeach`,
  `defensiveTotalSpeedResistance` → `tagTotalSpeedResistance`,
  `defensiveSleep` → `tagDefenseSleep`, `defensiveBlock` →
  `DefenseBlock` plus a "N% Chance to Block" line from
  `defensiveBlockChance` and `tagCharStatsBlockChance`,
  `blockRecoveryTime` → `ShieldBlockRecoveryTime`. `character<X>` →
  `tagChar<X>` / `tagChar<X>Modifier`, except the five attributes,
  which the game names by character-sheet slot:
  `characterDexterity` → `tagCharAttribute01` (Cunning),
  `characterStrength` → `02` (Physique), `characterIntelligence` →
  `03` (Spirit), `characterLife` → `04` (Health), `characterMana` →
  `05` (Energy); also `characterDeflectProjectile` →
  `tagCharDeflectProjectiles`, `characterGlobalReqReduction` →
  `tagCharItemGlobalReduction`, `characterHealIncreasePercent` →
  `tagCharPercentHealIncreaseModifier`. Skill stats on items and skill
  records: `skillCooldownReduction`, `skillManaCostReduction`,
  `skillProjectileSpeedModifier`, `SkillLifeBonus`, `SkillLifePercent`
  … carry their own numbers; `skillManaCost`, `skillActiveManaCost`,
  `skillActiveLifeCost` wrap a noun (`ManaCost`, `ActiveManaCost`,
  `ActiveLifeCost`) in `SkillIntFormat={%d0 %s1}`; `skillCooldownTime`
  / `skillActiveDuration` in `SkillSecondFormat` ("2.0 Second Skill
  Recharge"); `skillTargetRadius` / `projectileExplosionRadius` in
  `SkillDistanceFormat` ("4.0 Meter Target Area");
  `projectileLaunchNumber`, `projectilePiercingChance` (also
  `piercingProjectile`, `projectilePiercing`), `skillTargetNumber`,
  `skillChanceWeight`, `weaponDamagePct` (`SkillWeaponDamageFormat`),
  `petLimit`, `petBurstSpawn`, `spawnObjectsTimeToLive`,
  `damageAbsorption(Percent)`, `cooldownCharges`, `lifeMonitorPercent`
  have direct tags; `onHitActivationChance` prefixes
  `SkillActivationChance`.
- **Durations.** The duration-damage families whose amount is a total
  over the duration (record value × `DurationMin`, "over N Seconds"):
  Bleeding, Fire (Burn), Cold (Frostburn), Lightning (Electrocute),
  Physical (Internal Trauma), Poison, Life (Vitality Decay),
  LifeLeach, ManaLeach. The rest last "for N Seconds" at face value:
  the `Slow{Total,Attack,Run,SpellCast}Speed`,
  `Slow{Offensive,Defensive}{Ability,Reduction}` effects, the
  `offensive…Reduction…` resistance and damage reductions, and the
  fumbles. The influence effects (Stun, Freeze, Petrify, Trap,
  Confusion, Convert, Fear, Sleep, Disruption, Knockdown, Taunt) have
  their duration in `Min` and their label takes it as text
  (`DamageStun={^E}Stun target{%t0}` → "Stun target for 1.5
  Seconds"; retaliation through `RetaliationFixedSingleFormatTime`
  → "1.0 Seconds of Stun Retaliation").
- **Specials.** `characterBaseAttackSpeedTag` names the weapon speed
  tag (`tagAttackSpeedVeryFast=Speed:  Very Fast`) and is shown on
  weapons only. `conversionInType` / `conversionOutType` /
  `conversionPercentage` (and the `2` triple) →
  `tagDamageConversion={%.0f0}% {^E}{%s1} converted to {%s2}` with
  `tagConversion<Type>` names (`Life` → "Vitality Damage", `Poison`
  → "Acid Damage"). `racialBonusRace` (`Race001`…`Race018`, one or
  two) with `racialBonusPercentDamage` / `…PercentDefense` /
  `…AbsoluteDamage` / `…AbsoluteDefense` → `RacialBonus…` tags with
  the plural race name (`tagRace003P=Aetherials`), one line per race.
  `augmentSkillName<1..5>` + `augmentSkillLevel<N>` →
  `ItemSkillIncrement={+%d0} {^E}to {^Z}{%s1}` with the skill's
  `skillDisplayName` (following `buffSkillName` / `petSkillName`
  redirections); `augmentMasteryName<1..3>` → `ItemMasteryIncrement`;
  `augmentAllLevel` → `ItemAllSkillIncrement`. `itemSkillName` →
  `tagItemGrantSkill` ("Grants Skill:") + name + the autocast
  condition from `itemSkillAutoController` (a controller record whose
  `triggerType` maps to `tagAutoSkillCondition01..12`: `LowHealth`
  01, `LowMana` 02, `HitByEnemy` 03, `HitByMelee` 04,
  `HitByProjectile` 05, `CastBuff` 06, `AttackEnemy` 07, `OnEquip`
  08, `HitByCrit` 09, `AttackEnemyCrit` 10, `Block` 11, `OnKill` 12,
  with `chanceToRun` as `%d0`) then the skill's `skillBaseDescription`
  and its own lines at the level `itemSkillLevelEq` yields (an
  equation in `itemLevel`: `"1"`, `"2"`, `"itemLevel/4+1"` …,
  floored, ≥ 1; skill arrays are indexed by level − 1 and capped).
  `modifierSkillName<1..6>` + `modifiedSkillName<N>` → the modifier
  record's lines (a `SkillSecondary_PetModifier` redirects through
  `petSkillName`) suffixed with `tagItemSkillModified= {^E}to
  {^Z}{%s0}`. `petBonusName` → `tagPetBonusNameAllPets` heading then
  the record's lines. `itemText` is the flavor line.
- **Global chance.** `offensiveGlobalChance` / `retaliationGlobalChance`
  head the lines whose `…Global` flag is set, indented, under
  `GlobalPercentChanceOfOneTag` when any of them carries `…XOR`, else
  `…AllTag`; XOR members keep their own chance prefix.
- **`attributeScalePercent`.** Present on gear, largest on two-handed
  weapons (census by class: 2H melee 30–80, 2H ranged 22–70, 1H
  weapons 8–45, armor and jewelry 20–40). It multiplies the values of
  the item's **prefix and suffix** (GD Stash applies the base item's
  percent to both affixes' stats, and the designers' 2H-weapon affix
  premium is common knowledge); this app does **not** apply it to the
  base record's own numbers — GD Stash does, but an item authored with
  `+220%` physical and `+220%` internal trauma would then read
  `+220%` / `+396%`, which no designer writes, so the base values are
  taken as displayed. Which families scale, per GD Stash's categories
  (eyes-only; unverified in-game): flat and percent offense other than
  `offensivePhysical`, `offensivePierceRatio`, `offensiveLifeLeech`,
  `offensiveCritDamage`, `offensivePercentCurrentLife`,
  `offensiveDamageMult`, `offensiveManaBurn`, the base damages and the
  influences; every duration-damage family; `retaliationDamagePct`;
  `weaponDamagePct`; `damageAbsorptionPercent`. Never defenses,
  character values, or retaliation flats. Scaled values are truncated.
- **`lootRandomizerJitter`** (15–50 on affixes and completion
  bonuses) is the ± roll range the game applies per item seed; the
  seed → roll mapping is not public (GD Stash says as much), so every
  number shown is the record's nominal value.
- **Requirements.** `levelRequirement` is explicit on base, affix,
  component, augment and ascendant records and the item takes the
  maximum. `strengthRequirement` / `dexterityRequirement` /
  `intelligenceRequirement` exist on the templates but are zero on
  every item record; the attribute requirements come from the cost
  formulas: the base's `itemCostName` (`records/game/itemcostformulas
  _<rarity|slot>.dbr`, default `records/game/itemcostformulas.dbr`)
  holds `<class><Attribute>Equation` strings in `itemLevel` and
  `totalAttCount` — class prefixes `head`, `shoulders`, `chest`,
  `legs`, `feet`, `hands`, `waist`, `ring`, `amulet`, `axe`, `mace`,
  `sword`, `dagger`, `scepter`, `melee2h` (every 2H melee class),
  `ranged1h`, `ranged2h`, `shield`, `offhand`; medals have none.
  `totalAttCount` is one per amount, percent modifier, duration
  modifier and max-resistance on the base, prefix and suffix, plus
  one per racial bonus (GD Stash's count, eyes-only); the result is
  rounded up as `TQVaultAE` does for the same engine. Rendered
  through `MeetsRequirement=Required {%s0}: {%.0f1}` with
  `LevelRequirement=Player Level`, `Strength=Physique`,
  `Dexterity=Cunning`, `Intelligence=Spirit`.
- **Sets.** `itemSetName` → a set record with `setName` (tag),
  `setMembers` (records) and per-piece bonus arrays indexed by pieces
  worn − 1 (`characterOffensiveAbility = [0, 0, 65, 65, 65]` on a
  five-piece set is "+65 Offensive Ability" from three pieces); the
  tier shown for `n` pieces is what `n` adds over `n − 1`.
- **Components** are all single-piece today (every one of the 107
  `ItemRelic` records has `completedRelicLevel = 1`); the partial
  scaling GD Stash still carries for older saves is not implemented.
- **Coverage** (the user's transfer stashes, component storage,
  four characters' sacks, equipment and own stashes, and the imported
  GD Stash collection of 3,205 items: 4,114 items in all): 53,775
  lines; unknown attributes 0 distinct after the vocabulary was
  extended from the first run's 58 (every one a non-stat bookkeeping
  variable — blueprint reagent quantities, quest UIDs, physics and
  animation parameters); no missing tags; one missing record — an
  imported item whose suffix the export spells `records/items/l`
  followed by 41 NUL bytes (a corrupt string GD Stash wrote; the
  `.gds` import carries it verbatim and the renderer reports it,
  never drops it). Numbers are those of the run on 2026-09-06;
  `examples/stat_coverage.rs` reproduces them.
- **Departures from tq-univault's renderer**, deliberate: no
  `attributeScalePercent` on the record's own lines (see above); the
  duration multiplier uses `DurationMin` (Grim Dawn records carry no
  `DurationMax` arrays worth reading); no `totalAttCount` exclusions
  (TQ's list of uncounted base stats does not exist in GD Stash's
  count); requirements render Grim Dawn's names.
- **Unverified in-game:** the `%.1f` formats print "3.0 Seconds" and
  "25.0% Chance of" as the tags spell them — GD Stash's notes
  paraphrase the game as "3 Seconds" and "25%", so the engine may
  trim trailing zeros; whether the game's `DamagePhysical` line
  really shows a `+` on non-weapon flat physical damage; the scaling
  categories above; the ceil in the requirement equations; the GD-only
  colour letters (`^E` bonus tint, `^S` base white, `^Z` skill names,
  `^H` highlighted numbers) are rendered from the theme, not from a
  verified palette.

## Item search — 2026-09-06

The rules the store search (`grimvault-core::search`, the GUI's
search view, `examples/search_cli.rs`) applies over the stat lines
above. Ported in shape from tq-univault's `query.rs` (MIT OR
Apache-2.0, same author); every rule was re-checked against the
3,205-item GD Stash collection in the user's vault store.

- **Stat template.** A line's template is its text with every number
  replaced by `#`: "+24% Pierce Resistance" → "+#% Pierce
  Resistance", "9-67 Lightning Damage" → "#-# Lightning Damage",
  "45.0 Second Skill Recharge" → "# Second Skill Recharge". A `-` is
  part of the number only when directly attached and not itself
  following a digit, so "-15% …" is one negative number and "9-67"
  two; a `+` stays in the wording. The template is the key the
  search's stat vocabulary groups the store's lines under (the picker
  offers only templates the store's items actually produce) and one
  of the two haystacks a stat criterion matches against — the other
  is the line's text — so a picked template, free text ("cold
  damage") and text with numbers ("40% cold") all hit.
- **Value window.** A criterion's `min`/`max` compare the line's
  largest *rendered* number (`StatLine::values`, the numbers the
  template formatted — not digits found in prose), so "12-31
  Physical Damage" is bounded on 31 and a numberless line ("Speed:
  Very Slow") never satisfies a bounded criterion. Rendered numbers
  are the record's nominal values (the seed roll is not public, see
  "Item stat lines"), so a window is approximate by
  `lootRandomizerJitter` in-game.
- **Which lines.** "Any stat" reads every block the tooltip renders
  — base, prefix, suffix, transmute modifier, component and its
  completion bonus, augment, ascendant bonus — except a granted
  skill's description prose (`LineKind::SkillDescription`); "Affix
  stat" reads the prefix and suffix blocks only; "Affix name" reads
  the affixes' `lootRandomizerName`s. Requirement lines, set members
  and set bonuses are not stat lines.
- **Three-way answers.** A stat or affix criterion that finds no
  matching line is *unresolved*, not excluded, when one of the blocks
  it read names a record no database layer has (the item's
  `unrendered` carries the `MissingRecord`); a requirement cap passes
  a requirement the item does not state, fails a stated one above the
  cap whatever else is unknown, and is unresolved when the base
  record is missing (the effective requirement is incomplete);
  rarity, category and set need the base record and are unresolved
  without it; the socket filter reads the item's own `relicName` and
  always resolves. The GUI counts unresolved items in its summary
  line rather than hiding them silently.
- **Rarity** in the search is the *displayed* rarity — the base's
  `itemClassification` raised by a rarer affix, a quest item always
  quest — the colour the game names the item in; the tile border in
  the bucket view still shows the base's own classification.
- **Sort keys.** Name (case-folded), rarity (displayed, tier order
  common → legendary, quest after), level requirement (the item's
  effective `levelRequirement`, the maximum over its gating records),
  type (the `Bucket` display order). Every key ranks ascending with a
  name tiebreak; the direction is applied by the caller
  (`univault_ui::sort::SortDirection` in the GUI, `--desc` in the
  CLI). A column first opened sorts names and types A→Z, rarity and
  level highest first.
- **Not ported from tq-univault:** the expansion-origin filter.
  Grim Dawn's expansion records share the base game's path
  namespace (`gdx1/database/GDX1.arz` defines
  `records/items/gearhead/d112_head.dbr` exactly as the base
  archive spells its own), unlike Titan Quest's `XPACK*` prefixes,
  so an item's origin is knowable only from *which archive* supplied
  its record — and `GameData` composes anonymous layers (mods first
  as fill, then whichever shipped archives exist), so naming the
  layer would need labelled layer sets through both loaders, the
  headless check and the real-file tests. Left out; a labelled
  `LayerSet` is the follow-up if the filter is wanted.

## Sockets: components and augments — 2026-09-07

The rules `grimvault-core::socket` applies when an item gains or
gives up its component (`relicName`) or augment (`augmentName`), for
the inspector, `vault_cli attach` / `detach`, and the tooltip.
Established on the user's install (base, `gdx1`–`gdx3`, two mods) and
saves (five characters, both transfer stashes: 655 items, of which
37 socketed and 25 augmented); gated by `tests/socket_real.rs` under
`$GRIMVAULT_GAME_DIR` / `$GRIMVAULT_SAVE_DIR`. GD Stash (eyes-only)
pointed at the variables; every fact below was read from the real
database and files.

- **Which items a part fits is the part record's own choice.**
  `itemrelic.tpl` and `itemenchantment.tpl` carry one boolean per
  equipment slot, named plainly: `head`, `shoulders`, `chest`,
  `hands`, `legs`, `feet`, `waist`, `amulet`, `ring`, `medal`,
  `shield`, `offhand`, `axe`, `mace`, `sword`, `dagger`, `scepter`,
  `ranged1h`, `axe2h`, `mace2h`, `sword2h`, `spear2h`, `staff`, and
  `spear`. A flag absent from a record reads as false (the 107
  `ItemRelic` records carry between 35 and 107 of them each; the 386
  `ItemEnchantment` records carry all). A part goes on an item whose
  `Class` maps to a flagged slot (`socket::Slot::of_class`: the
  seven `ArmorProtective_*`, three `ArmorJewelry_*`, `WeaponArmor_*`
  shield and off-hand, the one- and two-handed `WeaponMelee_*` and
  `WeaponHunting_*` classes, `WeaponMagical_Staff`). `spear` (a
  one-handed spear) has no shipped class and no shipped part sets it;
  `staff` is never set on a shipped part either. **Verified:** every
  one of the 37 socketed and 25 augmented items in the saves carries
  a part whose flags admit its host's class.
- **Components are single-piece and complete at
  `relicCompletionLevel` 0.** All 107 records have
  `completedRelicLevel = 1`; every loose component (stacks of 1–3)
  and every socketed one in the saves reads 0 — so the "var1 ≥
  pieces" completeness GD Stash still tests belongs to the partial
  components older game versions dropped, and `attach` writes 0.
- **No completion bonus.** `relicBonus` is empty on all 37 socketed
  items, characters up to level 93 included; 83 of the 107 records
  still name a `bonusTableName` (`records/items/lootaffixes/
  completion/completionbonus_*.dbr`, a `LootRandomizerTable` of
  `randomizerName<N>` / `randomizerWeight<N>` entries), which the game
  no longer rolls — Forgotten Gods folded the bonuses into the
  components. `attach` therefore writes no bonus; the table reader was
  not built.
- **Seeds and the augment level.** `relicSeed` is 0 on 17 of the 37
  socketed items and arbitrary on the rest; `augmentSeed` is
  non-zero on all 25; the augment level word (`enchantmentLevel` in
  GD Stash's naming, `Item::unknown` here) is 0 throughout. `attach`
  takes the seed from the caller (the shell's clock-mixed value, the
  CLI's argument) and writes level 0; `detach` zeroes the seed it
  frees and hands the part back as a stand-alone item of stack 1.
  **Round trip:** detaching every socketed and augmented item in the
  saves and re-attaching under the original seed reproduces the item
  exactly.
- **Nothing is destroyed.** A filled socket refuses a second part
  (`SocketError::Occupied`) rather than overwriting; a freed part
  goes to the vault store under the host's origin. In-game the
  Inventor charges iron for the same removal; this app does not.

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
eyes-only); block 19's slot ids and the class → slot mapping (GD
Stash's constants agree with the nine observed, and every record of
the user's two collections, 2,945 in all, sits under the slot its
class maps to); the respec rules — both records' values on every
layer, the health / energy formula and the tree membership on every
real save; the `formulas.gst` layout, its `ItemArtifactFormula` rule,
and the game's append-at-end behaviour (three real files, one
game-written diff); GD
ARZ/ARC header and entry layouts, LZ4-block compression, the extra
decompressed-size field; TQ zlib + 2-byte ARC skip; the full XOR
scheme and block/checksum semantics; GD item field order including
the v8/v11 additions; the `.gds` version-3 layout end to end on both
of the user's exports; the socket rules — the slot flags on every
shipped part, the completion level, the absent bonus, and the
detach → attach round trip on every socketed item in the saves; no
relevant crates.io crates.

**Unverified:** whether the game reads a `formulas.gst` or
`transmutes.gst` this app appended to (the round trips and a
game-written diff are the evidence; no in-game read yet), whether it
minds an illusion record it would not itself have unlocked (e.g. a
mod-only record imported into the main campaign), and whether
`formulasVersion` 2 files exist anywhere (refused, not typed);
whether the 8-byte ARZ entry trailer is a FILETIME or
padding (preserved verbatim either way); whether block 20's zero word
is a mod name or a bare `u32` (identical bytes on the base game; only
a mod's `reagents.gst` would tell); whether the game writes a
zero-count entry or drops it (this app drops it; GD Stash does not
say); which of block 16's two word pairs is the v12 addition (GD
Stash and this crate name them the other way round); the shrine
lists' restored / discovered split (GD Stash's reading, not yet seen
in-game); whether the game accepts a reset character — re-offers the
mastery choice with `masteries_allowed` kept at 2, tolerates the
cleared class tag, and drops hotbar slots that name a removed skill
(no file this app wrote has been loaded by the game yet); whether
IAGD ever shipped
`GDCryptoDataBuffer.cs` itself (inferred from GDParser's derived
file); GD Stash's Nexus/ModDB permission text (HTTP 403); whether the "multiple count
groups" string-table loop in gdlc/lib-gddb reflects real files or
defensive coding; the fields the block-typing pass could not name
(the v8 per-skill byte, the two v12 stats words, the six shrine
lists, `Factions::faction`); skills v7 and stats v8/v10 layouts (no
sample); the `.gds` version-1 and version-2 layouts (read by GD
Stash's version gates; no sample — v1.90a writes only 3); the `Sex`
mapping (0 female / 1 male, inferred from character names only);
whether the game's monster-infrequent symbol excludes soulbound
(faction) rares the way this app's rule does, and whether item
ascension is gated on item level (no data names either; "Item facets"
above); whether the game accepts a component or augment this app
socketed — a `relicSeed` it did not roll, or a part on a slot it
flags but the game's own placement rule (not readable from the data)
might still refuse — and whether it minds a freed component stacking
with dropped ones (no file this app wrote has been loaded by the
game yet).
