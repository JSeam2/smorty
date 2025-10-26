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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{ColumnDef, EventField, QueryParam, TableSchema};
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
            query_params: vec![
                QueryParam {
                    name: "limit".to_string(),
                    param_type: "u64".to_string(),
                    default: Some(serde_json::json!("100")),
                },
            ],
            endpoint_description: "Test endpoint".to_string(),
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
                    start_block: 0,
                    endpoint: format!("/test/{}", name),
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
        }
    }

    #[test]
    fn test_generate_create_table() {
        let ir = create_mock_ir("test_table", "TestEvent");
        let result = Migration::generate_create_table(&ir);

        assert!(result.is_ok(), "Should generate CREATE TABLE successfully");

        let sql = result.unwrap();

        // Check that the SQL contains expected elements
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS test_table"));
        assert!(sql.contains("id BIGSERIAL PRIMARY KEY"));
        assert!(sql.contains("block_number BIGINT NOT NULL"));
        assert!(sql.contains("block_timestamp BIGINT NOT NULL"));
        assert!(sql.contains("amount NUMERIC(78, 0) NOT NULL"));
        assert!(sql.contains("user VARCHAR(42) NOT NULL"));
        assert!(sql.ends_with(");\n"));

        // Check column ordering (commas between columns, no comma after last)
        let lines: Vec<&str> = sql.lines().collect();
        assert!(lines[1].ends_with("id BIGSERIAL PRIMARY KEY,"));
        assert!(lines[lines.len() - 2].ends_with("user VARCHAR(42) NOT NULL"));
    }

    #[test]
    fn test_generate_create_table_with_different_columns() {
        let mut ir = create_mock_ir("custom_table", "CustomEvent");

        // Customize columns
        ir.table_schema.columns = vec![
            ColumnDef {
                name: "id".to_string(),
                column_type: "BIGSERIAL PRIMARY KEY".to_string(),
            },
            ColumnDef {
                name: "custom_field".to_string(),
                column_type: "TEXT NOT NULL".to_string(),
            },
        ];

        let sql = Migration::generate_create_table(&ir).unwrap();

        assert!(sql.contains("CREATE TABLE IF NOT EXISTS custom_table"));
        assert!(sql.contains("custom_field TEXT NOT NULL"));
        assert_eq!(sql.matches(',').count(), 1, "Should have exactly one comma");
    }

    #[test]
    fn test_generate_from_ir_creates_migration_file() {
        let temp_dir = TempDir::new().unwrap();
        let _guard = WorkingDirGuard::new(&temp_dir);

        // Create IR files first
        let config = create_mock_config(vec![("TestContract", vec!["Event1"])]);

        // Create IR directory and files
        let ir_dir = Path::new("ir").join("TestContract");
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
            .collect();

        assert_eq!(entries.len(), 1, "Should create exactly one migration file");

        let migration_file = entries[0].path();
        let filename = migration_file.file_name().unwrap().to_str().unwrap();

        // Check filename format: YYYYMMDDHHmmss_auto_generated_from_ir.sql
        assert!(filename.ends_with("_auto_generated_from_ir.sql"));
        assert!(filename.len() > 30); // timestamp + description

        // Check file contents
        let contents = fs::read_to_string(&migration_file).unwrap();
        assert!(contents.contains("-- Auto-generated migration from IR"));
        assert!(contents.contains("-- TestContract/Event1"));
        assert!(contents.contains("CREATE TABLE IF NOT EXISTS testcontract_event1"));
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
            let ir_dir = Path::new("ir").join(contract);
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
            let ir_dir = Path::new("ir").join(contract);
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
        let ir_dir = Path::new("ir").join("TestContract");
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
        let ir_dir = Path::new("ir").join("TestContract");
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
            .collect();

        let filename = entries[0].file_name();
        let filename_str = filename.to_str().unwrap();

        // Check format: YYYYMMDDHHmmss_auto_generated_from_ir.sql
        let parts: Vec<&str> = filename_str.split('_').collect();

        // First part should be 14-digit timestamp
        assert_eq!(parts[0].len(), 14, "Timestamp should be 14 digits (YYYYMMDDHHmmss)");
        assert!(parts[0].chars().all(|c| c.is_ascii_digit()), "Timestamp should be all digits");

        // Should end with .sql
        assert!(filename_str.ends_with(".sql"));

        // Should contain description
        assert!(filename_str.contains("auto_generated_from_ir"));
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
        let ir_dir = Path::new("ir").join("TestContract");
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
}
