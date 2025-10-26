use anyhow::{Context, Result};
use clap::Parser;
use smorty::ai::AiClient;
use smorty::cli::{Cli, Commands};
use smorty::config::Config;
use smorty::ir::Ir;
use smorty::migration::Migration;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "smorty=info,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Parse CLI arguments
    let cli = Cli::parse();

    // Load config
    let config =
        Config::load(&cli.config).context(format!("Failed to load config from: {}", cli.config))?;

    tracing::info!("Loaded config from: {}", cli.config);

    // Handle commands
    match cli.command {
        Commands::GenIr { force } => {
            gen_ir(&config, force).await?;
        }
        Commands::GenMigration => {
            gen_migration(&config)?;
        }
        Commands::Migrate => {
            migrate(&config).await?;
        }
        Commands::Index { daemon } => {
            index(&config, daemon).await?;
        }
        Commands::Serve { address, port } => {
            serve(&config, &address, port).await?;
        }
        Commands::Run { address, port } => {
            run(&config, &address, port).await?;
        }
    }

    Ok(())
}

async fn gen_ir(config: &Config, _force: bool) -> Result<()> {
    tracing::info!("Starting IR generation");

    // Create AI client
    let ai_client = AiClient::new(
        config.ai.openai.api_key.clone(),
        config.ai.openai.model.clone(),
        config.ai.openai.temperature,
    );

    // Generate IR
    let ir_generator = Ir::new(ai_client);
    ir_generator.generate_all(config).await?;

    tracing::info!("IR generation complete");

    Ok(())
}

fn gen_migration(config: &Config) -> Result<()> {
    tracing::info!("Generating migration from IR");

    Migration::generate_from_ir(config)?;

    tracing::info!("Migration generation complete");

    Ok(())
}

async fn migrate(config: &Config) -> Result<()> {
    tracing::info!("Running database migrations");

    Migration::run_migrations(&config.database.uri).await?;

    tracing::info!("Migrations complete");

    Ok(())
}

async fn index(
    _config: &Config,
    _daemon: bool,
) -> Result<()> {
    tracing::info!("Starting indexer");
    tracing::warn!("Indexer not yet implemented");
    // TODO: Implement indexer
    Ok(())
}

async fn serve(_config: &Config, address: &str, port: u16) -> Result<()> {
    tracing::info!("Starting API server on {}:{}", address, port);
    tracing::warn!("API server not yet implemented");
    // TODO: Implement API server
    Ok(())
}

async fn run(_config: &Config, address: &str, port: u16) -> Result<()> {
    tracing::info!("Starting indexer and API server on {}:{}", address, port);
    tracing::warn!("Combined mode not yet implemented");
    // TODO: Implement combined mode
    Ok(())
}
