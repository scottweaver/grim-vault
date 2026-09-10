//! Read-only MCP server exposing Grim Dawn game data over stdio.
//!
//! The client (Claude Code, Claude Desktop, any MCP client) spawns
//! this binary as a child process and speaks JSON-RPC on its
//! stdin/stdout. The record database is read once per process; every
//! tool call re-reads the saves and the vault store — the game or the
//! desktop shell may write them at any time, so nothing here is
//! authoritative and nothing is ever written back (ARCHITECTURE.md
//! "Planned surfaces").

use rmcp::ServiceExt;
use rmcp::transport::stdio;

mod build;
mod devotion;
mod gamedb;
mod server;
mod skills;
mod view;
mod world;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let service = server::GrimVault::from_settings().serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
