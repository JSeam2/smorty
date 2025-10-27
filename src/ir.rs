use crate::ai::{AiClient, EndpointIrResult, IrGenerationResult};
use crate::config::{Config, ContractConfig, EndpointConfig, SpecConfig};
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
            let ir = self.generate_spec(contract_name, &contract_config, spec, &abi).await?;

            // Save spec IR to file
            self.save_ir_spec(contract_name, spec, &ir)?;
        }

        Ok(())
    }

    /// Generate IR for a single spec
    async fn generate_spec(
        &self,
        contract_name: &str,
        contract: &ContractConfig,
        spec: &SpecConfig,
        abi: &Value,
    ) -> Result<IrGenerationResult> {
        let ir = self
            .ai_client
            .generate_ir(
                contract_name,
                &spec.name,
                spec.start_block,
                contract.address.as_str(),
                contract.chain.as_str(),
                abi,
                &spec.task
            )
            .await
            .context(format!("Failed to generate IR for spec: {}", spec.name))?;

        Ok(ir)
    }

    /// Save spec IR to file in the ir/specs/ directory
    fn save_ir_spec(
        &self,
        contract_name: &str,
        spec: &SpecConfig,
        ir: &IrGenerationResult,
    ) -> Result<()> {
        self.save_ir_spec_to_dir(Path::new("ir/specs"), contract_name, spec, ir)
    }

    /// Save spec IR to a specific directory (used for testing)
    fn save_ir_spec_to_dir(
        &self,
        base_dir: &Path,
        contract_name: &str,
        spec: &SpecConfig,
        ir: &IrGenerationResult,
    ) -> Result<()> {
        // Create ir directory if it doesn't exist
        if !base_dir.exists() {
            fs::create_dir_all(base_dir).context("Failed to create ir directory")?;
        }

        // Create subdirectory for contract
        let contract_dir = base_dir.join(contract_name);
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

    /// Load spec IR from file in the ir/specs/ directory
    pub fn load_ir_spec(contract_name: &str, spec_name: &str) -> Result<IrGenerationResult> {
        let ir_file = Path::new("ir/specs")
            .join(contract_name)
            .join(format!("{}.json", spec_name));

        let ir_content = fs::read_to_string(&ir_file)
            .context(format!("Failed to read IR file: {:?}", ir_file))?;

        let ir: IrGenerationResult =
            serde_json::from_str(&ir_content).context("Failed to parse IR JSON")?;

        Ok(ir)
    }

    /// Load all spec IR files
    pub fn load_all_ir_specs(config: &Config) -> Result<Vec<(String, String, IrGenerationResult)>> {
        let mut results = Vec::new();

        for (contract_name, contract_config) in &config.contracts {
            for spec in &contract_config.specs {
                let ir = Self::load_ir_spec(contract_name, &spec.name)?;
                results.push((contract_name.clone(), spec.name.clone(), ir));
            }
        }

        Ok(results)
    }

    /// Generate IR for all endpoints in the config
    pub async fn generate_all_endpoints(&self, config: &Config) -> Result<()> {
        tracing::info!("Starting endpoint IR generation");

        // First, load all spec IR to provide context to the endpoint generator
        let spec_irs = Self::load_all_ir_specs(config)?;
        let spec_irs_ref: Vec<_> = spec_irs.iter().map(|(_, _, ir)| ir.clone()).collect();

        for (index, endpoint_config) in config.endpoints.iter().enumerate() {
            tracing::info!(
                "Generating endpoint IR {}/{}: {}",
                index + 1,
                config.endpoints.len(),
                endpoint_config.endpoint
            );
            self.generate_endpoint(&endpoint_config, &spec_irs_ref)
                .await?;
        }

        tracing::info!("Endpoint IR generation complete");
        Ok(())
    }

    /// Generate IR for a single endpoint
    async fn generate_endpoint(
        &self,
        endpoint_config: &EndpointConfig,
        available_tables: &[IrGenerationResult],
    ) -> Result<()> {
        let endpoint_ir = self
            .ai_client
            .generate_endpoint_ir(
                &endpoint_config.endpoint,
                &endpoint_config.description,
                &endpoint_config.task,
                available_tables,
            )
            .await
            .context(format!(
                "Failed to generate endpoint IR for: {}",
                endpoint_config.endpoint
            ))?;

        // Save endpoint IR to file
        self.save_ir_endpoint(&endpoint_ir)?;

        Ok(())
    }

    /// Save endpoint IR to file in the ir/endpoints/ directory
    fn save_ir_endpoint(&self, ir: &EndpointIrResult) -> Result<()> {
        self.save_ir_endpoint_to_dir(Path::new("ir/endpoints"), ir)
    }

    /// Save endpoint IR to a specific directory (used for testing)
    fn save_ir_endpoint_to_dir(&self, base_dir: &Path, ir: &EndpointIrResult) -> Result<()> {
        // Create ir/endpoints directory if it doesn't exist
        if !base_dir.exists() {
            fs::create_dir_all(base_dir).context("Failed to create ir/endpoints directory")?;
        }

        // Convert endpoint path to filename
        // e.g., "/api/pool/{pool}/fees" -> "api_pool_fees"
        let filename = ir
            .endpoint_path
            .trim_start_matches('/')
            .replace('/', "_")
            .replace('{', "")
            .replace('}', "");

        // Save IR as JSON
        let ir_file = base_dir.join(format!("{}.json", filename));
        let ir_json = serde_json::to_string_pretty(ir).context("Failed to serialize endpoint IR")?;

        fs::write(&ir_file, ir_json)
            .context(format!("Failed to write endpoint IR file: {:?}", ir_file))?;

        tracing::info!("  Saved endpoint IR to: {:?}", ir_file);

        Ok(())
    }

    /// Load endpoint IR from file in the ir/endpoints/ directory
    pub fn load_ir_endpoint(endpoint_path: &str) -> Result<EndpointIrResult> {
        // Convert endpoint path to filename
        let filename = endpoint_path
            .trim_start_matches('/')
            .replace('/', "_")
            .replace('{', "")
            .replace('}', "");

        let ir_file = Path::new("ir/endpoints").join(format!("{}.json", filename));

        let ir_content = fs::read_to_string(&ir_file)
            .context(format!("Failed to read endpoint IR file: {:?}", ir_file))?;

        let ir: EndpointIrResult =
            serde_json::from_str(&ir_content).context("Failed to parse endpoint IR JSON")?;

        Ok(ir)
    }

    /// Load all endpoint IR files
    pub fn load_all_ir_endpoints() -> Result<Vec<EndpointIrResult>> {
        let endpoints_dir = Path::new("ir/endpoints");

        if !endpoints_dir.exists() {
            return Ok(Vec::new());
        }

        let mut results = Vec::new();

        for entry in fs::read_dir(endpoints_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                let ir_content = fs::read_to_string(&path)
                    .context(format!("Failed to read endpoint IR file: {:?}", path))?;

                let ir: EndpointIrResult =
                    serde_json::from_str(&ir_content).context("Failed to parse endpoint IR JSON")?;

                results.push(ir);
            }
        }

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{ColumnDef, EventField, TableSchema};
    use tempfile::TempDir;

    // NOTE: These tests use temporary directories to avoid interfering with the actual ir/ directory

    /// Helper to create a mock IrGenerationResult for testing
    fn create_mock_ir() -> IrGenerationResult {
        IrGenerationResult {
            event_name: "TestEvent".to_string(),
            event_signature: "TestEvent(uint256,address)".to_string(),
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
            description: "Get test events".to_string(),
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
            start_block: Some(0),
            task: "Test task".to_string(),
        }
    }

    #[test]
    fn test_save_and_load_ir() {
        // Create a temporary directory for the test
        let temp_dir = TempDir::new().unwrap();
        let ir_dir = temp_dir.path().join("ir");

        // Create IR instance with mock AI client
        let ai_client = create_mock_ai_client();
        let ir_generator = Ir::new(ai_client);

        // Create mock data
        let contract_name = "TestContract";
        let spec = create_mock_spec("TestEvent");
        let mock_ir = create_mock_ir();

        // Test save_ir_spec_to_dir
        ir_generator
            .save_ir_spec_to_dir(&ir_dir, contract_name, &spec, &mock_ir)
            .expect("Failed to save IR");

        // Verify file was created
        let ir_file = ir_dir
            .join(contract_name)
            .join(format!("{}.json", spec.name));
        assert!(ir_file.exists(), "IR file should exist");

        // Load and verify data
        let ir_content = fs::read_to_string(&ir_file).expect("Failed to read IR file");
        let loaded_ir: IrGenerationResult =
            serde_json::from_str(&ir_content).expect("Failed to parse IR JSON");

        // Verify loaded data matches original
        assert_eq!(loaded_ir.event_name, mock_ir.event_name);
        assert_eq!(loaded_ir.event_signature, mock_ir.event_signature);
        assert_eq!(loaded_ir.start_block, mock_ir.start_block);
        assert_eq!(loaded_ir.contract_address, mock_ir.contract_address);
        assert_eq!(loaded_ir.chain, mock_ir.chain);
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
            loaded_ir.description,
            mock_ir.description
        );
    }

    #[test]
    fn test_load_ir_spec_nonexistent_file() {
        // Try to load non-existent IR
        let result = Ir::load_ir_spec("NonExistentContract", "NonExistentSpec");

        // Should return an error
        assert!(result.is_err(), "Should fail when loading non-existent IR");
    }

    #[test]
    fn test_load_all_ir_specs() {
        let temp_dir = TempDir::new().unwrap();
        let ir_dir = temp_dir.path().join("ir");

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
                    .save_ir_spec_to_dir(&ir_dir, contract_name, &spec, &mock_ir)
                    .expect("Failed to save IR");
            }
        }

        // Verify files were created and can be loaded individually
        for (contract_name, spec_names) in &contracts {
            for spec_name in spec_names {
                let ir_file = ir_dir
                    .join(contract_name)
                    .join(format!("{}.json", spec_name));
                assert!(ir_file.exists(), "IR file should exist for {}/{}", contract_name, spec_name);

                // Load and verify
                let ir_content = fs::read_to_string(&ir_file).expect("Failed to read IR file");
                let loaded_ir: IrGenerationResult =
                    serde_json::from_str(&ir_content).expect("Failed to parse IR JSON");
                assert_eq!(loaded_ir.event_name, *spec_name);
            }
        }
    }

    #[test]
    fn test_save_ir_spec_creates_directories() {
        let temp_dir = TempDir::new().unwrap();
        let ir_dir = temp_dir.path().join("ir");

        // Create IR instance
        let ai_client = create_mock_ai_client();
        let ir_generator = Ir::new(ai_client);

        let contract_name = "NewContract";
        let spec = create_mock_spec("NewEvent");
        let mock_ir = create_mock_ir();

        // Save IR (should create directories if they don't exist)
        ir_generator
            .save_ir_spec_to_dir(&ir_dir, contract_name, &spec, &mock_ir)
            .expect("Failed to save IR");

        // Verify directories were created
        assert!(ir_dir.exists(), "ir directory should exist");
        assert!(
            ir_dir.join(contract_name).exists(),
            "Contract directory should exist"
        );
    }

    #[test]
    fn test_ir_serialization_roundtrip() {
        let temp_dir = TempDir::new().unwrap();
        let ir_dir = temp_dir.path().join("ir");

        let ai_client = create_mock_ai_client();
        let ir_generator = Ir::new(ai_client);

        let contract_name = "SerializationTest";
        let spec = create_mock_spec("SerializationEvent");
        let original_ir = create_mock_ir();

        // Save
        ir_generator
            .save_ir_spec_to_dir(&ir_dir, contract_name, &spec, &original_ir)
            .expect("Failed to save IR");

        // Load
        let ir_file = ir_dir
            .join(contract_name)
            .join(format!("{}.json", spec.name));
        let ir_content = fs::read_to_string(&ir_file).expect("Failed to read IR file");
        let loaded_ir: IrGenerationResult =
            serde_json::from_str(&ir_content).expect("Failed to parse IR JSON");

        // Check specific fields for exact equality
        assert_eq!(loaded_ir.event_name, original_ir.event_name);
        assert_eq!(loaded_ir.event_signature, original_ir.event_signature);
        assert_eq!(loaded_ir.start_block, original_ir.start_block);
        assert_eq!(loaded_ir.contract_address, original_ir.contract_address);
        assert_eq!(loaded_ir.chain, original_ir.chain);

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
    }
}
