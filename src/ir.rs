use crate::ai::{AiClient, IrGenerationResult};
use crate::config::{Config, ContractConfig, SpecConfig};
use anyhow::{Context, Result};
use serde_json::Value;
use std::fs;
use std::path::Path;

pub struct Ir {
    ai_client: AiClient,
}

impl Ir {
    pub fn new(ai_client: AiClient) -> Self {
        Self { ai_client }
    }

    /// Generate IR for all contracts in the config
    pub async fn generate_all(&self, config: &Config) -> Result<()> {
        tracing::info!("Starting IR generation for all contracts");

        for (contract_name, contract_config) in &config.contracts {
            tracing::info!("Generating IR for contract: {}", contract_name);
            self.generate_contract(contract_name, contract_config)
                .await?;
        }

        tracing::info!("IR generation complete");
        Ok(())
    }

    /// Generate IR for a specific contract
    async fn generate_contract(
        &self,
        contract_name: &str,
        contract_config: &ContractConfig,
    ) -> Result<()> {
        // Load ABI
        let abi_content = fs::read_to_string(&contract_config.abi_path).context(format!(
            "Failed to read ABI file: {}",
            contract_config.abi_path
        ))?;

        let abi: Value = serde_json::from_str(&abi_content).context("Failed to parse ABI JSON")?;

        // Generate IR for each spec
        for spec in &contract_config.specs {
            tracing::info!("  Generating spec: {}", spec.name);
            let ir = self.generate_spec(contract_name, spec, &abi).await?;

            // Save IR to file
            self.save_ir(contract_name, spec, &ir)?;
        }

        Ok(())
    }

    /// Generate IR for a single spec
    async fn generate_spec(
        &self,
        contract_name: &str,
        spec: &SpecConfig,
        abi: &Value,
    ) -> Result<IrGenerationResult> {
        let ir = self
            .ai_client
            .generate_ir(contract_name, &spec.name, abi, &spec.task)
            .await
            .context(format!("Failed to generate IR for spec: {}", spec.name))?;

        Ok(ir)
    }

    /// Save IR to file in the ir/ directory
    fn save_ir(
        &self,
        contract_name: &str,
        spec: &SpecConfig,
        ir: &IrGenerationResult,
    ) -> Result<()> {
        // Create ir directory if it doesn't exist
        let ir_dir = Path::new("ir");
        if !ir_dir.exists() {
            fs::create_dir_all(ir_dir).context("Failed to create ir directory")?;
        }

        // Create subdirectory for contract
        let contract_dir = ir_dir.join(contract_name);
        if !contract_dir.exists() {
            fs::create_dir_all(&contract_dir).context(format!(
                "Failed to create contract directory: {}",
                contract_name
            ))?;
        }

        // Save IR as JSON
        let ir_file = contract_dir.join(format!("{}.json", spec.name));
        let ir_json = serde_json::to_string_pretty(ir).context("Failed to serialize IR")?;

        fs::write(&ir_file, ir_json).context(format!("Failed to write IR file: {:?}", ir_file))?;

        tracing::info!("    Saved IR to: {:?}", ir_file);

        Ok(())
    }

    /// Load IR from file
    pub fn load_ir(contract_name: &str, spec_name: &str) -> Result<IrGenerationResult> {
        let ir_file = Path::new("ir")
            .join(contract_name)
            .join(format!("{}.json", spec_name));

        let ir_content = fs::read_to_string(&ir_file)
            .context(format!("Failed to read IR file: {:?}", ir_file))?;

        let ir: IrGenerationResult =
            serde_json::from_str(&ir_content).context("Failed to parse IR JSON")?;

        Ok(ir)
    }

    /// Load all IR files
    pub fn load_all_ir(config: &Config) -> Result<Vec<(String, String, IrGenerationResult)>> {
        let mut results = Vec::new();

        for (contract_name, contract_config) in &config.contracts {
            for spec in &contract_config.specs {
                let ir = Self::load_ir(contract_name, &spec.name)?;
                results.push((contract_name.clone(), spec.name.clone(), ir));
            }
        }

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{ColumnDef, EventField, QueryParam, TableSchema};
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
    fn create_mock_ir() -> IrGenerationResult {
        IrGenerationResult {
            event_name: "TestEvent".to_string(),
            event_signature: "TestEvent(uint256,address)".to_string(),
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
                table_name: "test_contract_test_event".to_string(),
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
                ],
            },
            query_params: vec![
                QueryParam {
                    name: "limit".to_string(),
                    param_type: "u64".to_string(),
                    default: Some(serde_json::json!("100")),
                },
                QueryParam {
                    name: "offset".to_string(),
                    param_type: "u64".to_string(),
                    default: Some(serde_json::json!("0")),
                },
            ],
            endpoint_description: "Get test events".to_string(),
        }
    }

    /// Helper to create a mock AiClient for testing (no-op)
    fn create_mock_ai_client() -> AiClient {
        AiClient::new(
            "test-api-key".to_string(),
            "test-model".to_string(),
            1.0,
        )
    }

    /// Helper to create a mock SpecConfig
    fn create_mock_spec(name: &str) -> SpecConfig {
        SpecConfig {
            name: name.to_string(),
            start_block: 0,
            endpoint: format!("/test/{}", name),
            task: "Test task".to_string(),
        }
    }

    #[test]
    fn test_save_and_load_ir() {
        // Create a temporary directory for the test
        let temp_dir = TempDir::new().unwrap();
        let _guard = WorkingDirGuard::new(&temp_dir);

        // Create IR instance with mock AI client
        let ai_client = create_mock_ai_client();
        let ir_generator = Ir::new(ai_client);

        // Create mock data
        let contract_name = "TestContract";
        let spec = create_mock_spec("TestEvent");
        let mock_ir = create_mock_ir();

        // Test save_ir
        ir_generator
            .save_ir(contract_name, &spec, &mock_ir)
            .expect("Failed to save IR");

        // Verify file was created
        let ir_file = Path::new("ir")
            .join(contract_name)
            .join(format!("{}.json", spec.name));
        assert!(ir_file.exists(), "IR file should exist");

        // Test load_ir
        let loaded_ir = Ir::load_ir(contract_name, &spec.name).expect("Failed to load IR");

        // Verify loaded data matches original
        assert_eq!(loaded_ir.event_name, mock_ir.event_name);
        assert_eq!(loaded_ir.event_signature, mock_ir.event_signature);
        assert_eq!(loaded_ir.indexed_fields.len(), mock_ir.indexed_fields.len());
        assert_eq!(
            loaded_ir.table_schema.table_name,
            mock_ir.table_schema.table_name
        );
        assert_eq!(
            loaded_ir.table_schema.columns.len(),
            mock_ir.table_schema.columns.len()
        );
        assert_eq!(
            loaded_ir.endpoint_description,
            mock_ir.endpoint_description
        );
        // Guard automatically restores directory when dropped
    }

    #[test]
    fn test_load_ir_nonexistent_file() {
        let temp_dir = TempDir::new().unwrap();
        let _guard = WorkingDirGuard::new(&temp_dir);

        // Try to load non-existent IR
        let result = Ir::load_ir("NonExistentContract", "NonExistentSpec");

        // Should return an error
        assert!(result.is_err(), "Should fail when loading non-existent IR");
        // Guard automatically restores directory when dropped
    }

    #[test]
    fn test_load_all_ir() {
        let temp_dir = TempDir::new().unwrap();
        let _guard = WorkingDirGuard::new(&temp_dir);

        // Create IR instance with mock AI client
        let ai_client = create_mock_ai_client();
        let ir_generator = Ir::new(ai_client);

        // Create multiple mock IR files
        let contracts = vec![
            ("Contract1", vec!["Event1", "Event2"]),
            ("Contract2", vec!["Event3"]),
        ];

        for (contract_name, spec_names) in &contracts {
            for spec_name in spec_names {
                let spec = create_mock_spec(spec_name);
                let mut mock_ir = create_mock_ir();
                mock_ir.event_name = spec_name.to_string();
                ir_generator
                    .save_ir(contract_name, &spec, &mock_ir)
                    .expect("Failed to save IR");
            }
        }

        // Create a Config object
        let mut contract_configs = HashMap::new();
        for (contract_name, spec_names) in &contracts {
            let specs: Vec<SpecConfig> = spec_names
                .iter()
                .map(|name| create_mock_spec(name))
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

        let config = Config {
            database: crate::config::DatabaseConfig {
                uri: "test".to_string(),
            },
            chains: HashMap::new(),
            ai: crate::config::AiConfig {
                openai: crate::config::OpenAiConfig {
                    api_key: "test".to_string(),
                    model: "test".to_string(),
                    temperature: 1.0,
                },
            },
            contracts: contract_configs,
        };

        // Test load_all_ir
        let results = Ir::load_all_ir(&config).expect("Failed to load all IR");

        // Verify we loaded all IR files
        assert_eq!(results.len(), 3, "Should load 3 IR files total");

        // Verify the contract/spec names are correct
        let loaded_names: Vec<(String, String)> = results
            .iter()
            .map(|(contract, spec, _)| (contract.clone(), spec.clone()))
            .collect();

        assert!(loaded_names.contains(&("Contract1".to_string(), "Event1".to_string())));
        assert!(loaded_names.contains(&("Contract1".to_string(), "Event2".to_string())));
        assert!(loaded_names.contains(&("Contract2".to_string(), "Event3".to_string())));
        // Guard automatically restores directory when dropped
    }

    #[test]
    fn test_save_ir_creates_directories() {
        let temp_dir = TempDir::new().unwrap();
        let _guard = WorkingDirGuard::new(&temp_dir);

        // Create IR instance
        let ai_client = create_mock_ai_client();
        let ir_generator = Ir::new(ai_client);

        let contract_name = "NewContract";
        let spec = create_mock_spec("NewEvent");
        let mock_ir = create_mock_ir();

        // Save IR (should create directories if they don't exist)
        ir_generator
            .save_ir(contract_name, &spec, &mock_ir)
            .expect("Failed to save IR");

        // Verify directories were created
        assert!(Path::new("ir").exists(), "ir directory should exist");
        assert!(
            Path::new("ir").join(contract_name).exists(),
            "Contract directory should exist"
        );
        // Guard automatically restores directory when dropped
    }

    #[test]
    fn test_ir_serialization_roundtrip() {
        let temp_dir = TempDir::new().unwrap();
        let _guard = WorkingDirGuard::new(&temp_dir);

        let ai_client = create_mock_ai_client();
        let ir_generator = Ir::new(ai_client);

        let contract_name = "SerializationTest";
        let spec = create_mock_spec("SerializationEvent");
        let original_ir = create_mock_ir();

        // Save and load
        ir_generator
            .save_ir(contract_name, &spec, &original_ir)
            .expect("Failed to save IR");
        let loaded_ir = Ir::load_ir(contract_name, &spec.name).expect("Failed to load IR");

        // Check specific fields for exact equality
        assert_eq!(loaded_ir.event_name, original_ir.event_name);
        assert_eq!(loaded_ir.event_signature, original_ir.event_signature);

        // Check indexed fields
        for (loaded_field, original_field) in loaded_ir
            .indexed_fields
            .iter()
            .zip(original_ir.indexed_fields.iter())
        {
            assert_eq!(loaded_field.name, original_field.name);
            assert_eq!(loaded_field.solidity_type, original_field.solidity_type);
            assert_eq!(loaded_field.rust_type, original_field.rust_type);
            assert_eq!(loaded_field.indexed, original_field.indexed);
        }

        // Check table schema
        assert_eq!(
            loaded_ir.table_schema.table_name,
            original_ir.table_schema.table_name
        );
        for (loaded_col, original_col) in loaded_ir
            .table_schema
            .columns
            .iter()
            .zip(original_ir.table_schema.columns.iter())
        {
            assert_eq!(loaded_col.name, original_col.name);
            assert_eq!(loaded_col.column_type, original_col.column_type);
        }
        // Guard automatically restores directory when dropped
    }
}
