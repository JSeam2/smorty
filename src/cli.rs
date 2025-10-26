use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "smorty")]
#[command(about = "smorty is a Smart Indexer which allows you to index events on the EVM easily.", long_about = None)]
pub struct Cli {
    /// Path to config file
    #[arg(short, long, default_value = "config.toml")]
    pub config: String,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Generate IR (Intermediate Representation) from config using AI
    GenerateIr {
        /// Force regeneration even if IR already exists
        #[arg(short, long)]
        force: bool,
    },

    /// Generate database migrations from IR
    GenerateMigrations,

    /// Run database migrations
    Migrate,

    /// Run the indexer (fetch and process events)
    Index {
        /// Run in daemon mode (continuously index new blocks)
        #[arg(short, long)]
        daemon: bool,

        /// Specific contract to index (optional, defaults to all)
        #[arg(short, long)]
        contract: Option<String>,

        /// Specific spec to index (requires --contract)
        #[arg(short, long)]
        spec: Option<String>,
    },

    /// Start the API server
    Serve {
        /// Host to bind to
        #[arg(long, default_value = "127.0.0.1")]
        host: String,

        /// Port to bind to
        #[arg(short, long, default_value = "3000")]
        port: u16,
    },

    /// Run both indexer and API server
    Run {
        /// Host to bind to
        #[arg(long, default_value = "127.0.0.1")]
        host: String,

        /// Port to bind to
        #[arg(short, long, default_value = "3000")]
        port: u16,
    },
}