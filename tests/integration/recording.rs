//! Recording helper for capturing OpenAI responses as cassettes
//!
//! Run with: OPENAI_API_KEY=sk-xxx cargo test --test integration record_ -- --ignored --nocapture

use anyhow::Result;
use serde_json::{Value, json};
use std::path::Path;

/// Records an OpenAI response for a given ABI and task
/// Saves the raw HTTP response to a cassette file
async fn record_ir_generation(
    abi_name: &str,
    contract_name: &str,
    spec_name: &str,
    task_description: &str,
    cassette_name: &str,
) -> Result<()> {
    let api_key =
        std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY must be set for recording");

    // Load ABI
    let abi_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/integration/fixtures/abi")
        .join(format!("{}.json", abi_name));

    let abi_content = std::fs::read_to_string(&abi_path)?;
    let abi: Value = serde_json::from_str(&abi_content)?;

    // Build the same prompt as generate_ir
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
  "contract_address": "0xContractAddress",
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
  "description": "A brief description of the event"
}

Important type mappings:
- uint256 -> NUMERIC(78, 0)
- uint128 -> NUMERIC(39, 0)
- address -> VARCHAR(42)
- bytes32 -> VARCHAR(66)
- bool -> BOOLEAN
- string -> TEXT"#;

    let user_prompt = format!(
        r#"Contract: {}
Spec Name: {}
Start Block: 0
Contract Address: 0x0000000000000000000000000000000000000001
Chain: mainnet

ABI:
{}

Task Description:
{}

Please generate the IR for this indexing specification."#,
        contract_name,
        spec_name,
        serde_json::to_string_pretty(&abi)?,
        task_description,
    );

    // Make the API call
    let client = reqwest::Client::new();
    let response = client
        .post("https://api.openai.com/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&json!({
            "model": "gpt-4o",
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": user_prompt}
            ],
            "temperature": 0.7
        }))
        .send()
        .await?;

    let status = response.status();
    let body = response.text().await?;

    if !status.is_success() {
        eprintln!("OpenAI API error: {} - {}", status, body);
        anyhow::bail!("API error: {}", status);
    }

    // Save the raw response as a cassette
    let cassette_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/integration/fixtures/cassettes")
        .join(format!("{}.json", cassette_name));

    std::fs::write(&cassette_path, &body)?;
    println!("Saved cassette to: {:?}", cassette_path);

    // Also print the extracted content for debugging
    let parsed: Value = serde_json::from_str(&body)?;
    if let Some(content) = parsed["choices"][0]["message"]["content"].as_str() {
        println!("\n=== Generated IR ===\n{}", content);
    }

    Ok(())
}

// ============================================
// Recording tests - run with --ignored flag
// ============================================

#[tokio::test]
#[ignore]
async fn record_weth_transfer() {
    record_ir_generation(
        "weth",
        "WETH",
        "transfers",
        "Track all WETH token transfers",
        "weth_transfer",
    )
    .await
    .expect("Failed to record");
}

#[tokio::test]
#[ignore]
async fn record_weth_deposit() {
    record_ir_generation(
        "weth",
        "WETH",
        "deposits",
        "Track all ETH deposits (wrapping) into WETH",
        "weth_deposit",
    )
    .await
    .expect("Failed to record");
}

#[tokio::test]
#[ignore]
async fn record_uni_transfer() {
    record_ir_generation(
        "uni",
        "UNI",
        "transfers",
        "Track all UNI token transfers",
        "uni_transfer",
    )
    .await
    .expect("Failed to record");
}

#[tokio::test]
#[ignore]
async fn record_uni_delegate_votes() {
    record_ir_generation(
        "uni",
        "UNI",
        "delegate_votes",
        "Track when voting power changes due to delegation",
        "uni_delegate_votes",
    )
    .await
    .expect("Failed to record");
}

#[tokio::test]
#[ignore]
async fn record_v3_pool_swap() {
    record_ir_generation(
        "uniswap_v3_pool",
        "UniswapV3Pool",
        "swaps",
        "Track all swap events on this Uniswap V3 pool",
        "v3_pool_swap",
    )
    .await
    .expect("Failed to record");
}

#[tokio::test]
#[ignore]
async fn record_v3_pool_mint() {
    record_ir_generation(
        "uniswap_v3_pool",
        "UniswapV3Pool",
        "mints",
        "Track liquidity additions (Mint events) to this pool",
        "v3_pool_mint",
    )
    .await
    .expect("Failed to record");
}

#[tokio::test]
#[ignore]
async fn record_v3_factory_pool_created() {
    record_ir_generation(
        "uniswap_v3_factory",
        "UniswapV3Factory",
        "pools",
        "Track when new Uniswap V3 pools are created",
        "v3_factory_pool_created",
    )
    .await
    .expect("Failed to record");
}

// ============================================
// Endpoint IR Recording
// ============================================

/// Mock table schemas for endpoint generation tests
fn mock_available_tables() -> String {
    r#"Table: weth_transfers
Chain: mainnet
Contract: 0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2
Event: Transfer
Columns: id (BIGSERIAL PRIMARY KEY), block_number (BIGINT NOT NULL), block_timestamp (BIGINT NOT NULL), transaction_hash (VARCHAR(66) NOT NULL), log_index (INTEGER NOT NULL), src (VARCHAR(42) NOT NULL), dst (VARCHAR(42) NOT NULL), wad (NUMERIC(78, 0) NOT NULL)
Description: Tracks all WETH token transfers

Table: uni_transfers
Chain: mainnet
Contract: 0x1f9840a85d5aF5bf1D1762F925BDADdC4201F984
Event: Transfer
Columns: id (BIGSERIAL PRIMARY KEY), block_number (BIGINT NOT NULL), block_timestamp (BIGINT NOT NULL), transaction_hash (VARCHAR(66) NOT NULL), log_index (INTEGER NOT NULL), from_addr (VARCHAR(42) NOT NULL), to_addr (VARCHAR(42) NOT NULL), value (NUMERIC(78, 0) NOT NULL)
Description: Tracks all UNI token transfers

Table: v3_pool_swaps
Chain: mainnet
Contract: 0x88e6A0c2dDD26FEEb64F039a2c41296FcB3f5640
Event: Swap
Columns: id (BIGSERIAL PRIMARY KEY), block_number (BIGINT NOT NULL), block_timestamp (BIGINT NOT NULL), transaction_hash (VARCHAR(66) NOT NULL), log_index (INTEGER NOT NULL), sender (VARCHAR(42) NOT NULL), recipient (VARCHAR(42) NOT NULL), amount0 (NUMERIC(78, 0) NOT NULL), amount1 (NUMERIC(78, 0) NOT NULL), sqrt_price_x96 (NUMERIC(78, 0) NOT NULL), liquidity (NUMERIC(39, 0) NOT NULL), tick (INTEGER NOT NULL)
Description: Tracks all swap events on Uniswap V3 USDC/ETH pool"#.to_string()
}

/// Records an OpenAI response for endpoint IR generation
async fn record_endpoint_ir_generation(
    endpoint_path: &str,
    endpoint_description: &str,
    task_description: &str,
    cassette_name: &str,
) -> Result<()> {
    let api_key =
        std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY must be set for recording");

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
- BIGINT -> i64
- NUMERIC(78, 0) -> String (for uint256)
- VARCHAR(42) -> String (for addresses)
- TEXT -> String
- BOOLEAN -> bool
- INTEGER -> i32

## Important Guidelines

1. **Pagination**: Always include 'limit' query parameter with reasonable defaults (e.g., 50, max 200)
2. **Time Filtering**: Support startBlockTimestamp and/or endBlockTimestamp when dealing with time series
3. **Validation**: Cap limit at 200 to prevent abuse
4. **Ordering**: Default to DESC for time series (newest first)
5. **Response Fields**: Must exactly match SQL query columns (name and type)
6. **Tables Referenced**: List all tables used in the query"#;

    let tables_info = mock_available_tables();

    let user_prompt = format!(
        r#"Endpoint Path:
{}

Endpoint Description:
{}

Task Description:
{}

Available Tables:
{}

Please generate the IR for this API endpoint."#,
        endpoint_path, endpoint_description, task_description, tables_info
    );

    let client = reqwest::Client::new();
    let response = client
        .post("https://api.openai.com/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&json!({
            "model": "gpt-4o",
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": user_prompt}
            ],
            "temperature": 0.7
        }))
        .send()
        .await?;

    let status = response.status();
    let body = response.text().await?;

    if !status.is_success() {
        eprintln!("OpenAI API error: {} - {}", status, body);
        anyhow::bail!("API error: {}", status);
    }

    let cassette_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/integration/fixtures/cassettes")
        .join(format!("{}.json", cassette_name));

    std::fs::write(&cassette_path, &body)?;
    println!("Saved cassette to: {:?}", cassette_path);

    let parsed: Value = serde_json::from_str(&body)?;
    if let Some(content) = parsed["choices"][0]["message"]["content"].as_str() {
        println!("\n=== Generated Endpoint IR ===\n{}", content);
    }

    Ok(())
}

// ============================================
// Endpoint Recording Tests
// ============================================

#[tokio::test]
#[ignore]
async fn record_endpoint_weth_transfers() {
    record_endpoint_ir_generation(
        "/api/weth/transfers",
        "Get recent WETH transfers",
        "Return the most recent WETH transfers with pagination and optional address filtering",
        "endpoint_weth_transfers",
    )
    .await
    .expect("Failed to record");
}

#[tokio::test]
#[ignore]
async fn record_endpoint_v3_swaps_by_pool() {
    record_endpoint_ir_generation(
        "/api/v3/swaps/{pool}",
        "Get swaps for a specific Uniswap V3 pool",
        "Return all swaps for a given pool address with time range filtering and pagination",
        "endpoint_v3_swaps_by_pool",
    )
    .await
    .expect("Failed to record");
}

#[tokio::test]
#[ignore]
async fn record_endpoint_cross_contract_whales() {
    record_endpoint_ir_generation(
        "/api/whales",
        "Find addresses active in both WETH and UNI",
        "Return addresses that have both sent WETH and received UNI tokens, showing their activity across both contracts. This requires joining weth_transfers and uni_transfers tables.",
        "endpoint_cross_contract_whales",
    )
    .await
    .expect("Failed to record");
}

#[tokio::test]
#[ignore]
async fn record_endpoint_swap_volume_hourly() {
    record_endpoint_ir_generation(
        "/api/v3/volume/hourly",
        "Get hourly swap volume statistics",
        "Return aggregated swap statistics grouped by hour: total swap count, sum of amount0, sum of amount1. Use DATE_TRUNC for grouping.",
        "endpoint_swap_volume_hourly",
    )
    .await
    .expect("Failed to record");
}
