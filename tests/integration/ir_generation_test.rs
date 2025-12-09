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

    // Check table schema
    assert_eq!(ir.table_schema.table_name, "weth_transfers");
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
