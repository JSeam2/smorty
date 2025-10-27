use anyhow::{Context, Result};
use clap::Parser;
use smorty::ai::AiClient;
use smorty::cli::{Cli, Commands};
use smorty::config::Config;
use smorty::indexer::Indexer;
use smorty::ir::Ir;
use smorty::migration::Migration;
use smorty::server;
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
        Commands::GenSpec => {
            gen_spec(&config).await?;
        }
        Commands::GenEndpoint => {
            gen_endpoint(&config).await?;
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

async fn gen_spec(config: &Config) -> Result<()> {
    tracing::info!("Starting spec IR generation");

    // Create AI client
    let ai_client = AiClient::new(
        config.ai.openai.api_key.clone(),
        config.ai.openai.model.clone(),
        config.ai.openai.temperature,
    );

    // Generate spec IR
    let ir_generator = Ir::new(ai_client);
    ir_generator.generate_all(config).await?;

    tracing::info!("Spec IR generation complete");

    Ok(())
}

async fn gen_endpoint(config: &Config) -> Result<()> {
    tracing::info!("Starting endpoint IR generation");

    // Create AI client
    let ai_client = AiClient::new(
        config.ai.openai.api_key.clone(),
        config.ai.openai.model.clone(),
        config.ai.openai.temperature,
    );

    // Generate endpoint IR
    let ir_generator = Ir::new(ai_client);
    ir_generator.generate_all_endpoints(config).await?;

    tracing::info!("Endpoint IR generation complete");

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

async fn index(config: &Config, daemon: bool) -> Result<()> {
    tracing::info!("Starting indexer");

    // Create indexer instance
    let indexer = Indexer::new(config).await?;

    // Start indexing
    indexer.start(daemon).await?;

    tracing::info!("Indexer finished");
    Ok(())
}

async fn serve(config: &Config, address: &str, port: u16) -> Result<()> {
    server::serve(config, address, port).await
}

async fn run(config: &Config, address: &str, port: u16) -> Result<()> {
    tracing::info!("Starting indexer and API server on {}:{}", address, port);

    // Start indexer in background
    let config_clone = config.clone();
    let indexer_handle = tokio::spawn(async move {
        match Indexer::new(&config_clone).await {
            Ok(indexer) => {
                if let Err(e) = indexer.start(true).await {
                    tracing::error!("Indexer error: {}", e);
                }
            }
            Err(e) => {
                tracing::error!("Failed to create indexer: {}", e);
            }
        }
    });

    // Start API server
    let server_result = server::serve(config, address, port).await;

    // If server exits, wait for indexer to finish
    indexer_handle.abort();

    server_result
}
