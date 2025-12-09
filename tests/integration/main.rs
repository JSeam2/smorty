// Integration test utilities
use std::path::Path;

pub mod ir_generation_test;
pub mod recording;

/// Load a cassette file by name
pub fn load_cassette(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/integration/fixtures/cassettes")
        .join(format!("{}.json", name));

    std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("Cassette not found: {:?}", path))
}

/// Load an ABI file by name
pub fn load_abi(name: &str) -> serde_json::Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/integration/fixtures/abi")
        .join(format!("{}.json", name));

    let content =
        std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("ABI not found: {:?}", path));

    serde_json::from_str(&content).unwrap_or_else(|_| panic!("Invalid JSON in ABI: {:?}", path))
}
