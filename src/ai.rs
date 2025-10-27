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

/// Validates and sanitizes SQL queries to catch common syntax errors
fn validate_and_sanitize_sql(sql: &str) -> Result<String> {
    let mut sanitized = sql.to_string();

    // Check for common SQL escaping issues
    if sanitized.contains(r#"\""#) {
        tracing::warn!("Found escaped quotes in SQL, attempting to fix");
        // Replace \" with ' for proper SQL syntax
        sanitized = sanitized.replace(r#"\""#, "'");
    }

    // Check for other common issues
    if sanitized.contains(r#"\'"#) {
        tracing::warn!("Found escaped single quotes, fixing");
        sanitized = sanitized.replace(r#"\'"#, "''"); // SQL standard way to escape single quotes
    }

    // Validate basic SQL structure
    if !sanitized.trim().to_lowercase().starts_with("select")
        && !sanitized.trim().to_lowercase().starts_with("with") {
        anyhow::bail!("SQL query must start with SELECT or WITH");
    }

    // Check for balanced parentheses
    let open_parens = sanitized.matches('(').count();
    let close_parens = sanitized.matches(')').count();
    if open_parens != close_parens {
        anyhow::bail!("Unbalanced parentheses in SQL query: {} open, {} close", open_parens, close_parens);
    }

    // Warn about potential syntax issues
    if sanitized.contains("numeric '") {
        tracing::warn!("Found 'numeric \\'' pattern which might cause issues. Consider using CAST() or ::numeric instead");
    }

    Ok(sanitized)
}

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
        start_block: Option<u64>,
        contract_address: &str,
        chain: &str,
        abi: &Value,
        task_description: &str
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
  "start_block": 12345678,
  "contract_address: "0xContractAddress",
  "chain": "chain_name",
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
      {"name": "field_1", "type": "NUMERIC(78, 0) NOT NULL"},
      {"name": "field_2", "type": "VARCHAR(42) NOT NULL"}
    ],
    "indexes": [
      "CREATE INDEX idx_block_number ON {table_name}(block_number)",
      "CREATE INDEX idx_timestamp ON {table_name}(block_timestamp)"
    ]
  },
  "description": "A brief and concise description of the event to be indexed"
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

For indexed event parameters, note them in the response but they don't need special database treatment.

IMPORTANT: Table naming convention (STRICT):
- ALWAYS use the format: {contract_name}_{spec_name} (in lowercase with underscores)
- Convert any hyphens, camelCase, or special characters to underscores
- Example: Contract "FeeManagerV3_Beets_Sonic_ETHUSD6h" + Spec "PoolUpdated" = "feemanagerv3_beets_sonic_ethusd6h_poolupdated"
- Example: swapFeePercentage to swap_fee_percentage
- DO NOT use generic names like "pool_updated_events" or "fee_updated_events"
- This ensures tables from different contracts never collide, even if they track the same event types."#;

        let sblock = start_block.unwrap_or(0);

        let user_prompt = format!(
            r#"Contract: {}
Spec Name:
{}

Start Block:
{}

Contract Address:
{}

Chain:
{}

ABI:
{}

Task Description:
{}

Please generate the IR for this indexing specification."#,
            contract_name,
            spec_name,
            sblock,
            contract_address,
            chain,
            serde_json::to_string_pretty(abi)?,
            task_description,
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

    /// Generate IR for an API endpoint with retry logic
    pub async fn generate_endpoint_ir(
        &self,
        endpoint_path: &str,
        endpoint_description: &str,
        task_description: &str,
        available_tables: &[IrGenerationResult],
    ) -> Result<EndpointIrResult> {
        const MAX_RETRIES: usize = 3;
        let mut last_error = None;

        for attempt in 1..=MAX_RETRIES {
            tracing::info!("Generating endpoint IR (attempt {}/{})", attempt, MAX_RETRIES);

            let result = self.generate_endpoint_ir_internal(
                endpoint_path,
                endpoint_description,
                task_description,
                available_tables,
                last_error.as_deref(),
            ).await;

            match result {
                Ok(mut endpoint_ir) => {
                    // Validate and sanitize SQL
                    match validate_and_sanitize_sql(&endpoint_ir.sql_query) {
                        Ok(sanitized_sql) => {
                            if sanitized_sql != endpoint_ir.sql_query {
                                tracing::warn!("SQL was sanitized, original had syntax issues");
                                endpoint_ir.sql_query = sanitized_sql;
                            }
                            tracing::info!("Successfully generated and validated endpoint IR");
                            return Ok(endpoint_ir);
                        }
                        Err(e) => {
                            let error_msg = format!("SQL validation failed: {}", e);
                            tracing::warn!("{}, retrying...", error_msg);
                            last_error = Some(error_msg);
                            continue;
                        }
                    }
                }
                Err(e) => {
                    let error_msg = format!("Generation failed: {}", e);
                    tracing::warn!("{}, retrying...", error_msg);
                    last_error = Some(error_msg);
                    continue;
                }
            }
        }

        Err(anyhow::anyhow!(
            "Failed to generate valid endpoint IR after {} attempts. Last error: {}",
            MAX_RETRIES,
            last_error.unwrap_or_else(|| "Unknown error".to_string())
        ))
    }

    /// Internal method to generate endpoint IR
    async fn generate_endpoint_ir_internal(
        &self,
        endpoint_path: &str,
        endpoint_description: &str,
        task_description: &str,
        available_tables: &[IrGenerationResult],
        previous_error: Option<&str>,
    ) -> Result<EndpointIrResult> {
        let system_prompt = r#"You are an expert API endpoint generator for an Ethereum indexer with deep knowledge of PostgreSQL and data analytics.

Given an endpoint path, description, task specification, and available database tables, you will:

1. Analyze the endpoint requirements carefully
2. Extract path parameters from the endpoint (e.g., {pool} in /api/pool/{pool})
3. Determine all necessary query parameters (filtering, pagination, time ranges, etc.)
4. Design and generate the appropriate SQL query to satisfy the requirements
5. Design the response schema that matches the query output
6. Provide utoipa-compatible type information for Rust code generation

## SQL Query Capabilities

You have full access to PostgreSQL features and should use them when appropriate:

**Basic Operations:**
- SELECT with WHERE clauses for filtering
- ORDER BY for sorting (typically by timestamp DESC for time series)
- LIMIT and OFFSET for pagination
- Column aliasing with AS

**Advanced Operations:**
- JOINs (INNER, LEFT, RIGHT, FULL) when combining multiple tables
- Subqueries and CTEs (WITH clauses) for complex logic
- Window functions (ROW_NUMBER, LAG, LEAD, FIRST_VALUE, LAST_VALUE) for time series analytics
- Aggregations (COUNT, SUM, AVG, MIN, MAX) with GROUP BY
- Date/time functions for timestamp filtering and grouping
- CASE statements for conditional logic
- JSON aggregation (json_agg, jsonb_agg) for nested data
- DISTINCT ON for deduplication

**Query Parameters:**
Always parameterize your queries using PostgreSQL numbered parameters ($1, $2, $3, etc.)
Map parameters in the order they appear in the query.

**CRITICAL SQL FORMATTING RULES:**
1. NEVER use backslash escaping for quotes in SQL queries
2. Use single quotes (') for string literals in SQL, not escaped quotes (\")
3. For numeric literals, write them directly without quotes (e.g., numeric '1000000000000000000' is WRONG, use 1000000000000000000::numeric or CAST(1000000000000000000 AS NUMERIC))
4. When the SQL query will be stored in JSON, use proper single quotes that don't require escaping
5. Test your SQL mentally - it should be valid PostgreSQL syntax that can be executed directly

**Example Complex Patterns:**

1. Time series with previous value comparison:
```sql
SELECT
  block_timestamp,
  value,
  LAG(value) OVER (ORDER BY block_timestamp) as previous_value,
  value - LAG(value) OVER (ORDER BY block_timestamp) as change
FROM table_name
WHERE condition
ORDER BY block_timestamp DESC
LIMIT $1
```

2. Aggregated statistics:
```sql
SELECT
  DATE_TRUNC('hour', to_timestamp(block_timestamp)) as hour,
  AVG(value) as avg_value,
  MAX(value) as max_value,
  MIN(value) as min_value,
  COUNT(*) as event_count
FROM table_name
WHERE block_timestamp >= $1
GROUP BY hour
ORDER BY hour DESC
LIMIT $2
```

3. Multiple table joins:
```sql
SELECT
  a.block_timestamp,
  a.pool,
  a.fee_percentage,
  b.total_volume
FROM table_a a
INNER JOIN table_b b ON a.pool = b.pool AND a.block_number = b.block_number
WHERE a.pool = $1
ORDER BY a.block_timestamp DESC
LIMIT $2
```

4. Latest state per entity:
```sql
WITH latest_per_pool AS (
  SELECT DISTINCT ON (pool)
    pool,
    block_timestamp,
    fee_percentage
  FROM table_name
  WHERE condition
  ORDER BY pool, block_timestamp DESC
)
SELECT * FROM latest_per_pool
ORDER BY block_timestamp DESC
```

## Response Format

Return your response in the following JSON format:
{
  "endpoint_path": "/api/example/{param}",
  "description": "Clear description of what this endpoint returns",
  "method": "GET",
  "path_params": [
    {"name": "param", "type": "String", "description": "What this parameter represents"}
  ],
  "query_params": [
    {"name": "limit", "type": "u32", "default": 50},
    {"name": "startBlockTimestamp", "type": "Option<u64>", "default": "null"}
  ],
  "response_schema": {
    "name": "ExampleResponse",
    "fields": [
      {"name": "block_number", "type": "i64", "description": "Block number where event occurred"},
      {"name": "block_timestamp", "type": "i64", "description": "Unix timestamp of the block"},
      {"name": "value", "type": "String", "description": "The indexed value"}
    ]
  },
  "sql_query": "SELECT block_number, block_timestamp, value FROM table_name WHERE condition ORDER BY block_timestamp DESC LIMIT $1",
  "tables_referenced": ["table_name"]
}

## Type Mappings

**Rust Types for API:**
- Integer numbers: i64 (for block numbers, timestamps, counts)
- Unsigned numbers: u32, u64 (for pagination limits, small positive values)
- Large integers (uint256): String (since they exceed Rust integer limits)
- Addresses: String (hex format with 0x prefix)
- Booleans: bool
- Optional values: Option<T>
- Arrays: Vec<T>
- Decimals: String (for precise financial values)

**PostgreSQL to Rust:**
- BIGINT → i64
- NUMERIC(78, 0) → String (for uint256)
- VARCHAR(42) → String (for addresses)
- TEXT → String
- BOOLEAN → bool
- INTEGER → i32

## Important Guidelines

1. **Pagination**: Always include 'limit' query parameter with reasonable defaults (e.g., 50, max 200)
2. **Time Filtering**: Support startBlockTimestamp and/or endBlockTimestamp when dealing with time series. Use Option<u64> with default: "null" (the string "null") to make it optional. When NULL, the query should return the latest data ordered DESC. When provided, filter from that timestamp onwards.
3. **Validation**: Cap limit at 200 to prevent abuse
4. **Ordering**: Default to DESC for time series (newest first) to show most recent data
5. **Performance**: Create efficient queries with proper WHERE clauses and indexes
6. **Null Handling**: Use Option<T> for nullable fields in response schemas
7. **Response Fields**: Must exactly match SQL query columns (name and type)
8. **Tables Referenced**: List all tables used in the query (including subqueries and CTEs)

## Task Analysis

Carefully read the task description to understand:
- What data needs to be returned
- What filtering is required
- Whether aggregation or analytics are needed
- Time range requirements
- Pagination needs
- Any special computations or transformations"#;

        let tables_info = available_tables
            .iter()
            .map(|ir| {
                format!(
                    "Table: {}\nChain: {}\nContract: {}\nEvent: {}\nColumns: {}\nDescription: {}",
                    ir.table_schema.table_name,
                    ir.chain,
                    ir.contract_address,
                    ir.event_name,
                    ir.table_schema
                        .columns
                        .iter()
                        .map(|col| format!("{} ({})", col.name, col.column_type))
                        .collect::<Vec<_>>()
                        .join(", "),
                    ir.description
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        let error_context = if let Some(error) = previous_error {
            format!("\n\nIMPORTANT - Previous attempt failed with error: {}\nPlease fix this issue in your response.", error)
        } else {
            String::new()
        };

        let user_prompt = format!(
            r#"Endpoint Path:
{}

Endpoint Description:
{}

Task Description:
{}

Available Tables:
{}
{}

Please generate the IR for this API endpoint. Analyze the task carefully and create an appropriate SQL query that fulfills all requirements, using advanced PostgreSQL features if needed.

REMINDER - SQL Syntax Requirements:
1. Use single quotes (') for SQL string literals, NOT escaped quotes (\")
2. For numeric literals like 1000000000000000000, use ::numeric or CAST() syntax, NOT numeric '...'
3. Ensure all parentheses are balanced
4. The SQL must be valid PostgreSQL syntax that can be executed directly"#,
            endpoint_path, endpoint_description, task_description, tables_info, error_context
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

        let endpoint_ir: EndpointIrResult = serde_json::from_str(json_str)
            .context("Failed to parse AI response as JSON")?;

        Ok(endpoint_ir)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IrGenerationResult {
    pub event_name: String,
    pub event_signature: String,
    pub start_block: u64,
    pub contract_address: String,
    pub chain: String,
    pub indexed_fields: Vec<EventField>,
    pub table_schema: TableSchema,
    pub description: String,
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

/// Represents the generated IR for an API endpoint
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EndpointIrResult {
    pub endpoint_path: String,
    pub description: String,
    pub method: String,
    pub path_params: Vec<PathParam>,
    pub query_params: Vec<QueryParam>,
    pub response_schema: ResponseSchema,
    pub sql_query: String,
    pub tables_referenced: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PathParam {
    pub name: String,
    #[serde(rename = "type")]
    pub param_type: String,
    pub description: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ResponseSchema {
    pub name: String,
    pub fields: Vec<ResponseField>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ResponseField {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: String,
    pub description: String,
}