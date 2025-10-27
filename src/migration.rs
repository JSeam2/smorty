use crate::ai::IrGenerationResult;
use crate::config::Config;
use crate::ir::Ir;
use crate::schema_diff::{SchemaDiff, TableDiff};
use crate::schema_state::{ColumnState, IndexState, SchemaState, TableState};
use anyhow::{Context, Result};
use chrono::Utc;
use sqlx::migrate::Migrator;
use sqlx::postgres::PgPoolOptions;
use std::fs;
use std::path::Path;

pub struct Migration;

impl Migration {
    /// Generate SQLx migrations from IR files using schema diffing
    pub fn generate_from_ir(config: &Config) -> Result<()> {
        tracing::info!("Generating database migrations from IR");

        // Create migrations directory if it doesn't exist
        let migrations_dir = Path::new("migrations");
        if !migrations_dir.exists() {
            fs::create_dir_all(migrations_dir).context("Failed to create migrations directory")?;
        }

        // Load previous schema state (if it exists)
        let state_file = migrations_dir.join("schema.json");
        let old_state = if state_file.exists() {
            tracing::info!("Loading previous schema state from migrations/schema.json");
            SchemaState::load(&state_file)?
        } else {
            tracing::info!("No previous schema state found - this is an initial migration");
            SchemaState::new()
        };

        // Build new schema state from IR files
        let ir_results = Ir::load_all_ir_specs(config)?;
        let new_state = Self::build_schema_state_from_ir(&ir_results)?;

        // Compute diff
        let diff = SchemaDiff::compute(&old_state, &new_state);

        if !diff.has_changes() {
            tracing::info!("No schema changes detected. Skipping migration generation.");
            return Ok(());
        }

        // Generate timestamp for this migration
        let timestamp = Utc::now().format("%Y%m%d%H%M%S").to_string();

        // Create backup of old schema state if it exists
        if state_file.exists() {
            let backup_file = migrations_dir.join(format!("{}_schema.json", timestamp));
            fs::copy(&state_file, &backup_file)
                .context("Failed to create schema backup")?;

            tracing::info!(
                "Created schema backup: {:?}",
                backup_file.file_name().unwrap()
            );
            tracing::info!(
                "This backup is for recovery purposes in case the new migration fails."
            );
            tracing::info!(
                "You can safely delete old schema backups once migrations are verified to work correctly."
            );
        }

        // Generate migration SQL based on diff
        let migration_sql = Self::generate_migration_sql(&diff)?;

        // Write migration file
        let description = if diff.is_initial() {
            "initial_schema"
        } else {
            "schema_update"
        };
        let migration_file = migrations_dir.join(format!("{}_{}.sql", timestamp, description));

        fs::write(&migration_file, migration_sql)
            .context("Failed to write migration file")?;

        // Save new schema state
        new_state.save(&state_file)?;

        tracing::info!("Generated migration file: {:?}", migration_file);
        tracing::info!("Schema state saved to migrations/schema.json");

        Ok(())
    }

    /// Build SchemaState from IR results
    fn build_schema_state_from_ir(
        ir_results: &[(String, String, IrGenerationResult)],
    ) -> Result<SchemaState> {
        let mut state = SchemaState::new();

        for (contract_name, spec_name, ir) in ir_results {
            let mut table = TableState::new(
                ir.table_schema.table_name.clone(),
                contract_name.clone(),
                spec_name.clone(),
            );

            // Add columns
            for column in &ir.table_schema.columns {
                table.add_column(ColumnState::new(
                    column.name.clone(),
                    column.column_type.clone(),
                ));
            }

            // Add indexes
            for index_sql in &ir.table_schema.indexes {
                // Replace table name placeholder
                let index_sql = index_sql.replace("{table_name}", &ir.table_schema.table_name);

                // Make index names unique by prefixing with table name
                let index_sql = Self::make_index_name_unique(&index_sql, &ir.table_schema.table_name);

                // Extract index name
                let index_name = IndexState::extract_index_name(&index_sql)
                    .unwrap_or_else(|| format!("idx_{}", table.columns.len()));

                table.add_index(IndexState::new(index_name, index_sql));
            }

            state.add_table(table);
        }

        Ok(state)
    }

    /// Generate migration SQL from schema diff
    fn generate_migration_sql(diff: &SchemaDiff) -> Result<String> {
        let mut sql = String::new();

        sql.push_str("-- Auto-generated migration from IR\n");
        sql.push_str(&format!("-- Generated at: {}\n\n", chrono::Utc::now().to_rfc3339()));

        // Handle new tables (initial migration or new tables added)
        if !diff.tables_added.is_empty() {
            sql.push_str("-- Create new tables\n\n");

            for table in &diff.tables_added {
                sql.push_str(&format!(
                    "-- {}/{}\n",
                    table.source.contract_name, table.source.spec_name
                ));

                // Generate CREATE TABLE
                sql.push_str(&Self::generate_create_table_from_state(table)?);
                sql.push_str("\n");

                // Generate indexes
                for index in &table.indexes {
                    sql.push_str(&index.definition);
                    sql.push_str(";\n");
                }
                sql.push_str("\n");
            }
        }

        // Handle dropped tables
        if !diff.tables_dropped.is_empty() {
            sql.push_str("-- Drop removed tables\n\n");

            for table_name in &diff.tables_dropped {
                sql.push_str(&format!("DROP TABLE IF EXISTS {} CASCADE;\n", table_name));
            }
            sql.push_str("\n");
        }

        // Handle modified tables
        if !diff.tables_modified.is_empty() {
            sql.push_str("-- Modify existing tables\n\n");

            for table_diff in &diff.tables_modified {
                sql.push_str(&Self::generate_table_modification_sql(table_diff)?);
            }
        }

        Ok(sql)
    }

    /// Generate SQL for modifying an existing table
    fn generate_table_modification_sql(table_diff: &TableDiff) -> Result<String> {
        let mut sql = String::new();

        sql.push_str(&format!("-- Modify table: {}\n", table_diff.table_name));

        // Add new columns
        for column in &table_diff.columns_added {
            // Check if column has NOT NULL constraint
            let has_not_null = column.column_type.to_uppercase().contains("NOT NULL");

            if has_not_null {
                // Generate warning for NOT NULL columns being added to existing tables
                sql.push_str("-- WARNING: Adding NOT NULL column to existing table with data will fail\n");
                sql.push_str(&format!(
                    "-- You must manually decide how to handle this. Options:\n"
                ));
                sql.push_str(&format!(
                    "-- 1. Add column as nullable first, set default values, then add NOT NULL:\n"
                ));
                sql.push_str(&format!(
                    "--    ALTER TABLE {} ADD COLUMN IF NOT EXISTS {} {};\n",
                    table_diff.table_name,
                    column.name,
                    column.column_type.replace("NOT NULL", "").trim()
                ));
                sql.push_str(&format!(
                    "--    UPDATE {} SET {} = <default_value> WHERE {} IS NULL;\n",
                    table_diff.table_name, column.name, column.name
                ));
                sql.push_str(&format!(
                    "--    ALTER TABLE {} ALTER COLUMN {} SET NOT NULL;\n",
                    table_diff.table_name, column.name
                ));
                sql.push_str(&format!(
                    "-- 2. Add column with a DEFAULT value:\n"
                ));
                sql.push_str(&format!(
                    "--    ALTER TABLE {} ADD COLUMN IF NOT EXISTS {} {} DEFAULT <default_value>;\n",
                    table_diff.table_name, column.name, column.column_type
                ));
                sql.push_str("-- Uncomment and modify one of the approaches above\n\n");
            } else {
                sql.push_str(&format!(
                    "ALTER TABLE {} ADD COLUMN IF NOT EXISTS {} {};\n",
                    table_diff.table_name, column.name, column.column_type
                ));
            }
        }

        // Drop columns
        for column_name in &table_diff.columns_dropped {
            sql.push_str(&format!(
                "ALTER TABLE {} DROP COLUMN IF EXISTS {} CASCADE;\n",
                table_diff.table_name, column_name
            ));
        }

        // Modify columns (type changes)
        for column_mod in &table_diff.columns_modified {
            sql.push_str(&format!(
                "-- WARNING: Manual review required for column type change\n"
            ));
            sql.push_str(&format!(
                "-- ALTER TABLE {} ALTER COLUMN {} TYPE {}; -- Old type: {}\n",
                table_diff.table_name,
                column_mod.column_name,
                column_mod.new_type,
                column_mod.old_type
            ));
        }

        // Drop indexes
        for index_name in &table_diff.indexes_dropped {
            sql.push_str(&format!("DROP INDEX IF EXISTS {};\n", index_name));
        }

        // Add new indexes
        for index in &table_diff.indexes_added {
            sql.push_str(&format!("{};\n", index.definition));
        }

        sql.push_str("\n");

        Ok(sql)
    }

    /// Generate CREATE TABLE statement from TableState
    fn generate_create_table_from_state(table: &TableState) -> Result<String> {
        let mut sql = format!("CREATE TABLE IF NOT EXISTS {} (\n", table.name);

        // Add columns
        for (i, column) in table.columns.iter().enumerate() {
            sql.push_str(&format!("    {} {}", column.name, column.column_type));

            if i < table.columns.len() - 1 {
                sql.push_str(",\n");
            } else {
                sql.push('\n');
            }
        }

        sql.push_str(");\n");

        Ok(sql)
    }

    /// Make index name unique by prefixing with table name
    fn make_index_name_unique(index_sql: &str, table_name: &str) -> String {
        let mut index_sql = index_sql.to_string();

        if let Some(idx_pos) = index_sql.find("CREATE INDEX ") {
            if let Some(on_pos) = index_sql.find(" ON ") {
                let start = idx_pos + "CREATE INDEX ".len();
                let old_index_name = &index_sql[start..on_pos];
                let new_index_name = format!("{}_{}", table_name, old_index_name);
                index_sql = index_sql.replace(old_index_name, &new_index_name);
            }
        }

        index_sql
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{ColumnDef, EventField, TableSchema};
    use crate::config::{AiConfig, ContractConfig, DatabaseConfig, OpenAiConfig, SpecConfig};
    use std::collections::HashMap;
    use tempfile::TempDir;

    // NOTE: These tests change the current working directory and create temporary files.
    // They use WorkingDirGuard to ensure proper cleanup even if tests panic.
    // The guard automatically restores the original working directory when dropped.

    /// RAII guard to automatically restore the working directory when dropped
    /// This ensures cleanup happens even if tests panic
    struct WorkingDirGuard {
        original_dir: std::path::PathBuf,
    }

    impl WorkingDirGuard {
        fn new(temp_dir: &TempDir) -> Self {
            let original_dir = std::env::current_dir().unwrap();
            std::env::set_current_dir(temp_dir).unwrap();
            Self { original_dir }
        }
    }

    impl Drop for WorkingDirGuard {
        fn drop(&mut self) {
            // Restore original directory - this runs even if test panics
            let _ = std::env::set_current_dir(&self.original_dir);
        }
    }

    /// Helper to create a mock IrGenerationResult for testing
    fn create_mock_ir(table_name: &str, event_name: &str) -> IrGenerationResult {
        IrGenerationResult {
            event_name: event_name.to_string(),
            event_signature: format!("{}(uint256,address)", event_name),
            start_block: 12345678,
            contract_address: "0x1234567890123456789012345678901234567890".to_string(),
            chain: "ethereum".to_string(),
            indexed_fields: vec![
                EventField {
                    name: "amount".to_string(),
                    solidity_type: "uint256".to_string(),
                    rust_type: "String".to_string(),
                    indexed: false,
                },
                EventField {
                    name: "user".to_string(),
                    solidity_type: "address".to_string(),
                    rust_type: "String".to_string(),
                    indexed: true,
                },
            ],
            table_schema: TableSchema {
                table_name: table_name.to_string(),
                columns: vec![
                    ColumnDef {
                        name: "id".to_string(),
                        column_type: "BIGSERIAL PRIMARY KEY".to_string(),
                    },
                    ColumnDef {
                        name: "block_number".to_string(),
                        column_type: "BIGINT NOT NULL".to_string(),
                    },
                    ColumnDef {
                        name: "block_timestamp".to_string(),
                        column_type: "BIGINT NOT NULL".to_string(),
                    },
                    ColumnDef {
                        name: "amount".to_string(),
                        column_type: "NUMERIC(78, 0) NOT NULL".to_string(),
                    },
                    ColumnDef {
                        name: "user".to_string(),
                        column_type: "VARCHAR(42) NOT NULL".to_string(),
                    },
                ],
                indexes: vec![
                    "CREATE INDEX idx_block_number ON {table_name}(block_number)".to_string(),
                    "CREATE INDEX idx_timestamp ON {table_name}(block_timestamp)".to_string(),
                    "CREATE INDEX idx_user ON {table_name}(user)".to_string(),
                ],
            },
            description: "Test endpoint".to_string(),
        }
    }

    /// Helper to create a mock Config for testing
    fn create_mock_config(contracts: Vec<(&str, Vec<&str>)>) -> Config {
        let mut contract_configs = HashMap::new();

        for (contract_name, spec_names) in contracts {
            let specs: Vec<SpecConfig> = spec_names
                .iter()
                .map(|name| SpecConfig {
                    name: name.to_string(),
                    start_block: Some(0),
                    task: "Test task".to_string(),
                })
                .collect();

            contract_configs.insert(
                contract_name.to_string(),
                ContractConfig {
                    chain: "test".to_string(),
                    address: "0x1234".to_string(),
                    abi_path: "test.json".to_string(),
                    specs,
                },
            );
        }

        Config {
            database: DatabaseConfig {
                uri: "postgresql://test:test@localhost:5432/test".to_string(),
            },
            chains: HashMap::new(),
            ai: AiConfig {
                openai: OpenAiConfig {
                    api_key: "test".to_string(),
                    model: "test".to_string(),
                    temperature: 1.0,
                },
            },
            contracts: contract_configs,
            endpoints: Vec::new(),
        }
    }

    // NOTE: These tests have been removed as they tested the old implementation
    // The new schema-diff based implementation is tested through integration tests below
    // and unit tests in schema_state.rs and schema_diff.rs

    #[test]
    fn test_generate_from_ir_creates_migration_file() {
        let temp_dir = TempDir::new().unwrap();
        let _guard = WorkingDirGuard::new(&temp_dir);

        // Create IR files first
        let config = create_mock_config(vec![("TestContract", vec!["Event1"])]);

        // Create IR directory and files
        let ir_dir = Path::new("ir/specs").join("TestContract");
        fs::create_dir_all(&ir_dir).unwrap();

        let ir = create_mock_ir("testcontract_event1", "Event1");
        let ir_json = serde_json::to_string_pretty(&ir).unwrap();
        fs::write(ir_dir.join("Event1.json"), ir_json).unwrap();

        // Generate migration
        let result = Migration::generate_from_ir(&config);

        assert!(result.is_ok(), "Should generate migration successfully");

        // Check that migrations directory was created
        assert!(Path::new("migrations").exists());

        // Check that a migration file was created
        let entries: Vec<_> = fs::read_dir("migrations")
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map_or(false, |ext| ext == "sql"))
            .collect();

        assert_eq!(entries.len(), 1, "Should create exactly one migration file");

        let migration_file = entries[0].path();
        let filename = migration_file.file_name().unwrap().to_str().unwrap();

        // Check filename format: YYYYMMDDHHmmss_initial_schema.sql
        assert!(filename.ends_with("_initial_schema.sql"));
        assert!(filename.len() > 20); // timestamp + description

        // Check file contents
        let contents = fs::read_to_string(&migration_file).unwrap();
        assert!(contents.contains("-- Auto-generated migration from IR"));
        assert!(contents.contains("-- TestContract/Event1"));
        assert!(contents.contains("CREATE TABLE IF NOT EXISTS testcontract_event1"));

        // Check that schema.json was created in migrations directory
        assert!(Path::new("migrations/schema.json").exists());
        // Guard automatically restores directory when dropped
    }

    #[test]
    fn test_generate_from_ir_with_multiple_contracts() {
        let temp_dir = TempDir::new().unwrap();
        let _guard = WorkingDirGuard::new(&temp_dir);

        let config = create_mock_config(vec![
            ("Contract1", vec!["Event1", "Event2"]),
            ("Contract2", vec!["Event3"]),
        ]);

        // Create IR files
        let contracts = vec![
            ("Contract1", "Event1", "contract1_event1"),
            ("Contract1", "Event2", "contract1_event2"),
            ("Contract2", "Event3", "contract2_event3"),
        ];

        for (contract, event, table_name) in contracts {
            let ir_dir = Path::new("ir/specs").join(contract);
            fs::create_dir_all(&ir_dir).unwrap();

            let ir = create_mock_ir(table_name, event);
            let ir_json = serde_json::to_string_pretty(&ir).unwrap();
            fs::write(ir_dir.join(format!("{}.json", event)), ir_json).unwrap();
        }

        // Generate migration
        Migration::generate_from_ir(&config).unwrap();

        // Read migration file
        let migration_files: Vec<_> = fs::read_dir("migrations")
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map_or(false, |ext| ext == "sql"))
            .collect();

        assert_eq!(migration_files.len(), 1);

        let contents = fs::read_to_string(migration_files[0].path()).unwrap();

        // Check all contracts and events are present
        assert!(contents.contains("-- Contract1/Event1"));
        assert!(contents.contains("-- Contract1/Event2"));
        assert!(contents.contains("-- Contract2/Event3"));

        assert!(contents.contains("CREATE TABLE IF NOT EXISTS contract1_event1"));
        assert!(contents.contains("CREATE TABLE IF NOT EXISTS contract1_event2"));
        assert!(contents.contains("CREATE TABLE IF NOT EXISTS contract2_event3"));

        // Clean up IR directories created by test
        let _ = fs::remove_dir_all("ir/specs/Contract1");
        let _ = fs::remove_dir_all("ir/specs/Contract2");
        // Guard automatically restores directory when dropped
    }

    #[test]
    fn test_index_name_uniquification() {
        let temp_dir = TempDir::new().unwrap();
        let _guard = WorkingDirGuard::new(&temp_dir);

        let config = create_mock_config(vec![
            ("Contract1", vec!["Event1"]),
            ("Contract2", vec!["Event1"]), // Same event name, different contract
        ]);

        // Create IR files with same index names
        for contract in ["Contract1", "Contract2"] {
            let ir_dir = Path::new("ir/specs").join(contract);
            fs::create_dir_all(&ir_dir).unwrap();

            let table_name = format!("{}_event1", contract.to_lowercase());
            let ir = create_mock_ir(&table_name, "Event1");
            let ir_json = serde_json::to_string_pretty(&ir).unwrap();
            fs::write(ir_dir.join("Event1.json"), ir_json).unwrap();
        }

        // Generate migration
        Migration::generate_from_ir(&config).unwrap();

        // Read migration file
        let migration_files: Vec<_> = fs::read_dir("migrations")
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();

        let contents = fs::read_to_string(migration_files[0].path()).unwrap();

        // Check that index names are prefixed with table names to avoid collisions
        assert!(contents.contains("CREATE INDEX contract1_event1_idx_block_number"));
        assert!(contents.contains("CREATE INDEX contract1_event1_idx_timestamp"));
        assert!(contents.contains("CREATE INDEX contract1_event1_idx_user"));

        assert!(contents.contains("CREATE INDEX contract2_event1_idx_block_number"));
        assert!(contents.contains("CREATE INDEX contract2_event1_idx_timestamp"));
        assert!(contents.contains("CREATE INDEX contract2_event1_idx_user"));

        // Ensure no generic index names that would collide
        assert!(!contents.contains("CREATE INDEX idx_block_number ON"));
        assert!(!contents.contains("CREATE INDEX idx_timestamp ON"));
        // Guard automatically restores directory when dropped
    }

    #[test]
    fn test_migration_sql_syntax() {
        let temp_dir = TempDir::new().unwrap();
        let _guard = WorkingDirGuard::new(&temp_dir);

        let config = create_mock_config(vec![("TestContract", vec!["TestEvent"])]);

        // Create IR file
        let ir_dir = Path::new("ir/specs").join("TestContract");
        fs::create_dir_all(&ir_dir).unwrap();

        let ir = create_mock_ir("testcontract_testevent", "TestEvent");
        let ir_json = serde_json::to_string_pretty(&ir).unwrap();
        fs::write(ir_dir.join("TestEvent.json"), ir_json).unwrap();

        // Generate migration
        Migration::generate_from_ir(&config).unwrap();

        // Read and validate SQL syntax
        let migration_files: Vec<_> = fs::read_dir("migrations")
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map_or(false, |ext| ext == "sql"))
            .collect();

        let contents = fs::read_to_string(migration_files[0].path()).unwrap();

        // Check proper SQL statement termination
        assert!(contents.contains(");\n"), "CREATE TABLE should end with );");

        // Check index statements end with semicolons
        let index_count = contents.matches("CREATE INDEX").count();
        let semicolon_count = contents.matches(";\n").count();
        assert!(
            semicolon_count >= index_count + 1,
            "Each CREATE statement should end with semicolon"
        );

        // Check no placeholder remains
        assert!(
            !contents.contains("{table_name}"),
            "All placeholders should be replaced"
        );

        // Check proper spacing
        assert!(contents.contains("CREATE TABLE IF NOT EXISTS"));
        assert!(contents.contains(" ON "));
        // Guard automatically restores directory when dropped
    }

    #[test]
    fn test_migration_filename_format() {
        let temp_dir = TempDir::new().unwrap();
        let _guard = WorkingDirGuard::new(&temp_dir);

        let config = create_mock_config(vec![("TestContract", vec!["TestEvent"])]);

        // Create IR file
        let ir_dir = Path::new("ir/specs").join("TestContract");
        fs::create_dir_all(&ir_dir).unwrap();

        let ir = create_mock_ir("testcontract_testevent", "TestEvent");
        let ir_json = serde_json::to_string_pretty(&ir).unwrap();
        fs::write(ir_dir.join("TestEvent.json"), ir_json).unwrap();

        // Generate migration
        Migration::generate_from_ir(&config).unwrap();

        // Check filename format
        let entries: Vec<_> = fs::read_dir("migrations")
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map_or(false, |ext| ext == "sql"))
            .collect();

        let filename = entries[0].file_name();
        let filename_str = filename.to_str().unwrap();

        // Check format: YYYYMMDDHHmmss_initial_schema.sql
        let parts: Vec<&str> = filename_str.split('_').collect();

        // First part should be 14-digit timestamp
        assert_eq!(parts[0].len(), 14, "Timestamp should be 14 digits (YYYYMMDDHHmmss)");
        assert!(parts[0].chars().all(|c| c.is_ascii_digit()), "Timestamp should be all digits");

        // Should end with .sql
        assert!(filename_str.ends_with(".sql"));

        // Should contain description
        assert!(filename_str.contains("initial_schema"));
        // Guard automatically restores directory when dropped
    }

    #[test]
    fn test_generate_from_ir_missing_ir_files() {
        let temp_dir = TempDir::new().unwrap();
        let _guard = WorkingDirGuard::new(&temp_dir);

        let config = create_mock_config(vec![("TestContract", vec!["MissingEvent"])]);

        // Don't create IR files - should fail
        let result = Migration::generate_from_ir(&config);

        assert!(result.is_err(), "Should fail when IR files are missing");
        // Guard automatically restores directory when dropped
    }

    #[test]
    fn test_migrations_directory_creation() {
        let temp_dir = TempDir::new().unwrap();
        let _guard = WorkingDirGuard::new(&temp_dir);

        let config = create_mock_config(vec![("TestContract", vec!["TestEvent"])]);

        // Create IR file
        let ir_dir = Path::new("ir/specs").join("TestContract");
        fs::create_dir_all(&ir_dir).unwrap();

        let ir = create_mock_ir("testcontract_testevent", "TestEvent");
        let ir_json = serde_json::to_string_pretty(&ir).unwrap();
        fs::write(ir_dir.join("TestEvent.json"), ir_json).unwrap();

        // Ensure migrations directory doesn't exist
        assert!(!Path::new("migrations").exists());

        // Generate migration
        Migration::generate_from_ir(&config).unwrap();

        // Check that migrations directory was created
        assert!(
            Path::new("migrations").exists(),
            "Migrations directory should be created"
        );
        assert!(
            Path::new("migrations").is_dir(),
            "Migrations should be a directory"
        );
        // Guard automatically restores directory when dropped
    }

    #[test]
    fn test_not_null_column_addition_generates_warning() {
        let temp_dir = TempDir::new().unwrap();
        let _guard = WorkingDirGuard::new(&temp_dir);

        let config = create_mock_config(vec![("TestContract", vec!["TestEvent"])]);

        // Create initial IR and migration
        let ir_dir = Path::new("ir/specs").join("TestContract");
        fs::create_dir_all(&ir_dir).unwrap();

        let mut initial_ir = create_mock_ir("testcontract_testevent", "TestEvent");
        // Remove the "amount" column for initial state
        initial_ir.table_schema.columns.retain(|c| c.name != "amount");
        let ir_json = serde_json::to_string_pretty(&initial_ir).unwrap();
        fs::write(ir_dir.join("TestEvent.json"), ir_json).unwrap();

        // Generate initial migration
        Migration::generate_from_ir(&config).unwrap();

        // Now add the NOT NULL column back
        let updated_ir = create_mock_ir("testcontract_testevent", "TestEvent");
        let ir_json = serde_json::to_string_pretty(&updated_ir).unwrap();
        fs::write(ir_dir.join("TestEvent.json"), ir_json).unwrap();

        // Generate update migration
        Migration::generate_from_ir(&config).unwrap();

        // Find the schema_update migration file
        let migration_files: Vec<_> = fs::read_dir("migrations")
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_str().unwrap().contains("schema_update"))
            .collect();

        assert_eq!(migration_files.len(), 1, "Should have one schema_update migration");

        let contents = fs::read_to_string(migration_files[0].path()).unwrap();

        // Verify warning is present
        assert!(
            contents.contains("-- WARNING: Adding NOT NULL column to existing table with data will fail"),
            "Should contain warning about NOT NULL column"
        );
        assert!(
            contents.contains("-- You must manually decide how to handle this"),
            "Should provide manual intervention instructions"
        );
        assert!(
            contents.contains("-- 1. Add column as nullable first"),
            "Should provide option 1"
        );
        assert!(
            contents.contains("-- 2. Add column with a DEFAULT value"),
            "Should provide option 2"
        );

        // Verify the actual ALTER TABLE commands are commented out
        assert!(
            contents.contains(&format!("--    ALTER TABLE testcontract_testevent ADD COLUMN IF NOT EXISTS amount NUMERIC(78, 0)")),
            "Should have commented ALTER TABLE with nullable column"
        );
        assert!(
            contents.contains("--    UPDATE testcontract_testevent SET amount = <default_value>"),
            "Should have commented UPDATE statement"
        );
        assert!(
            contents.contains("--    ALTER TABLE testcontract_testevent ALTER COLUMN amount SET NOT NULL"),
            "Should have commented SET NOT NULL statement"
        );

        // Guard automatically restores directory when dropped
    }

    #[test]
    fn test_nullable_column_addition_no_warning() {
        let temp_dir = TempDir::new().unwrap();
        let _guard = WorkingDirGuard::new(&temp_dir);

        let config = create_mock_config(vec![("TestContract", vec!["TestEvent"])]);

        // Create initial IR and migration with base columns only
        let ir_dir = Path::new("ir/specs").join("TestContract");
        fs::create_dir_all(&ir_dir).unwrap();

        let mut initial_ir = create_mock_ir("testcontract_testevent", "TestEvent");
        let ir_json = serde_json::to_string_pretty(&initial_ir).unwrap();
        fs::write(ir_dir.join("TestEvent.json"), ir_json).unwrap();

        Migration::generate_from_ir(&config).unwrap();

        // Add a nullable column to the existing schema
        initial_ir.table_schema.columns.push(ColumnDef {
            name: "optional_field".to_string(),
            column_type: "TEXT".to_string(), // No NOT NULL constraint
        });
        let ir_json = serde_json::to_string_pretty(&initial_ir).unwrap();
        fs::write(ir_dir.join("TestEvent.json"), ir_json).unwrap();

        Migration::generate_from_ir(&config).unwrap();

        // Find the schema_update migration file
        let migration_files: Vec<_> = fs::read_dir("migrations")
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_str().unwrap().contains("schema_update"))
            .collect();

        let contents = fs::read_to_string(migration_files[0].path()).unwrap();

        // Verify no warning for nullable column
        assert!(
            contents.contains("ALTER TABLE testcontract_testevent ADD COLUMN IF NOT EXISTS optional_field TEXT;"),
            "Should have uncommented ALTER TABLE for nullable column"
        );
        assert!(
            !contents.contains("-- WARNING: Adding NOT NULL column"),
            "Should not contain NOT NULL warning for nullable column"
        );

        // Guard automatically restores directory when dropped
    }

    #[test]
    fn test_schema_backup_created_on_update() {
        let temp_dir = TempDir::new().unwrap();
        let _guard = WorkingDirGuard::new(&temp_dir);

        let config = create_mock_config(vec![("TestContract", vec!["TestEvent"])]);

        // Create initial IR and migration
        let ir_dir = Path::new("ir/specs").join("TestContract");
        fs::create_dir_all(&ir_dir).unwrap();

        let mut initial_ir = create_mock_ir("testcontract_testevent", "TestEvent");
        let ir_json = serde_json::to_string_pretty(&initial_ir).unwrap();
        fs::write(ir_dir.join("TestEvent.json"), ir_json).unwrap();

        // Generate initial migration
        Migration::generate_from_ir(&config).unwrap();

        // Verify schema.json exists
        assert!(Path::new("migrations/schema.json").exists());

        // Modify IR to trigger an update
        initial_ir.table_schema.columns.push(ColumnDef {
            name: "new_field".to_string(),
            column_type: "TEXT".to_string(),
        });
        let ir_json = serde_json::to_string_pretty(&initial_ir).unwrap();
        fs::write(ir_dir.join("TestEvent.json"), ir_json).unwrap();

        // Generate update migration
        Migration::generate_from_ir(&config).unwrap();

        // Find schema backup files (format: {timestamp}_schema.json)
        let backup_files: Vec<_> = fs::read_dir("migrations")
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                let name = e.file_name();
                let name_str = name.to_str().unwrap();
                name_str.ends_with("_schema.json") && name_str != "schema.json"
            })
            .collect();

        assert_eq!(
            backup_files.len(),
            1,
            "Should create exactly one schema backup"
        );

        let backup_name = backup_files[0].file_name();
        let backup_name_str = backup_name.to_str().unwrap();

        // Verify backup filename format: {timestamp}_schema.json
        assert!(backup_name_str.ends_with("_schema.json"));
        let timestamp_part = backup_name_str.trim_end_matches("_schema.json");
        assert_eq!(
            timestamp_part.len(),
            14,
            "Timestamp should be 14 digits (YYYYMMDDHHmmss)"
        );
        assert!(
            timestamp_part.chars().all(|c| c.is_ascii_digit()),
            "Timestamp should be all digits"
        );

        // Verify the backup contains valid schema data
        let backup_contents = fs::read_to_string(backup_files[0].path()).unwrap();
        assert!(
            serde_json::from_str::<serde_json::Value>(&backup_contents).is_ok(),
            "Backup should contain valid JSON"
        );

        // Guard automatically restores directory when dropped
    }

    #[test]
    fn test_no_backup_on_initial_migration() {
        let temp_dir = TempDir::new().unwrap();
        let _guard = WorkingDirGuard::new(&temp_dir);

        let config = create_mock_config(vec![("TestContract", vec!["TestEvent"])]);

        // Create initial IR
        let ir_dir = Path::new("ir/specs").join("TestContract");
        fs::create_dir_all(&ir_dir).unwrap();

        let ir = create_mock_ir("testcontract_testevent", "TestEvent");
        let ir_json = serde_json::to_string_pretty(&ir).unwrap();
        fs::write(ir_dir.join("TestEvent.json"), ir_json).unwrap();

        // Generate initial migration
        Migration::generate_from_ir(&config).unwrap();

        // Verify no backup files exist (only schema.json should exist)
        let backup_files: Vec<_> = fs::read_dir("migrations")
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                let name = e.file_name();
                let name_str = name.to_str().unwrap();
                name_str.ends_with("_schema.json") && name_str != "schema.json"
            })
            .collect();

        assert_eq!(
            backup_files.len(),
            0,
            "Should not create backup on initial migration"
        );

        // Verify schema.json exists
        assert!(Path::new("migrations/schema.json").exists());

        // Guard automatically restores directory when dropped
    }

    #[test]
    fn test_schema_backup_timestamp_matches_migration() {
        let temp_dir = TempDir::new().unwrap();
        let _guard = WorkingDirGuard::new(&temp_dir);

        let config = create_mock_config(vec![("TestContract", vec!["TestEvent"])]);

        // Create initial IR and migration
        let ir_dir = Path::new("ir/specs").join("TestContract");
        fs::create_dir_all(&ir_dir).unwrap();

        let mut initial_ir = create_mock_ir("testcontract_testevent", "TestEvent");
        let ir_json = serde_json::to_string_pretty(&initial_ir).unwrap();
        fs::write(ir_dir.join("TestEvent.json"), ir_json).unwrap();

        Migration::generate_from_ir(&config).unwrap();

        // Modify IR
        initial_ir.table_schema.columns.push(ColumnDef {
            name: "new_field".to_string(),
            column_type: "TEXT".to_string(),
        });
        let ir_json = serde_json::to_string_pretty(&initial_ir).unwrap();
        fs::write(ir_dir.join("TestEvent.json"), ir_json).unwrap();

        // Generate update migration
        Migration::generate_from_ir(&config).unwrap();

        // Find the migration file
        let migration_files: Vec<_> = fs::read_dir("migrations")
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_str().unwrap().contains("schema_update.sql"))
            .collect();

        let migration_name = migration_files[0].file_name();
        let migration_timestamp = migration_name
            .to_str()
            .unwrap()
            .split('_')
            .next()
            .unwrap();

        // Find the backup file
        let backup_files: Vec<_> = fs::read_dir("migrations")
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                let name = e.file_name();
                let name_str = name.to_str().unwrap();
                name_str.ends_with("_schema.json") && name_str != "schema.json"
            })
            .collect();

        let backup_name = backup_files[0].file_name();
        let backup_timestamp = backup_name
            .to_str()
            .unwrap()
            .trim_end_matches("_schema.json");

        // Verify timestamps match
        assert_eq!(
            migration_timestamp, backup_timestamp,
            "Migration and backup timestamps should match"
        );

        // Guard automatically restores directory when dropped
    }
}
