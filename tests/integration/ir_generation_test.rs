//! Integration tests for IR generation using recorded cassettes
//!
//! These tests use wiremock to replay recorded OpenAI responses,
//! making them deterministic, fast, and free to run.

use anyhow::Result;
use serde_json::Value;
use serial_test::serial;
use std::path::Path;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Load a cassette file by name
fn load_cassette(name: &str) -> String {
    let cassette_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/integration/fixtures/cassettes")
        .join(format!("{}.json", name));

    std::fs::read_to_string(&cassette_path)
        .unwrap_or_else(|_| panic!("Cassette not found: {:?}", cassette_path))
}

/// Load an ABI file by name
fn load_abi(name: &str) -> Value {
    let abi_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/integration/fixtures/abi")
        .join(format!("{}.json", name));

    let content = std::fs::read_to_string(&abi_path)
        .unwrap_or_else(|_| panic!("ABI not found: {:?}", abi_path));

    serde_json::from_str(&content).unwrap_or_else(|_| panic!("Invalid JSON in ABI: {:?}", abi_path))
}

/// Start a mock server and mount a cassette for chat completions
async fn setup_mock_with_cassette(cassette_name: &str) -> MockServer {
    let server = MockServer::start().await;
    let cassette = load_cassette(cassette_name);

    // async-openai sends to /chat/completions when using with_api_base()
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(cassette)
                .insert_header("content-type", "application/json"),
        )
        .mount(&server)
        .await;

    server
}

#[tokio::test]
#[serial]
async fn test_weth_transfer_ir_generation() -> Result<()> {
    // 1. Start mock server with recorded cassette
    let server = setup_mock_with_cassette("weth_transfer").await;

    // 2. Set the base URL to point at our mock
    // SAFETY: We're running tests serially, no other threads accessing this env var
    unsafe {
        std::env::set_var("OPENAI_BASE_URL", server.uri());
    }

    // 3. Load the WETH ABI
    let abi = load_abi("weth");

    // 4. Create AI client and generate IR
    let ai_client =
        smorty::ai::AiClient::new("fake-api-key".to_string(), "gpt-4o".to_string(), 0.7);

    let result = ai_client
        .generate_ir(
            "WETH",
            "transfers",
            Some(0),
            "0x0000000000000000000000000000000000000001",
            "mainnet",
            &abi,
            "Track all WETH token transfers",
        )
        .await;

    // 5. Clean up env var
    // SAFETY: We're running tests serially, no other threads accessing this env var
    unsafe {
        std::env::remove_var("OPENAI_BASE_URL");
    }

    // 6. Assert on the result
    let ir = result.expect("IR generation should succeed");

    assert_eq!(ir.event_name, "Transfer");
    assert_eq!(ir.event_signature, "Transfer(address,address,uint256)");
    assert_eq!(ir.chain, "mainnet");
    assert_eq!(
        ir.contract_address,
        "0x0000000000000000000000000000000000000001"
    );
    assert_eq!(ir.start_block, 0);

    // Check indexed fields
    assert_eq!(ir.indexed_fields.len(), 3);
    let field_names: Vec<&str> = ir.indexed_fields.iter().map(|f| f.name.as_str()).collect();
    assert!(field_names.contains(&"src"));
    assert!(field_names.contains(&"dst"));
    assert!(field_names.contains(&"wad"));

    // Check table schema - deterministic from recorded cassette
    assert_eq!(ir.table_schema.table_name, "weth_transfer_events");
    assert!(!ir.table_schema.columns.is_empty());

    // Verify standard columns exist
    let column_names: Vec<&str> = ir
        .table_schema
        .columns
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert!(column_names.contains(&"id"));
    assert!(column_names.contains(&"block_number"));
    assert!(column_names.contains(&"block_timestamp"));
    assert!(column_names.contains(&"transaction_hash"));
    assert!(column_names.contains(&"log_index"));
    assert!(column_names.contains(&"src"));
    assert!(column_names.contains(&"dst"));
    assert!(column_names.contains(&"wad"));

    // Check indexes exist
    assert!(!ir.table_schema.indexes.is_empty());

    // Check description is present
    assert!(!ir.description.is_empty());

    Ok(())
}

#[tokio::test]
#[serial]
async fn test_weth_deposit_ir_generation() -> Result<()> {
    let server = setup_mock_with_cassette("weth_deposit").await;

    unsafe {
        std::env::set_var("OPENAI_BASE_URL", server.uri());
    }

    let abi = load_abi("weth");
    let ai_client =
        smorty::ai::AiClient::new("fake-api-key".to_string(), "gpt-4o".to_string(), 0.7);

    let result = ai_client
        .generate_ir(
            "WETH",
            "deposits",
            Some(0),
            "0x0000000000000000000000000000000000000001",
            "mainnet",
            &abi,
            "Track all ETH deposits (wrapping) into WETH",
        )
        .await;

    unsafe {
        std::env::remove_var("OPENAI_BASE_URL");
    }

    let ir = result.expect("IR generation should succeed");

    assert_eq!(ir.event_name, "Deposit");
    assert_eq!(ir.event_signature, "Deposit(address,uint256)");
    assert_eq!(ir.chain, "mainnet");

    let field_names: Vec<&str> = ir.indexed_fields.iter().map(|f| f.name.as_str()).collect();
    assert!(field_names.contains(&"dst"));
    assert!(field_names.contains(&"wad"));

    assert!(!ir.table_schema.columns.is_empty());
    assert!(!ir.description.is_empty());

    Ok(())
}

#[tokio::test]
#[serial]
async fn test_uni_transfer_ir_generation() -> Result<()> {
    let server = setup_mock_with_cassette("uni_transfer").await;

    unsafe {
        std::env::set_var("OPENAI_BASE_URL", server.uri());
    }

    let abi = load_abi("uni");
    let ai_client =
        smorty::ai::AiClient::new("fake-api-key".to_string(), "gpt-4o".to_string(), 0.7);

    let result = ai_client
        .generate_ir(
            "UNI",
            "transfers",
            Some(0),
            "0x0000000000000000000000000000000000000001",
            "mainnet",
            &abi,
            "Track all UNI token transfers",
        )
        .await;

    unsafe {
        std::env::remove_var("OPENAI_BASE_URL");
    }

    let ir = result.expect("IR generation should succeed");

    assert_eq!(ir.event_name, "Transfer");
    assert_eq!(ir.event_signature, "Transfer(address,address,uint256)");
    assert_eq!(ir.chain, "mainnet");

    assert_eq!(ir.indexed_fields.len(), 3);
    assert!(!ir.table_schema.columns.is_empty());
    assert!(!ir.description.is_empty());

    Ok(())
}

#[tokio::test]
#[serial]
async fn test_uni_delegate_votes_ir_generation() -> Result<()> {
    let server = setup_mock_with_cassette("uni_delegate_votes").await;

    unsafe {
        std::env::set_var("OPENAI_BASE_URL", server.uri());
    }

    let abi = load_abi("uni");
    let ai_client =
        smorty::ai::AiClient::new("fake-api-key".to_string(), "gpt-4o".to_string(), 0.7);

    let result = ai_client
        .generate_ir(
            "UNI",
            "delegate_votes",
            Some(0),
            "0x0000000000000000000000000000000000000001",
            "mainnet",
            &abi,
            "Track when voting power changes due to delegation",
        )
        .await;

    unsafe {
        std::env::remove_var("OPENAI_BASE_URL");
    }

    let ir = result.expect("IR generation should succeed");

    assert_eq!(ir.event_name, "DelegateVotesChanged");
    assert_eq!(
        ir.event_signature,
        "DelegateVotesChanged(address,uint256,uint256)"
    );
    assert_eq!(ir.chain, "mainnet");

    let field_names: Vec<&str> = ir.indexed_fields.iter().map(|f| f.name.as_str()).collect();
    assert!(field_names.contains(&"delegate"));

    assert!(!ir.table_schema.columns.is_empty());
    assert!(!ir.description.is_empty());

    Ok(())
}

#[tokio::test]
#[serial]
async fn test_v3_pool_swap_ir_generation() -> Result<()> {
    let server = setup_mock_with_cassette("v3_pool_swap").await;

    unsafe {
        std::env::set_var("OPENAI_BASE_URL", server.uri());
    }

    let abi = load_abi("uniswap_v3_pool");
    let ai_client =
        smorty::ai::AiClient::new("fake-api-key".to_string(), "gpt-4o".to_string(), 0.7);

    let result = ai_client
        .generate_ir(
            "UniswapV3Pool",
            "swaps",
            Some(0),
            "0x0000000000000000000000000000000000000001",
            "mainnet",
            &abi,
            "Track all swap events on this Uniswap V3 pool",
        )
        .await;

    unsafe {
        std::env::remove_var("OPENAI_BASE_URL");
    }

    let ir = result.expect("IR generation should succeed");

    assert_eq!(ir.event_name, "Swap");
    assert_eq!(
        ir.event_signature,
        "Swap(address,address,int256,int256,uint160,uint128,int24)"
    );
    assert_eq!(ir.chain, "mainnet");

    let field_names: Vec<&str> = ir.indexed_fields.iter().map(|f| f.name.as_str()).collect();
    assert!(field_names.contains(&"sender"));
    assert!(field_names.contains(&"recipient"));

    let column_names: Vec<&str> = ir
        .table_schema
        .columns
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert!(column_names.contains(&"amount0"));
    assert!(column_names.contains(&"amount1"));
    assert!(column_names.contains(&"liquidity"));
    assert!(column_names.contains(&"tick"));

    assert!(!ir.description.is_empty());

    Ok(())
}

#[tokio::test]
#[serial]
async fn test_v3_pool_mint_ir_generation() -> Result<()> {
    let server = setup_mock_with_cassette("v3_pool_mint").await;

    unsafe {
        std::env::set_var("OPENAI_BASE_URL", server.uri());
    }

    let abi = load_abi("uniswap_v3_pool");
    let ai_client =
        smorty::ai::AiClient::new("fake-api-key".to_string(), "gpt-4o".to_string(), 0.7);

    let result = ai_client
        .generate_ir(
            "UniswapV3Pool",
            "mints",
            Some(0),
            "0x0000000000000000000000000000000000000001",
            "mainnet",
            &abi,
            "Track liquidity additions (Mint events) to this pool",
        )
        .await;

    unsafe {
        std::env::remove_var("OPENAI_BASE_URL");
    }

    let ir = result.expect("IR generation should succeed");

    assert_eq!(ir.event_name, "Mint");
    assert_eq!(
        ir.event_signature,
        "Mint(address,address,int24,int24,uint128,uint256,uint256)"
    );
    assert_eq!(ir.chain, "mainnet");

    let field_names: Vec<&str> = ir.indexed_fields.iter().map(|f| f.name.as_str()).collect();
    assert!(field_names.contains(&"owner"));

    let column_names: Vec<&str> = ir
        .table_schema
        .columns
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert!(column_names.contains(&"amount0"));
    assert!(column_names.contains(&"amount1"));

    assert!(!ir.description.is_empty());

    Ok(())
}

#[tokio::test]
#[serial]
async fn test_v3_factory_pool_created_ir_generation() -> Result<()> {
    let server = setup_mock_with_cassette("v3_factory_pool_created").await;

    unsafe {
        std::env::set_var("OPENAI_BASE_URL", server.uri());
    }

    let abi = load_abi("uniswap_v3_factory");
    let ai_client =
        smorty::ai::AiClient::new("fake-api-key".to_string(), "gpt-4o".to_string(), 0.7);

    let result = ai_client
        .generate_ir(
            "UniswapV3Factory",
            "pools",
            Some(0),
            "0x0000000000000000000000000000000000000001",
            "mainnet",
            &abi,
            "Track when new Uniswap V3 pools are created",
        )
        .await;

    unsafe {
        std::env::remove_var("OPENAI_BASE_URL");
    }

    let ir = result.expect("IR generation should succeed");

    assert_eq!(ir.event_name, "PoolCreated");
    assert_eq!(
        ir.event_signature,
        "PoolCreated(address,address,uint24,int24,address)"
    );
    assert_eq!(ir.chain, "mainnet");

    let field_names: Vec<&str> = ir.indexed_fields.iter().map(|f| f.name.as_str()).collect();
    assert!(field_names.contains(&"token0"));
    assert!(field_names.contains(&"token1"));
    assert!(field_names.contains(&"fee"));

    let column_names: Vec<&str> = ir
        .table_schema
        .columns
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert!(column_names.contains(&"pool"));

    assert!(!ir.description.is_empty());

    Ok(())
}
