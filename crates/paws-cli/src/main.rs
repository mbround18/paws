use clap::Parser;
use paws_cli_core::{Cli, Commands, McpCommand};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // `mcp serve` is handled here rather than inside `paws_cli_core::execute`
    // — `paws-mcp` depends on `paws-cli-core` for its tool handlers, so
    // routing this call from within that lib would be a build-graph cycle.
    // Everything else goes through the normal dispatch.
    if let Commands::Mcp(McpCommand::Serve(_)) = &cli.command {
        return paws_mcp::serve().await;
    }

    paws_cli_core::execute(cli.command).await
}
