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
