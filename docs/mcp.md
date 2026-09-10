# The Grim Vault MCP server

`crates/grimvault-mcp` is a read-only [Model Context Protocol](https://modelcontextprotocol.io)
server over a Grim Dawn install and its saves, built for planning and
refining builds around what the player actually owns. It is the
sibling of tq-univault's `univault-mcp`, on the same SDK (`rmcp`),
with Grim Dawn's own shapes: masteries, devotion constellations, item
sets, the affix reference, blueprints, and the Grim Vault store.

The contract is recorded in `.claude/rules/ARCHITECTURE.md` ("Planned
surfaces"): JSON-RPC over **stdio only**, spawned as a child process
by the client; no listening sockets; **nothing is ever written**.

## Building and registering

```sh
cargo build --release -p grimvault-mcp
```

The repository's `.mcp.json` points Claude Code at
`target/release/grimvault-mcp` whenever it is started in this
directory. To use the server from anywhere, register the binary by
absolute path, for example:

```sh
claude mcp add --scope user grimvault -- /Users/you/Projects/grim-vault/target/release/grimvault-mcp
```

Any other MCP client (Claude Desktop, an IDE) takes the same command
and no arguments.

## Where it reads from

The server reads the desktop shell's `settings.json` under the config
directory — the game directory, the save directory, the store file
(`storeFile`, which may sit on a NAS), and the campaign opened last —
so the paths are typed once, in the app. `GRIMVAULT_CONFIG_DIR`
overrides the config directory, as it does for the app. Without a
settings file every tool answers with what to do; `overview` says
what is configured.

The record databases and text archives of every layer (base game,
`gdx1`–`gdx3`, installed mods as fill layers) load once per process,
on the first tool that needs them — about two seconds from the NAS.
The item bitmap archives are never read. Every tool that looks at a
save, a stash, or the vault store re-reads the file on call, uncached
and length-checked, because the game and the app write those files at
any time.

## Tools

| Tool | What it answers |
|---|---|
| `overview` | directories, campaigns and the open one, characters, store size, mods, whether the database is loaded |
| `list_characters` | every character under `main/` and `user/` with level, class, masteries, hardcore, iron bits |
| `get_character` | one character: attributes and pools, unspent points, the build (skills grouped by mastery with tiers and points, devotions grouped by constellation with completion, item-granted skills), gear in both weapon sets, sacks, own stash, play statistics |
| `get_stash` | a campaign's transfer stash tabs, or its component / crafting-material storage with counts |
| `list_buckets`, `get_store` | the vault store by group and bucket, each item with its origin and moment |
| `search_items` | the app's typed query over the store, the campaign's stashes, and every character: stat lines with value bounds, affix names, requirement caps, rarity, group or bucket, set membership, socket state, monster infrequent, double rare, ascension; each hit with its location |
| `get_item_details` | the tooltip for an item given its records and seed: blocks by source, requirements, set bonuses |
| `list_masteries`, `get_mastery`, `get_skill` | the mastery trees, and any skill's stat lines rendered at chosen levels |
| `list_constellations`, `get_constellation` | the devotion constellations with affinities and every star's lines |
| `list_sets`, `get_set` | item sets with members and per-piece bonuses |
| `search_affixes` | the affix reference: every named prefix and suffix with its grants as ranges across tiers |
| `list_blueprints` | blueprints the vault knows, a campaign has learned, or has not learned yet |
| `search_records`, `get_record`, `list_record_classes`, `translate_tag` | the entire layered record database, each record naming the layers that define it |

A record path is accepted in either spelling (`records/...` or
`RECORDS\...`); tools return the database's own lowercase spelling,
the one the save files use.

## Trying it without a client

The server speaks newline-delimited JSON-RPC on stdin/stdout. A
throwaway driver that sends `initialize`, then `tools/call`, is a
dozen lines of Python; the smoke run on 2026-09-10 used one to
exercise every tool against the real install.
