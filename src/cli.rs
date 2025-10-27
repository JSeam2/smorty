use crate::constants;
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "smorty")]
#[command(about = constants::SMORTY_ASCII, long_about = None)]
#[command(after_help = constants::SMORTY_DESCRIPTION)]
pub struct Cli {
    /// Path to config file
    #[arg(short, long, default_value = "config.toml")]
    pub config: String,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Generate spec IR (Intermediate Representation) from config using AI
    GenSpec,

    /// Generate endpoint IR from config using AI
    GenEndpoint,

    /// Generate database migration from IR
    GenMigration,

    /// Run database migration
    Migrate,

    /// Run the indexer (fetch and process events)
    #[command(hide = true)]
    Index {
        /// Run in daemon mode (continuously index new blocks)
        #[arg(short, long)]
        daemon: bool,
    },

    /// Start the API server
    #[command(hide = true)]
    Serve {
        /// IP address to bind to
        #[arg(short, long, default_value = "0.0.0.0")]
        address: String,

        /// Port to bind to
        #[arg(short, long, default_value = "3000")]
        port: u16,
    },

    /// Run both indexer and API server
    Run {
        /// IP address to bind to
        #[arg(short, long, default_value = "0.0.0.0")]
        address: String,

        /// Port to bind to
        #[arg(short, long, default_value = "3000")]
        port: u16,
    },
}
