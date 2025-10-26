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
