use anyhow::{Context, Result};
use async_openai::{
    config::OpenAIConfig,
    types::{
        ChatCompletionRequestMessage, ChatCompletionRequestSystemMessageArgs,
        ChatCompletionRequestUserMessageArgs, CreateChatCompletionRequestArgs,
    },
    Client,
};
use serde_json::Value;

pub struct AiClient {
    client: Client<OpenAIConfig>,
    model: String,
    temperature: f32,
}

impl AiClient {
    pub fn new(api_key: String, model: String, temperature: f32) -> Self {
        let config = OpenAIConfig::new().with_api_key(api_key);
        let client = Client::with_config(config);

        Self {
            client,
            model,
            temperature,
        }
    }

    /// Generate IR (Intermediate Representation) for an event spec
    pub async fn generate_ir(
        &self,
        contract_name: &str,
        spec_name: &str,
        abi: &Value,
        task_description: &str,
    ) -> Result<IrGenerationResult> {
        let system_prompt = r#"You are an expert Ethereum indexer code generator.
Given a contract ABI and a natural language task description, you will:

1. Analyze the ABI to find the relevant event
2. Extract event parameters and their types
3. Determine which fields to index based on the task
4. Generate a JSON schema for the database table
5. Identify query parameters and filters needed for the API endpoint

Return your response in the following JSON format:
{
  "event_name": "EventName",
  "event_signature": "EventName(uint256,address)",
  "indexed_fields": [
    {"name": "field1", "solidity_type": "uint256", "rust_type": "String", "indexed": false},
    {"name": "field2", "solidity_type": "address", "rust_type": "String", "indexed": true}
  ],
  "table_schema": {
    "table_name": "event_table_name",
    "columns": [
      {"name": "id", "type": "BIGSERIAL PRIMARY KEY"},
      {"name": "block_number", "type": "BIGINT NOT NULL"},
      {"name": "block_timestamp", "type": "BIGINT NOT NULL"},
      {"name": "transaction_hash", "type": "VARCHAR(66) NOT NULL"},
      {"name": "log_index", "type": "INTEGER NOT NULL"},
      {"name": "field1", "type": "NUMERIC(78, 0) NOT NULL"},
      {"name": "field2", "type": "VARCHAR(42) NOT NULL"}
    ],
    "indexes": [
      "CREATE INDEX idx_block_number ON {table_name}(block_number)",
      "CREATE INDEX idx_timestamp ON {table_name}(block_timestamp)"
    ]
  },
  "query_params": [
    {"name": "limit", "type": "u64", "default": "100"},
    {"name": "offset", "type": "u64", "default": "0"},
    {"name": "startBlockNumber", "type": "Option<u64>", "default": null},
    {"name": "endBlockNumber", "type": "Option<u64>", "default": null},
    {"name": "startTimestamp", "type": "Option<i64>", "default": null},
    {"name": "endTimestamp", "type": "Option<i64>", "default": null}
  ],
  "endpoint_description": "Get time series of fee updates with optional filtering"
}

Important Solidity to PostgreSQL type mappings:
- uint8 -> SMALLINT
- uint16 -> INTEGER
- uint24 -> INTEGER
- uint32 -> BIGINT
- uint40, uint48, uint56, uint64 -> BIGINT
- uint72, uint80, uint88, uint96, uint104, uint112, uint120, uint128 -> NUMERIC(39, 0)
- uint136, uint144, uint152, uint160, uint168, uint176, uint184, uint192, uint200, uint208, uint216, uint224, uint232, uint240, uint248, uint256 -> NUMERIC(78, 0)
- address -> VARCHAR(42)
- bytes1-bytes32 -> VARCHAR(66)
- bytes (dynamic) -> TEXT
- string -> TEXT
- bool -> BOOLEAN

For indexed event parameters, note them in the response but they don't need special database treatment."#;

        let user_prompt = format!(
            r#"Contract: {}
Spec Name: {}

ABI:
{}

Task Description:
{}

Please generate the IR for this indexing specification."#,
            contract_name,
            spec_name,
            serde_json::to_string_pretty(abi)?,
            task_description
        );

        let messages = vec![
            ChatCompletionRequestMessage::System(
                ChatCompletionRequestSystemMessageArgs::default()
                    .content(system_prompt)
                    .build()?,
            ),
            ChatCompletionRequestMessage::User(
                ChatCompletionRequestUserMessageArgs::default()
                    .content(user_prompt)
                    .build()?,
            ),
        ];

        let request = CreateChatCompletionRequestArgs::default()
            .model(&self.model)
            .messages(messages)
            .temperature(self.temperature)
            .build()?;

        let response = self
            .client
            .chat()
            .create(request)
            .await
            .context("Failed to call OpenAI API")?;

        let content = response
            .choices
            .first()
            .and_then(|choice| choice.message.content.as_ref())
            .context("No response from AI")?;

        // Parse JSON from response (handle markdown code blocks if present)
        let json_str = if content.contains("```json") {
            content
                .split("```json")
                .nth(1)
                .and_then(|s| s.split("```").next())
                .unwrap_or(content)
                .trim()
        } else if content.contains("```") {
            content
                .split("```")
                .nth(1)
                .and_then(|s| s.split("```").next())
                .unwrap_or(content)
                .trim()
        } else {
            content.trim()
        };

        let ir: IrGenerationResult = serde_json::from_str(json_str)
            .context("Failed to parse AI response as JSON")?;

        Ok(ir)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IrGenerationResult {
    pub event_name: String,
    pub event_signature: String,
    pub indexed_fields: Vec<EventField>,
    pub table_schema: TableSchema,
    pub query_params: Vec<QueryParam>,
    pub endpoint_description: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EventField {
    pub name: String,
    pub solidity_type: String,
    pub rust_type: String,
    pub indexed: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TableSchema {
    pub table_name: String,
    pub columns: Vec<ColumnDef>,
    pub indexes: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ColumnDef {
    pub name: String,
    #[serde(rename = "type")]
    pub column_type: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QueryParam {
    pub name: String,
    #[serde(rename = "type")]
    pub param_type: String,
    pub default: Option<serde_json::Value>,
}