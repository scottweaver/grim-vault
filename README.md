# Grim Vault

A platform-independent item vault for **Grim Dawn** — extra storage
outside the game, and a way to move items between the shared
(transfer) stash and that storage. Written in Rust with an
egui/eframe front-end; runs on Windows, macOS, and Linux. The
Grim Dawn sibling of [TQ-AE-Univault](https://github.com/scottweaver/TQ-AE-Univault).

## Status

Early. What works today:

- Reads the game's record database (base game + Ashes of Malmouth +
  Forgotten Gods), localization, and item bitmaps.
- Reads every character (`player.gdc`) and the transfer stash
  (`transfer.gst`) losslessly — an unmodified file re-encodes
  byte-for-byte, and the app refuses to edit any file it cannot
  reproduce.
- Vaults items out of the transfer stash into its own store and
  places them back, with automatic backups.
- Reads and edits the component and crafting-material storage
  (`reagents.gst`) the game shows as its Components / Crafting
  Materials tabs: components and materials move between it, the
  transfer stash, and the store by drag-and-drop, counts merging by
  record. The illusion collection (`transmutes.gst`) is read
  losslessly but never written.
- Every block of `player.gdc` is typed and re-encodes faithfully
  after an edit, so character inventories can be vaulted too; the
  first GUI shell shows characters read-only and the next milestone
  enables editing them.

## Safety

The game's files are the source of truth; this app is a guest editor.

- Every write to a game-owned file takes a backup first
  (`<file>.grimvault-bak-<timestamp>`, five kept), writes in place,
  and re-reads the result to verify it.
- The game's own backup rotation (`transfer.t00`–`.t09`,
  `player.g00`/`.g01`) is never touched.
- Close the game before using the app. If the stash changes on disk
  while the app has unsaved edits, autosave suspends and asks.

## Build and run

Requires a stable Rust toolchain (edition 2024).

```sh
cargo run --release -p grimvault-gui
```

On first run the app asks for two directories:

- **Game dir** — the Grim Dawn install (contains `database/database.arz`).
- **Save dir** — the folder holding `transfer.gst` and `main/`
  (`Documents/My Games/Grim Dawn/save`, or Steam cloud:
  `<Steam>/userdata/<account>/219990/remote/save`).

Settings and the vault store (`vault-store.json`) live in the app's
config directory: `~/Library/Application Support/grim-vault` (macOS),
`%APPDATA%\grim-vault` (Windows), `$XDG_CONFIG_HOME/grim-vault`
(Linux).

Every command-line tool reads the same saved settings, so the paths
are typed once, in the app. Headless check of the load path, without
a window:

```sh
cargo run --release -p grimvault-gui -- --check
```

Command-line examples against the core library (the store defaults
to the one in the config directory):

```sh
cargo run --release -p grimvault-core --example smoke
cargo run --release -p grimvault-core --example vault_cli -- list
cargo run --release -p grimvault-core --example vault_cli -- reagents
```

Any of them accepts `--game DIR`, `--save DIR`, and (for `vault_cli`)
`--store FILE` to override the saved paths, and `--check` still takes
`<game dir> <save dir>` explicitly.

## Workspace

| Crate | Role |
|---|---|
| `univault-engine` | Game-agnostic engine formats shared with tq-univault: little-endian reader/writer, ARC/ARZ containers (zlib or LZ4), localization text, `.tex` headers, grid placement. Pure; never touches the filesystem. |
| `univault-io` | Shell-side file IO: uncached verified reads, backup-first synced writes. |
| `univault-ui` | Art-free egui kit: theme, chrome components, slicing. |
| `grimvault-core` | Grim Dawn formats (rolling-XOR save codec, `player.gdc`, `*.gst`), the layered game-data facade, the vault store, buckets, and item transfer. GUI-agnostic. |
| `grimvault-gui` | The desktop app. |

Architecture constraints live in `.claude/rules/ARCHITECTURE.md`;
format provenance and licensing in `docs/format-references.md`.

## License

MIT OR Apache-2.0. Parsers are ported from MIT references
([TQVaultAE](https://github.com/EtienneLamoureux/TQVaultAE) via
tq-univault, [gdlc](https://github.com/dandels/gdlc),
[yagde](https://github.com/wr8fdy/yagde)); GPL references are
consulted, never transcribed.
