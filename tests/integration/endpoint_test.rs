//! Integration tests for endpoint IR generation using recorded cassettes
//!
//! These tests verify the "natural language to SQL" functionality -
//! the Dune-like feature where users describe queries in plain English.

use anyhow::Result;
use serial_test::serial;
use std::path::Path;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use smorty::ai::{ColumnDef, EventField, IrGenerationResult, TableSchema};

fn load_cassette(name: &str) -> String {
    let cassette_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/integration/fixtures/cassettes")
        .join(format!("{}.json", name));

    std::fs::read_to_string(&cassette_path)
        .unwrap_or_else(|_| panic!("Cassette not found: {:?}", cassette_path))
}

async fn setup_mock_with_cassette(cassette_name: &str) -> MockServer {
    let server = MockServer::start().await;
    let cassette = load_cassette(cassette_name);

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

/// Create mock table schemas matching what the recording helper uses
fn mock_available_tables() -> Vec<IrGenerationResult> {
    vec![
        IrGenerationResult {
            event_name: "Transfer".to_string(),
            event_signature: "Transfer(address,address,uint256)".to_string(),
            start_block: 0,
            contract_address: "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2".to_string(),
            chain: "mainnet".to_string(),
            indexed_fields: vec![
                EventField {
                    name: "src".to_string(),
                    solidity_type: "address".to_string(),
                    rust_type: "String".to_string(),
                    indexed: true,
                },
                EventField {
                    name: "dst".to_string(),
                    solidity_type: "address".to_string(),
                    rust_type: "String".to_string(),
                    indexed: true,
                },
                EventField {
                    name: "wad".to_string(),
                    solidity_type: "uint256".to_string(),
                    rust_type: "String".to_string(),
                    indexed: false,
                },
            ],
            table_schema: TableSchema {
                table_name: "weth_transfers".to_string(),
                columns: vec![
                    ColumnDef { name: "id".to_string(), column_type: "BIGSERIAL PRIMARY KEY".to_string() },
                    ColumnDef { name: "block_number".to_string(), column_type: "BIGINT NOT NULL".to_string() },
                    ColumnDef { name: "block_timestamp".to_string(), column_type: "BIGINT NOT NULL".to_string() },
                    ColumnDef { name: "transaction_hash".to_string(), column_type: "VARCHAR(66) NOT NULL".to_string() },
                    ColumnDef { name: "log_index".to_string(), column_type: "INTEGER NOT NULL".to_string() },
                    ColumnDef { name: "src".to_string(), column_type: "VARCHAR(42) NOT NULL".to_string() },
                    ColumnDef { name: "dst".to_string(), column_type: "VARCHAR(42) NOT NULL".to_string() },
                    ColumnDef { name: "wad".to_string(), column_type: "NUMERIC(78, 0) NOT NULL".to_string() },
                ],
                indexes: vec![],
            },
            description: "Tracks all WETH token transfers".to_string(),
        },
        IrGenerationResult {
            event_name: "Transfer".to_string(),
            event_signature: "Transfer(address,address,uint256)".to_string(),
            start_block: 0,
            contract_address: "0x1f9840a85d5aF5bf1D1762F925BDADdC4201F984".to_string(),
            chain: "mainnet".to_string(),
            indexed_fields: vec![
                EventField {
                    name: "from_addr".to_string(),
                    solidity_type: "address".to_string(),
                    rust_type: "String".to_string(),
                    indexed: true,
                },
                EventField {
                    name: "to_addr".to_string(),
                    solidity_type: "address".to_string(),
                    rust_type: "String".to_string(),
                    indexed: true,
                },
                EventField {
                    name: "value".to_string(),
                    solidity_type: "uint256".to_string(),
                    rust_type: "String".to_string(),
                    indexed: false,
                },
            ],
            table_schema: TableSchema {
                table_name: "uni_transfers".to_string(),
                columns: vec![
                    ColumnDef { name: "id".to_string(), column_type: "BIGSERIAL PRIMARY KEY".to_string() },
                    ColumnDef { name: "block_number".to_string(), column_type: "BIGINT NOT NULL".to_string() },
                    ColumnDef { name: "block_timestamp".to_string(), column_type: "BIGINT NOT NULL".to_string() },
                    ColumnDef { name: "transaction_hash".to_string(), column_type: "VARCHAR(66) NOT NULL".to_string() },
                    ColumnDef { name: "log_index".to_string(), column_type: "INTEGER NOT NULL".to_string() },
                    ColumnDef { name: "from_addr".to_string(), column_type: "VARCHAR(42) NOT NULL".to_string() },
                    ColumnDef { name: "to_addr".to_string(), column_type: "VARCHAR(42) NOT NULL".to_string() },
                    ColumnDef { name: "value".to_string(), column_type: "NUMERIC(78, 0) NOT NULL".to_string() },
                ],
                indexes: vec![],
            },
            description: "Tracks all UNI token transfers".to_string(),
        },
        IrGenerationResult {
            event_name: "Swap".to_string(),
            event_signature: "Swap(address,address,int256,int256,uint160,uint128,int24)".to_string(),
            start_block: 0,
            contract_address: "0x88e6A0c2dDD26FEEb64F039a2c41296FcB3f5640".to_string(),
            chain: "mainnet".to_string(),
            indexed_fields: vec![
                EventField {
                    name: "sender".to_string(),
                    solidity_type: "address".to_string(),
                    rust_type: "String".to_string(),
                    indexed: true,
                },
                EventField {
                    name: "recipient".to_string(),
                    solidity_type: "address".to_string(),
                    rust_type: "String".to_string(),
                    indexed: true,
                },
            ],
            table_schema: TableSchema {
                table_name: "v3_pool_swaps".to_string(),
                columns: vec![
                    ColumnDef { name: "id".to_string(), column_type: "BIGSERIAL PRIMARY KEY".to_string() },
                    ColumnDef { name: "block_number".to_string(), column_type: "BIGINT NOT NULL".to_string() },
                    ColumnDef { name: "block_timestamp".to_string(), column_type: "BIGINT NOT NULL".to_string() },
                    ColumnDef { name: "transaction_hash".to_string(), column_type: "VARCHAR(66) NOT NULL".to_string() },
                    ColumnDef { name: "log_index".to_string(), column_type: "INTEGER NOT NULL".to_string() },
                    ColumnDef { name: "sender".to_string(), column_type: "VARCHAR(42) NOT NULL".to_string() },
                    ColumnDef { name: "recipient".to_string(), column_type: "VARCHAR(42) NOT NULL".to_string() },
                    ColumnDef { name: "amount0".to_string(), column_type: "NUMERIC(78, 0) NOT NULL".to_string() },
                    ColumnDef { name: "amount1".to_string(), column_type: "NUMERIC(78, 0) NOT NULL".to_string() },
                    ColumnDef { name: "sqrt_price_x96".to_string(), column_type: "NUMERIC(78, 0) NOT NULL".to_string() },
                    ColumnDef { name: "liquidity".to_string(), column_type: "NUMERIC(39, 0) NOT NULL".to_string() },
                    ColumnDef { name: "tick".to_string(), column_type: "INTEGER NOT NULL".to_string() },
                ],
                indexes: vec![],
            },
            description: "Tracks all swap events on Uniswap V3 USDC/ETH pool".to_string(),
        },
    ]
}

#[tokio::test]
#[serial]
async fn test_endpoint_weth_transfers() -> Result<()> {
    let server = setup_mock_with_cassette("endpoint_weth_transfers").await;

    unsafe {
        std::env::set_var("OPENAI_BASE_URL", server.uri());
    }

    let ai_client =
        smorty::ai::AiClient::new("fake-api-key".to_string(), "gpt-4o".to_string(), 0.7);

    let tables = mock_available_tables();
    let result = ai_client
        .generate_endpoint_ir(
            "/api/weth/transfers",
            "Get recent WETH transfers",
            "Return the most recent WETH transfers with pagination and optional address filtering",
            &tables,
        )
        .await;

    unsafe {
        std::env::remove_var("OPENAI_BASE_URL");
    }

    let endpoint = result.expect("Endpoint IR generation should succeed");

    // Verify endpoint structure
    assert_eq!(endpoint.endpoint_path, "/api/weth/transfers");
    assert_eq!(endpoint.method, "GET");
    assert!(endpoint.tables_referenced.contains(&"weth_transfers".to_string()));

    // Verify SQL is present and references the table
    assert!(endpoint.sql_query.to_lowercase().contains("weth_transfers"));
    assert!(endpoint.sql_query.contains("SELECT"));

    // Verify response schema has expected fields
    let field_names: Vec<&str> = endpoint
        .response_schema
        .fields
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert!(field_names.contains(&"src") || field_names.contains(&"dst"));

    // Verify query params include limit (pagination)
    let param_names: Vec<&str> = endpoint
        .query_params
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    assert!(param_names.contains(&"limit"));

    Ok(())
}

#[tokio::test]
#[serial]
async fn test_endpoint_cross_contract_whales() -> Result<()> {
    let server = setup_mock_with_cassette("endpoint_cross_contract_whales").await;

    unsafe {
        std::env::set_var("OPENAI_BASE_URL", server.uri());
    }

    let ai_client =
        smorty::ai::AiClient::new("fake-api-key".to_string(), "gpt-4o".to_string(), 0.7);

    let tables = mock_available_tables();
    let result = ai_client
        .generate_endpoint_ir(
            "/api/whales",
            "Find addresses active in both WETH and UNI",
            "Return addresses that have both sent WETH and received UNI tokens, showing their activity across both contracts. This requires joining weth_transfers and uni_transfers tables.",
            &tables,
        )
        .await;

    unsafe {
        std::env::remove_var("OPENAI_BASE_URL");
    }

    let endpoint = result.expect("Cross-contract endpoint IR generation should succeed");

    // This is the key test - verifies cross-contract JOIN works
    assert!(
        endpoint.tables_referenced.len() >= 2,
        "Cross-contract query should reference multiple tables"
    );
    assert!(endpoint.tables_referenced.contains(&"weth_transfers".to_string()));
    assert!(endpoint.tables_referenced.contains(&"uni_transfers".to_string()));

    // Verify SQL uses JOIN or subqueries to combine tables
    let sql_lower = endpoint.sql_query.to_lowercase();
    assert!(
        sql_lower.contains("join") || sql_lower.contains("with"),
        "Cross-contract query should use JOIN or CTE"
    );

    Ok(())
}

#[tokio::test]
#[serial]
async fn test_endpoint_swap_volume_hourly() -> Result<()> {
    let server = setup_mock_with_cassette("endpoint_swap_volume_hourly").await;

    unsafe {
        std::env::set_var("OPENAI_BASE_URL", server.uri());
    }

    let ai_client =
        smorty::ai::AiClient::new("fake-api-key".to_string(), "gpt-4o".to_string(), 0.7);

    let tables = mock_available_tables();
    let result = ai_client
        .generate_endpoint_ir(
            "/api/v3/volume/hourly",
            "Get hourly swap volume statistics",
            "Return aggregated swap statistics grouped by hour: total swap count, sum of amount0, sum of amount1. Use DATE_TRUNC for grouping.",
            &tables,
        )
        .await;

    unsafe {
        std::env::remove_var("OPENAI_BASE_URL");
    }

    let endpoint = result.expect("Aggregation endpoint IR generation should succeed");

    // Verify aggregation SQL patterns
    let sql_lower = endpoint.sql_query.to_lowercase();
    assert!(sql_lower.contains("group by"), "Aggregation query should GROUP BY");
    assert!(
        sql_lower.contains("date_trunc") || sql_lower.contains("extract"),
        "Hourly aggregation should use DATE_TRUNC or EXTRACT"
    );
    assert!(
        sql_lower.contains("count") || sql_lower.contains("sum"),
        "Aggregation query should use COUNT or SUM"
    );

    Ok(())
}

#[tokio::test]
#[serial]
async fn test_endpoint_v3_swaps_by_pool() -> Result<()> {
    let server = setup_mock_with_cassette("endpoint_v3_swaps_by_pool").await;

    unsafe {
        std::env::set_var("OPENAI_BASE_URL", server.uri());
    }

    let ai_client =
        smorty::ai::AiClient::new("fake-api-key".to_string(), "gpt-4o".to_string(), 0.7);

    let tables = mock_available_tables();
    let result = ai_client
        .generate_endpoint_ir(
            "/api/v3/swaps/{pool}",
            "Get swaps for a specific Uniswap V3 pool",
            "Return all swaps for a given pool address with time range filtering and pagination",
            &tables,
        )
        .await;

    unsafe {
        std::env::remove_var("OPENAI_BASE_URL");
    }

    let endpoint = result.expect("Path param endpoint IR generation should succeed");

    // Verify path parameter extraction
    assert!(
        !endpoint.path_params.is_empty(),
        "Should have path parameters for {{pool}}"
    );
    assert!(endpoint
        .path_params
        .iter()
        .any(|p| p.name == "pool"));

    // Verify SQL uses parameterized query
    assert!(
        endpoint.sql_query.contains("$1"),
        "SQL should use parameterized queries"
    );

    Ok(())
}
