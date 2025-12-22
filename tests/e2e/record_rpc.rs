//! Recording helper for capturing Ethereum RPC responses as cassettes
//!
//! Run with: RPC_URL=https://eth-mainnet.g.alchemy.com/v2/YOUR_KEY \
//!           cargo test --test e2e record_rpc -- --ignored --nocapture

#![cfg(feature = "e2e")]

use anyhow::Result;
use serde_json::{Value, json};
use std::path::Path;

/// WETH contract address on mainnet
const WETH_ADDRESS: &str = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2";

/// Transfer event signature hash: keccak256("Transfer(address,address,uint256)")
const TRANSFER_TOPIC: &str =
    "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";

/// Makes a JSON-RPC call and returns the response
async fn rpc_call(rpc_url: &str, method: &str, params: Value) -> Result<Value> {
    let client = reqwest::Client::new();
    let response = client
        .post(rpc_url)
        .header("Content-Type", "application/json")
        .json(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": 1
        }))
        .send()
        .await?;

    let body: Value = response.json().await?;
    Ok(body)
}

/// Records eth_blockNumber response
async fn record_block_number(rpc_url: &str) -> Result<u64> {
    println!("Recording eth_blockNumber...");

    let response = rpc_call(rpc_url, "eth_blockNumber", json!([])).await?;

    let cassette_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/e2e/fixtures/cassettes/rpc/eth_blockNumber.json");

    std::fs::write(&cassette_path, serde_json::to_string_pretty(&response)?)?;
    println!("Saved cassette to: {:?}", cassette_path);

    // Extract block number
    let block_hex = response["result"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No result in response"))?;
    let block_num = u64::from_str_radix(&block_hex[2..], 16)?;
    println!("Current block: {} ({})", block_num, block_hex);

    Ok(block_num)
}

/// Records eth_getLogs for WETH Transfer events
async fn record_weth_transfer_logs(
    rpc_url: &str,
    from_block: u64,
    to_block: u64,
) -> Result<()> {
    println!(
        "Recording eth_getLogs for WETH transfers (blocks {} to {})...",
        from_block, to_block
    );

    let from_hex = format!("0x{:x}", from_block);
    let to_hex = format!("0x{:x}", to_block);

    let params = json!([{
        "address": WETH_ADDRESS,
        "topics": [TRANSFER_TOPIC],
        "fromBlock": from_hex,
        "toBlock": to_hex
    }]);

    let response = rpc_call(rpc_url, "eth_getLogs", params.clone()).await?;

    // Save full cassette with request info
    let cassette = json!({
        "request": {
            "method": "eth_getLogs",
            "params": params
        },
        "response": response
    });

    let cassette_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/e2e/fixtures/cassettes/rpc/eth_getLogs_weth.json");

    std::fs::write(&cassette_path, serde_json::to_string_pretty(&cassette)?)?;
    println!("Saved cassette to: {:?}", cassette_path);

    // Count logs
    if let Some(logs) = response["result"].as_array() {
        println!("Captured {} WETH Transfer logs", logs.len());
        if !logs.is_empty() {
            println!("First log: {}", serde_json::to_string_pretty(&logs[0])?);
        }
    }

    Ok(())
}

/// Records eth_getBlockByNumber for block timestamps
async fn record_block_timestamps(
    rpc_url: &str,
    block_numbers: &[u64],
) -> Result<()> {
    println!("Recording block timestamps for {} blocks...", block_numbers.len());

    let mut blocks = Vec::new();

    for &block_num in block_numbers {
        let block_hex = format!("0x{:x}", block_num);
        let response = rpc_call(
            rpc_url,
            "eth_getBlockByNumber",
            json!([block_hex, false]),
        )
        .await?;

        if let Some(result) = response.get("result") {
            blocks.push(json!({
                "blockNumber": block_hex,
                "response": result
            }));
        }
    }

    let cassette = json!({
        "method": "eth_getBlockByNumber",
        "blocks": blocks
    });

    let cassette_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/e2e/fixtures/cassettes/rpc/eth_getBlockByNumber.json");

    std::fs::write(&cassette_path, serde_json::to_string_pretty(&cassette)?)?;
    println!("Saved cassette to: {:?}", cassette_path);

    Ok(())
}

// ============================================
// Recording tests - run with --ignored flag
// ============================================

#[tokio::test]
#[ignore]
async fn record_rpc_cassettes() {
    let rpc_url = std::env::var("RPC_URL").expect("RPC_URL must be set for recording");

    // Get current block
    let current_block = record_block_number(&rpc_url)
        .await
        .expect("Failed to record block number");

    // Record WETH transfers from a small range (10 blocks back)
    // This should capture a few Transfer events
    let from_block = current_block.saturating_sub(100);
    let to_block = current_block;

    record_weth_transfer_logs(&rpc_url, from_block, to_block)
        .await
        .expect("Failed to record WETH logs");

    // Record block timestamps for the blocks we care about
    let blocks_to_record: Vec<u64> = (from_block..=to_block).step_by(10).collect();
    record_block_timestamps(&rpc_url, &blocks_to_record)
        .await
        .expect("Failed to record block timestamps");

    println!("\nAll RPC cassettes recorded successfully!");
    println!("You can now run e2e tests without a real RPC endpoint.");
}

/// Alternative: Record with a specific block range (for reproducible tests)
#[tokio::test]
#[ignore]
async fn record_rpc_fixed_range() {
    let rpc_url = std::env::var("RPC_URL").expect("RPC_URL must be set for recording");

    // Use a fixed block range with known WETH activity
    // Block 18500000 is from late October 2023
    let from_block: u64 = 18500000;
    let to_block: u64 = 18500010;

    // Record block number (use the to_block as "current")
    let response = json!({
        "jsonrpc": "2.0",
        "result": format!("0x{:x}", to_block),
        "id": 1
    });

    let cassette_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/e2e/fixtures/cassettes/rpc/eth_blockNumber.json");

    std::fs::write(
        &cassette_path,
        serde_json::to_string_pretty(&response).expect("Failed to serialize"),
    )
    .expect("Failed to write cassette");
    println!("Saved fixed block number cassette: {}", to_block);

    // Record WETH transfers
    record_weth_transfer_logs(&rpc_url, from_block, to_block)
        .await
        .expect("Failed to record WETH logs");

    // Record block timestamps
    let blocks: Vec<u64> = (from_block..=to_block).collect();
    record_block_timestamps(&rpc_url, &blocks)
        .await
        .expect("Failed to record block timestamps");

    println!("\nFixed range RPC cassettes recorded!");
}
