use clap::{Parser, Subcommand};

/// paws: run-anywhere CI/CD pipelines, backed by Dagger.
#[derive(Parser)]
#[command(name = "paws", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Build and test a language target (node, rust, docker, ...).
    Ci {
        #[arg(long)]
        toolchain: Option<String>,
    },
    /// Build and push a container image.
    Docker {
        #[arg(long)]
        push: bool,
    },
    /// Compute the next semantic version from commit history.
    Semver {
        #[arg(long, default_value = "main")]
        branch: String,
    },
    /// Run the audit/compliance scanner suite.
    Audit,
    /// Publish generated docs (e.g. rustdoc) to GitHub Pages.
    Docs,
    /// Provision toolchains concurrently (rust, node, python, ...).
    Provision {
        /// Comma-separated ecosystems to install, e.g. "rust,node,python".
        #[arg(long, value_delimiter = ',')]
        toolchains: Vec<String>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Ci { toolchain } => {
            println!("ci: toolchain={:?} (unimplemented)", toolchain);
        }
        Commands::Docker { push } => {
            println!("docker: push={push} (unimplemented)");
        }
        Commands::Semver { branch } => {
            println!("semver: branch={branch} (unimplemented)");
        }
        Commands::Audit => {
            println!("audit: (unimplemented)");
        }
        Commands::Docs => {
            println!("docs: (unimplemented)");
        }
        Commands::Provision { toolchains } => {
            println!("provision: toolchains={:?} (unimplemented - installers not wired)", toolchains);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::Cli;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
