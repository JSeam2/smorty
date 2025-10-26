use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub database: DatabaseConfig,
    pub chains: HashMap<String, String>,
    pub ai: AiConfig,
    pub contracts: HashMap<String, ContractConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    pub uri: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiConfig {
    pub openai: OpenAiConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiConfig {
    pub model: String,
    #[serde(rename = "apiKey")]
    pub api_key: String,
    pub temperature: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractConfig {
    pub chain: String,
    pub address: String,
    #[serde(rename = "abiPath")]
    pub abi_path: String,
    pub specs: Vec<SpecConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpecConfig {
    pub name: String,
    #[serde(rename = "startBlock")]
    pub start_block: u64,
    pub endpoint: String,
    pub task: String,
}

impl Config {
    /// Load configuration from a TOML file
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = fs::read_to_string(path.as_ref())
            .context("Failed to read config file")?;

        let config: Config = toml::from_str(&content)
            .context("Failed to parse config TOML")?;

        config.validate()?;

        Ok(config)
    }

    /// Validate the configuration
    fn validate(&self) -> Result<()> {
        // Validate that all contract chains exist in the chains map
        for (contract_name, contract) in &self.contracts {
            if !self.chains.contains_key(&contract.chain) {
                anyhow::bail!(
                    "Contract '{}' references chain '{}' which is not defined in chains section",
                    contract_name,
                    contract.chain
                );
            }

            // Validate that ABI file exists
            if !Path::new(&contract.abi_path).exists() {
                anyhow::bail!(
                    "ABI file '{}' for contract '{}' does not exist",
                    contract.abi_path,
                    contract_name
                );
            }

            // Validate specs
            if contract.specs.is_empty() {
                anyhow::bail!("Contract '{}' has no specs defined", contract_name);
            }
        }

        Ok(())
    }

    /// Get RPC URL for a chain
    pub fn get_rpc_url(&self, chain: &str) -> Result<&String> {
        self.chains.get(chain)
            .ok_or_else(|| anyhow::anyhow!("Chain '{}' not found in config", chain))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_parsing() {
        let toml_str = r#"
[database]
uri = "postgresql://test:test@localhost:5432/test"

[chains]
mainnet = "https://mainnet.example.com"
sonic = "https://sonic.example.com"

[ai.openai]
model = "gpt-4"
apiKey = "sk-test"
temperature = 0.0

[contracts.TestContract]
chain = "sonic"
address = "0x1234567890123456789012345678901234567890"
abiPath = "abi/test.json"

[[contracts.TestContract.specs]]
name = "TestEvent"
startBlock = 1000
endpoint = "/test/event"
task = "Track TestEvent"
"#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.chains.len(), 2);
        assert_eq!(config.contracts.len(), 1);
    }
}
