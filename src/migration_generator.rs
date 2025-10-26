use crate::ai::IrGenerationResult;
use crate::config::Config;
use crate::ir_generator::IrGenerator;
use anyhow::{Context, Result};
use chrono::Utc;
use std::fs;
use std::path::Path;

pub struct MigrationGenerator;

impl MigrationGenerator {
    /// Generate SQLx migrations from IR files
    pub fn generate_from_ir(config: &Config) -> Result<()> {
        tracing::info!("Generating database migrations from IR");

        // Create migrations directory if it doesn't exist
        let migrations_dir = Path::new("migrations");
        if !migrations_dir.exists() {
            fs::create_dir_all(migrations_dir).context("Failed to create migrations directory")?;
        }

        // Load all IR files
        let ir_results = IrGenerator::load_all_ir(config)?;

        // Generate timestamp for migration file
        let timestamp = Utc::now().format("%Y%m%d%H%M%S").to_string();

        // Collect all table creation SQL
        let mut up_sql = String::new();
        let mut down_sql = String::new();

        up_sql.push_str("-- Auto-generated migration from IR\n\n");

        for (contract_name, spec_name, ir) in ir_results {
            tracing::info!("  Processing: {} / {}", contract_name, spec_name);

            // Generate CREATE TABLE statement
            let create_table = Self::generate_create_table(&ir)?;
            up_sql.push_str(&format!("-- {}/{}\n", contract_name, spec_name));
            up_sql.push_str(&create_table);
            up_sql.push_str("\n\n");

            // Generate indexes
            for index_sql in &ir.table_schema.indexes {
                let index_sql = index_sql.replace("{table_name}", &ir.table_schema.table_name);
                up_sql.push_str(&index_sql);
                up_sql.push_str(";\n");
            }
            up_sql.push_str("\n");
        }

        // Write migration files
        let up_file = migrations_dir.join(format!("{}.sql", timestamp));

        fs::write(&up_file, up_sql).context("Failed to write up migration file")?;

        tracing::info!("Generated migration files:");
        tracing::info!("  Up:   {:?}", up_file);

        Ok(())
    }

    /// Generate CREATE TABLE statement from IR
    fn generate_create_table(ir: &IrGenerationResult) -> Result<String> {
        let mut sql = format!(
            "CREATE TABLE IF NOT EXISTS {} (\n",
            ir.table_schema.table_name
        );

        // Add columns
        for (i, column) in ir.table_schema.columns.iter().enumerate() {
            sql.push_str(&format!("    {} {}", column.name, column.column_type));

            if i < ir.table_schema.columns.len() - 1 {
                sql.push_str(",\n");
            } else {
                sql.push_str("\n");
            }
        }

        sql.push_str(");\n");

        Ok(sql)
    }

    /// Run migrations using sqlx
    pub async fn run_migrations(database_url: &str) -> Result<()> {
        use sqlx::postgres::PgPoolOptions;

        tracing::info!("Running database migrations");

        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await
            .context("Failed to connect to database")?;

        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .context("Failed to run migrations")?;

        tracing::info!("Migrations completed successfully");

        Ok(())
    }
}
