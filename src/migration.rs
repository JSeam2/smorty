use crate::ai::IrGenerationResult;
use crate::config::Config;
use crate::ir::Ir;
use anyhow::{Context, Result};
use chrono::Utc;
use sqlx::migrate::Migrator;
use sqlx::postgres::PgPoolOptions;
use std::fs;
use std::path::Path;

pub struct Migration;

impl Migration {
    /// Generate SQLx migrations from IR files
    pub fn generate_from_ir(config: &Config) -> Result<()> {
        tracing::info!("Generating database migrations from IR");

        // Create migrations directory if it doesn't exist
        let migrations_dir = Path::new("migrations");
        if !migrations_dir.exists() {
            fs::create_dir_all(migrations_dir).context("Failed to create migrations directory")?;
        }

        // Load all IR files
        let ir_results = Ir::load_all_ir(config)?;

        // Generate timestamp for migration file in SQLx format
        // SQLx expects: <VERSION>_<DESCRIPTION>.sql where VERSION is typically YYYYMMDDHHmmss
        let timestamp = Utc::now().format("%Y%m%d%H%M%S").to_string();

        // Collect all table creation SQL
        let mut up_sql = String::new();

        up_sql.push_str("-- Auto-generated migration from IR\n\n");

        for (contract_name, spec_name, ir) in ir_results {
            tracing::info!("  Processing: {} / {}", contract_name, spec_name);

            // Generate CREATE TABLE statement
            let create_table = Self::generate_create_table(&ir)?;
            up_sql.push_str(&format!("-- {}/{}\n", contract_name, spec_name));
            up_sql.push_str(&create_table);
            up_sql.push_str("\n\n");

            // Generate indexes with unique names per table
            for (_idx, index_sql) in ir.table_schema.indexes.iter().enumerate() {
                // Replace table name placeholder
                let mut index_sql = index_sql.replace("{table_name}", &ir.table_schema.table_name);

                // Make index names unique by prefixing with table name
                // This handles cases like: CREATE INDEX idx_name ON table(column)
                if let Some(idx_pos) = index_sql.find("CREATE INDEX ") {
                    if let Some(on_pos) = index_sql.find(" ON ") {
                        let start = idx_pos + "CREATE INDEX ".len();
                        let old_index_name = &index_sql[start..on_pos];
                        let new_index_name = format!("{}_{}", ir.table_schema.table_name, old_index_name);
                        index_sql = index_sql.replace(old_index_name, &new_index_name);
                    }
                }

                up_sql.push_str(&index_sql);
                up_sql.push_str(";\n");
            }
            up_sql.push_str("\n");
        }

        // Write migration files with SQLx naming convention: <VERSION>_<DESCRIPTION>.sql
        let up_file = migrations_dir.join(format!("{}_auto_generated_from_ir.sql", timestamp));

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
                sql.push('\n');
            }
        }

        sql.push_str(");\n");

        Ok(sql)
    }

    /// Run migrations using sqlx
    /// Uses runtime migration loading to support dynamically generated migrations
    pub async fn run_migrations(database_url: &str) -> Result<()> {
        tracing::info!("Running database migrations");

        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await
            .context("Failed to connect to database")?;

        // Use runtime migrator to read migrations from filesystem at runtime
        let migrations_dir = Path::new("./migrations");
        let migrator = Migrator::new(migrations_dir)
            .await
            .context("Failed to load migrations from ./migrations directory")?;

        migrator
            .run(&pool)
            .await
            .context("Failed to run migrations")?;

        tracing::info!("Migrations completed successfully");

        Ok(())
    }
}
