mod ai;
mod cli;
mod config;
mod ir_generator;
mod migration_generator;

use anyhow::{Context, Result};
use clap::Parser;
use cli::{Cli, Commands};
use config::Config;
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
    let config = Config::load(&cli.config)
        .context(format!("Failed to load config from: {}", cli.config))?;

    tracing::info!("Loaded config from: {}", cli.config);

    // Handle commands
    match cli.command {
        Commands::GenerateIr { force } => {
            generate_ir(&config, force).await?;
        }
        Commands::GenerateMigrations => {
            generate_migrations(&config)?;
        }
        Commands::Migrate => {
            migrate(&config).await?;
        }
        Commands::Index { daemon, contract, spec } => {
            index(&config, daemon, contract, spec).await?;
        }
        Commands::Serve { host, port } => {
            serve(&config, &host, port).await?;
        }
        Commands::Run { host, port } => {
            run(&config, &host, port).await?;
        }
    }

    Ok(())
}

async fn generate_ir(config: &Config, _force: bool) -> Result<()> {
    tracing::info!("Starting IR generation");

    // Create AI client
    let ai_client = ai::AiClient::new(
        config.ai.openai.api_key.clone(),
        config.ai.openai.model.clone(),
        config.ai.openai.temperature,
    );

    // Generate IR
    let ir_generator = ir_generator::IrGenerator::new(ai_client);
    ir_generator.generate_all(config).await?;

    tracing::info!("IR generation complete");

    Ok(())
}

fn generate_migrations(config: &Config) -> Result<()> {
    tracing::info!("Generating migrations from IR");

    migration_generator::MigrationGenerator::generate_from_ir(config)?;

    tracing::info!("Migration generation complete");

    Ok(())
}

async fn migrate(config: &Config) -> Result<()> {
    tracing::info!("Running database migrations");

    migration_generator::MigrationGenerator::run_migrations(&config.database.uri).await?;

    tracing::info!("Migrations complete");

    Ok(())
}

async fn index(
    _config: &Config,
    _daemon: bool,
    _contract: Option<String>,
    _spec: Option<String>,
) -> Result<()> {
    tracing::info!("Starting indexer");
    tracing::warn!("Indexer not yet implemented");
    // TODO: Implement indexer
    Ok(())
}

async fn serve(
    _config: &Config,
    host: &str,
    port: u16,
) -> Result<()> {
    tracing::info!("Starting API server on {}:{}", host, port);
    tracing::warn!("API server not yet implemented");
    // TODO: Implement API server
    Ok(())
}

async fn run(
    _config: &Config,
    host: &str,
    port: u16,
) -> Result<()> {
    tracing::info!("Starting indexer and API server on {}:{}", host, port);
    tracing::warn!("Combined mode not yet implemented");
    // TODO: Implement combined mode
    Ok(())
}
