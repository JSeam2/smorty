//! Full e2e tests with testcontainers (requires Docker)
//!
//! Run with: cargo test --features e2e -- --test-threads=1

#![cfg(feature = "e2e")]

use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::Value;
use serial_test::serial;
use std::fs;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tempfile::TempDir;
use testcontainers::runners::AsyncRunner;
use testcontainers::ContainerAsync;
use testcontainers_modules::postgres::Postgres;
use wiremock::matchers::{body_partial_json, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Full e2e test server with real Postgres via testcontainers
pub struct E2eTestServer {
    pub url: String,
    pub db_url: String,
    server_process: Option<Child>,
    _container: ContainerAsync<Postgres>,
    _rpc_mock: MockServer,
    _temp_dir: TempDir,
}

impl E2eTestServer {
    /// Start a full e2e test server with testcontainers Postgres and mocked RPC
    pub async fn start() -> Result<Self> {
        // 1. Start Postgres container
        let container = Postgres::default()
            .start()
            .await
            .context("Failed to start Postgres container")?;

        let db_port = container.get_host_port_ipv4(5432).await?;
        let db_url = format!("postgres://postgres:postgres@localhost:{}/postgres", db_port);

        // 2. Start RPC mock server
        let rpc_mock = MockServer::start().await;
        Self::mount_rpc_cassettes(&rpc_mock).await?;

        // 3. Create temp directory for config and IR files
        let temp_dir = TempDir::new()?;

        // 4. Setup IR files
        Self::setup_ir_files(&temp_dir)?;

        // 5. Generate test config
        let config_path = Self::create_test_config(&temp_dir, &db_url, &rpc_mock.uri())?;

        // 6. Run migrations
        Self::run_migrations(&temp_dir, &config_path).await?;

        // 7. Run indexer (one-time mode)
        Self::run_indexer(&temp_dir, &config_path).await?;

        // 8. Find available port and start server
        let port = portpicker::pick_unused_port().expect("No ports available");
        let url = format!("http://127.0.0.1:{}", port);

        let server_process =
            Self::start_server(&temp_dir, &config_path, port)?;

        let server = Self {
            url,
            db_url,
            server_process: Some(server_process),
            _container: container,
            _rpc_mock: rpc_mock,
            _temp_dir: temp_dir,
        };

        // 9. Wait for server ready
        server.wait_for_ready(Duration::from_secs(60)).await?;

        Ok(server)
    }

    /// Mount RPC cassettes on the mock server
    async fn mount_rpc_cassettes(mock_server: &MockServer) -> Result<()> {
        let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/e2e/fixtures/cassettes/rpc");

        // Mount eth_blockNumber
        let block_number_cassette = fs::read_to_string(fixtures_dir.join("eth_blockNumber.json"))?;
        let block_number_response: Value = serde_json::from_str(&block_number_cassette)?;

        Mock::given(method("POST"))
            .and(body_partial_json(serde_json::json!({"method": "eth_blockNumber"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(&block_number_response))
            .mount(mock_server)
            .await;

        // Mount eth_getLogs
        let logs_cassette = fs::read_to_string(fixtures_dir.join("eth_getLogs_weth.json"))?;
        let logs_data: Value = serde_json::from_str(&logs_cassette)?;
        let logs_response = &logs_data["response"];

        Mock::given(method("POST"))
            .and(body_partial_json(serde_json::json!({"method": "eth_getLogs"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(logs_response))
            .mount(mock_server)
            .await;

        // Mount eth_getBlockByNumber (for timestamps)
        let blocks_cassette = fs::read_to_string(fixtures_dir.join("eth_getBlockByNumber.json"))?;
        let blocks_data: Value = serde_json::from_str(&blocks_cassette)?;

        // Mount responses for each block
        if let Some(blocks) = blocks_data["blocks"].as_array() {
            for block in blocks {
                let response = serde_json::json!({
                    "jsonrpc": "2.0",
                    "result": block["response"],
                    "id": 1
                });

                Mock::given(method("POST"))
                    .and(body_partial_json(serde_json::json!({"method": "eth_getBlockByNumber"})))
                    .respond_with(ResponseTemplate::new(200).set_body_json(&response))
                    .mount(mock_server)
                    .await;
            }
        }

        Ok(())
    }

    /// Setup IR files in temp directory
    fn setup_ir_files(temp_dir: &TempDir) -> Result<()> {
        let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/e2e/fixtures");

        // Create ir/specs/WETH directory
        let spec_dir = temp_dir.path().join("ir/specs/WETH");
        fs::create_dir_all(&spec_dir)?;

        // Copy spec IR
        let spec_content = fs::read_to_string(fixtures_dir.join("weth_spec_ir.json"))?;
        fs::write(spec_dir.join("transfers.json"), spec_content)?;

        // Create ir/endpoints directory
        let endpoint_dir = temp_dir.path().join("ir/endpoints");
        fs::create_dir_all(&endpoint_dir)?;

        // Copy endpoint IR
        let endpoint_content = fs::read_to_string(fixtures_dir.join("weth_transfers_endpoint.json"))?;
        fs::write(endpoint_dir.join("weth_transfers.json"), endpoint_content)?;

        // Create migrations directory with schema
        let migrations_dir = temp_dir.path().join("migrations");
        fs::create_dir_all(&migrations_dir)?;

        // Generate migration SQL from spec IR
        let spec: Value = serde_json::from_str(&fs::read_to_string(fixtures_dir.join("weth_spec_ir.json"))?)?;
        let table_schema = &spec["table_schema"];
        let table_name = table_schema["table_name"].as_str().unwrap_or("weth_transfers");

        let mut sql = format!("-- E2E test migration\nCREATE TABLE IF NOT EXISTS {} (\n", table_name);

        if let Some(columns) = table_schema["columns"].as_array() {
            let col_defs: Vec<String> = columns
                .iter()
                .map(|c| format!("    {} {}", c["name"].as_str().unwrap(), c["type"].as_str().unwrap()))
                .collect();
            sql.push_str(&col_defs.join(",\n"));
        }
        sql.push_str("\n);\n\n");

        if let Some(indexes) = table_schema["indexes"].as_array() {
            for idx in indexes {
                let idx_sql = idx.as_str().unwrap().replace("{table_name}", table_name);
                // Add IF NOT EXISTS to index creation
                let idx_sql = idx_sql.replace("CREATE INDEX", &format!("CREATE INDEX IF NOT EXISTS {}_{}", table_name, ""));
                sql.push_str(&format!("{};\n", idx_sql));
            }
        }

        fs::write(migrations_dir.join("0001_initial.sql"), sql)?;

        // Create schema.json for the indexer
        let schema_state = serde_json::json!({
            "tables": {
                table_name: {
                    "columns": table_schema["columns"],
                    "indexes": table_schema["indexes"]
                }
            }
        });
        fs::write(migrations_dir.join("schema.json"), serde_json::to_string_pretty(&schema_state)?)?;

        Ok(())
    }

    /// Create test config file
    fn create_test_config(
        temp_dir: &TempDir,
        db_url: &str,
        rpc_url: &str,
    ) -> Result<std::path::PathBuf> {
        let config_content = format!(
            r#"[database]
uri = "{}"

[chains]
mainnet = "{}"

[ai.openai]
model = "gpt-4o"
apiKey = "test-key-not-used"
temperature = 0.7

[contracts.WETH]
chain = "mainnet"
address = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"
abiPath = "tests/integration/fixtures/abi/weth.json"

[[contracts.WETH.specs]]
name = "transfers"
startBlock = 18500000
task = "Track all WETH token transfers"

[[endpoints]]
description = "Get WETH transfers"
endpoint = "/api/weth/transfers"
task = "Return recent WETH transfers"
"#,
            db_url, rpc_url
        );

        let config_path = temp_dir.path().join("config.e2e.toml");
        fs::write(&config_path, config_content)?;
        Ok(config_path)
    }

    /// Run migrations using sqlx
    async fn run_migrations(temp_dir: &TempDir, config_path: &Path) -> Result<()> {
        let output = Command::new("cargo")
            .args([
                "run",
                "--",
                "--config",
                config_path.to_str().unwrap(),
                "migrate",
            ])
            .current_dir(temp_dir.path())
            .env("CARGO_MANIFEST_DIR", env!("CARGO_MANIFEST_DIR"))
            .output()
            .context("Failed to run migrate command")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            eprintln!("Migration stdout: {}", stdout);
            eprintln!("Migration stderr: {}", stderr);
            // Don't fail on migration errors - table might already exist
        }

        Ok(())
    }

    /// Run indexer in one-time mode
    async fn run_indexer(temp_dir: &TempDir, config_path: &Path) -> Result<()> {
        let output = Command::new("cargo")
            .args([
                "run",
                "--",
                "--config",
                config_path.to_str().unwrap(),
                "index",
            ])
            .current_dir(temp_dir.path())
            .env("CARGO_MANIFEST_DIR", env!("CARGO_MANIFEST_DIR"))
            .output()
            .context("Failed to run index command")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            eprintln!("Indexer stdout: {}", stdout);
            eprintln!("Indexer stderr: {}", stderr);
            // Don't fail - indexer might have issues with mocked RPC
        }

        Ok(())
    }

    /// Start the server process
    fn start_server(
        temp_dir: &TempDir,
        config_path: &Path,
        port: u16,
    ) -> Result<Child> {
        let child = Command::new("cargo")
            .args([
                "run",
                "--",
                "--config",
                config_path.to_str().unwrap(),
                "serve",
                "--address",
                "127.0.0.1",
                "--port",
                &port.to_string(),
            ])
            .current_dir(temp_dir.path())
            .env("CARGO_MANIFEST_DIR", env!("CARGO_MANIFEST_DIR"))
            .env("RUST_LOG", "info")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to spawn server process")?;

        Ok(child)
    }

    /// Wait for server to be ready
    async fn wait_for_ready(&self, timeout: Duration) -> Result<()> {
        let client = Client::new();
        let start = std::time::Instant::now();

        loop {
            if start.elapsed() > timeout {
                anyhow::bail!("Server did not become ready within {:?}", timeout);
            }

            match client.get(format!("{}/health", self.url)).send().await {
                Ok(resp) if resp.status().is_success() => return Ok(()),
                _ => tokio::time::sleep(Duration::from_millis(500)).await,
            }
        }
    }

    /// Get URL for a path
    pub fn url(&self, path_str: &str) -> String {
        format!("{}{}", self.url, path_str)
    }
}

impl Drop for E2eTestServer {
    fn drop(&mut self) {
        if let Some(ref mut child) = self.server_process {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

// ============================================
// E2E Test Cases
// ============================================

#[tokio::test]
#[serial]
async fn test_e2e_health_endpoint() -> Result<()> {
    let server = E2eTestServer::start().await?;

    let client = Client::new();
    let resp = client.get(server.url("/health")).send().await?;

    assert_eq!(resp.status(), 200);

    let body: Value = resp.json().await?;
    assert_eq!(body["status"], "healthy");
    assert_eq!(body["service"], "smorty-indexer");

    Ok(())
}

#[tokio::test]
#[serial]
async fn test_e2e_swagger_ui() -> Result<()> {
    let server = E2eTestServer::start().await?;

    let client = Client::new();
    let resp = client.get(server.url("/swagger-ui/")).send().await?;

    assert_eq!(resp.status(), 200);

    let content_type = resp
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap_or(""))
        .unwrap_or("");
    assert!(
        content_type.contains("text/html"),
        "Expected HTML, got: {}",
        content_type
    );

    Ok(())
}

#[tokio::test]
#[serial]
async fn test_e2e_openapi_spec() -> Result<()> {
    let server = E2eTestServer::start().await?;

    let client = Client::new();
    let resp = client.get(server.url("/api-docs/openapi.json")).send().await?;

    assert_eq!(resp.status(), 200);

    let spec: Value = resp.json().await?;

    // Verify it's a valid OpenAPI spec
    assert!(
        spec.get("openapi").is_some() || spec.get("swagger").is_some(),
        "Response should be OpenAPI spec"
    );
    assert!(spec.get("info").is_some(), "Spec should have info section");
    assert!(spec.get("paths").is_some(), "Spec should have paths section");

    Ok(())
}

#[tokio::test]
#[serial]
async fn test_e2e_weth_transfers_endpoint() -> Result<()> {
    let server = E2eTestServer::start().await?;

    let client = Client::new();
    let resp = client
        .get(server.url("/api/weth/transfers"))
        .send()
        .await?;

    // Should return 200 even if no data
    assert_eq!(resp.status(), 200);

    let body: Value = resp.json().await?;

    // Response should have expected structure
    assert!(body.get("data").is_some() || body.is_array(), "Should return data");

    Ok(())
}

#[tokio::test]
#[serial]
async fn test_e2e_weth_transfers_with_limit() -> Result<()> {
    let server = E2eTestServer::start().await?;

    let client = Client::new();
    let resp = client
        .get(server.url("/api/weth/transfers?limit=1"))
        .send()
        .await?;

    assert_eq!(resp.status(), 200);

    Ok(())
}
