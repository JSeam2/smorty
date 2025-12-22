# Smorty Testing Documentation

## Table of Contents

1. [Overview](#overview)
2. [Test Architecture](#test-architecture)
3. [Integration Tests](#integration-tests)
4. [End-to-End Tests](#end-to-end-tests)
5. [Continuous Integration](#continuous-integration)
6. [Test Fixtures](#test-fixtures)
7. [Running Tests](#running-tests)

---

## Overview

The Smorty test suite is organized into three layers:

| Layer | Location | Purpose | Dependencies |
|-------|----------|---------|--------------|
| Unit | `src/*.rs` | Module-level logic | None |
| Integration | `tests/integration/` | AI response parsing | Wiremock |
| E2E | `tests/e2e/` | Full runtime flow | Testcontainers, Docker |

### Design Principles

1. **Determinism** - Tests produce identical results on every run
2. **Isolation** - No shared state between tests (serial execution where needed)
3. **Speed** - Mocked external services eliminate network latency
4. **CI Compatibility** - All tests run in GitHub Actions without external credentials

---

## Test Architecture

### Component Coverage

```
┌─────────────────────────────────────────────────────────────────────┐
│                         SMORTY COMPONENTS                           │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐             │
│  │   src/ai.rs │    │  src/ir.rs  │    │src/server.rs│             │
│  │             │    │             │    │             │             │
│  │  OpenAI     │───>│  IR Files   │───>│  HTTP API   │             │
│  │  Client     │    │  Load/Save  │    │  Dynamic    │             │
│  │             │    │             │    │  Routes     │             │
│  └─────────────┘    └─────────────┘    └─────────────┘             │
│        │                  │                  │                      │
│        ▼                  ▼                  ▼                      │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐             │
│  │ Integration │    │    Unit     │    │    E2E      │             │
│  │   Tests     │    │   Tests     │    │   Tests     │             │
│  │  (Wiremock) │    │  (inline)   │    │(testcontain)│             │
│  └─────────────┘    └─────────────┘    └─────────────┘             │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Integration Tests

### Purpose

Validate AI-generated intermediate representations (IR) without requiring live OpenAI API calls.

### Location

```
tests/integration/
├── main.rs                 # Shared utilities
├── ir_generation_test.rs   # Event IR tests (7 tests)
├── endpoint_test.rs        # Endpoint IR tests (4 tests)
├── recording.rs            # Cassette recording helpers
└── fixtures/
    ├── abi/                # Contract ABIs
    └── cassettes/          # Recorded API responses
```

### Mechanism

```
┌──────────────┐     ┌──────────────┐     ┌──────────────┐
│  Test Code   │────>│   Wiremock   │────>│  AiClient    │
│              │     │ Mock Server  │     │              │
│ 1. Load      │     │              │     │ Thinks it's  │
│    cassette  │     │ Intercepts   │     │ talking to   │
│ 2. Mount on  │     │ POST to      │     │ OpenAI       │
│    mock      │     │ /chat/       │     │              │
│ 3. Set env   │     │ completions  │     │              │
│ 4. Call AI   │     │              │     │              │
│ 5. Assert    │     │ Returns      │     │              │
│              │<────│ cassette     │<────│              │
└──────────────┘     └──────────────┘     └──────────────┘
```

### Test: Event IR Generation

**File:** `tests/integration/ir_generation_test.rs`

**Function:** `setup_mock_with_cassette`
```rust
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
```

**Tests:**

| Test | Cassette | Validates |
|------|----------|-----------|
| `test_weth_transfer_ir_generation` | `weth_transfer.json` | Transfer event with 3 fields (src, dst, wad) |
| `test_weth_deposit_ir_generation` | `weth_deposit.json` | Deposit event with 2 fields (dst, wad) |
| `test_uni_transfer_ir_generation` | `uni_transfer.json` | ERC20 Transfer standard |
| `test_uni_delegate_votes_ir_generation` | `uni_delegate_votes.json` | Governance event (delegate, previousBalance, newBalance) |
| `test_v3_pool_swap_ir_generation` | `v3_pool_swap.json` | Complex event with signed integers (int256) |
| `test_v3_pool_mint_ir_generation` | `v3_pool_mint.json` | Liquidity event with tick ranges |
| `test_v3_factory_pool_created_ir_generation` | `v3_factory_pool_created.json` | Factory event with indexed token addresses |

**Assertions per test:**
- `event_name` matches expected event
- `event_signature` matches Solidity signature
- `chain` is "mainnet"
- `indexed_fields` contains expected field names
- `table_schema.columns` includes standard columns (id, block_number, block_timestamp, transaction_hash, log_index)
- `table_schema.indexes` is non-empty
- `description` is non-empty

### Test: Endpoint IR Generation

**File:** `tests/integration/endpoint_test.rs`

**Mock Data:** `mock_available_tables()` provides three table schemas:
- `weth_transfers` - WETH Transfer events
- `uni_transfers` - UNI Transfer events
- `v3_pool_swaps` - Uniswap V3 Swap events

**Tests:**

| Test | Cassette | Validates |
|------|----------|-----------|
| `test_endpoint_weth_transfers` | `endpoint_weth_transfers.json` | Single-table SELECT with pagination |
| `test_endpoint_cross_contract_whales` | `endpoint_cross_contract_whales.json` | Multi-table JOIN query |
| `test_endpoint_swap_volume_hourly` | `endpoint_swap_volume_hourly.json` | GROUP BY with aggregation |
| `test_endpoint_v3_swaps_by_pool` | `endpoint_v3_swaps_by_pool.json` | Path parameters with parameterized SQL |

**Assertions per test:**
- `endpoint_path` matches request
- `method` is "GET"
- `tables_referenced` contains expected tables
- `sql_query` contains appropriate SQL patterns
- `query_params` includes pagination (limit)
- `path_params` extracted for path variables

---

## End-to-End Tests

### Purpose

Validate complete runtime flow: database creation, server startup, HTTP requests, SQL execution.

### Location

```
tests/e2e/
├── main.rs                 # Test harness (TestServer, E2eTestServer)
├── health_test.rs          # Lightweight tests (no Docker)
├── full_flow_test.rs       # Full tests with testcontainers
├── record_rpc.rs           # RPC cassette recording
└── fixtures/
    ├── weth_spec_ir.json           # Event specification
    ├── weth_transfers_endpoint.json # Endpoint definition
    └── cassettes/rpc/              # Mocked Ethereum RPC
        ├── eth_blockNumber.json
        ├── eth_getLogs_weth.json
        └── eth_getBlockByNumber.json
```

### Lightweight Tests (No Docker)

**File:** `tests/e2e/health_test.rs`

**Harness:** `TestServer` - Minimal Axum server on random port

```rust
pub struct TestServer {
    pub url: String,
    handle: JoinHandle<()>,
}

impl TestServer {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind");
        let port = listener.local_addr().unwrap().port();
        // ... spawn server, wait for ready
    }
}
```

**Tests:**

| Test | Endpoint | Validates |
|------|----------|-----------|
| `test_health_endpoint_returns_200` | `GET /health` | Returns `{"status": "healthy", "service": "smorty-indexer"}` |
| `test_root_endpoint_returns_200` | `GET /` | Returns text containing "smorty" |
| `test_swagger_ui_returns_html` | `GET /swagger-ui/` | Returns HTML with "swagger" |
| `test_openapi_spec_returns_valid_json` | `GET /api-docs/openapi.json` | Returns valid OpenAPI with info, paths, /health |

### Full E2E Tests (Docker Required)

**File:** `tests/e2e/full_flow_test.rs`

**Harness:** `E2eTestServer` - Complete environment with:
- Testcontainers Postgres
- Wiremock RPC mock
- Temporary config file
- Real smorty server process

```rust
pub struct E2eTestServer {
    pub url: String,
    pub db_url: String,
    server_process: Option<Child>,
    _container: ContainerAsync<Postgres>,
    _rpc_mock: MockServer,
    _temp_dir: TempDir,
}
```

**Lifecycle:**

```
1. Start Postgres container (testcontainers)
2. Start Wiremock server, mount RPC cassettes
3. Create temp directory with config.toml
4. Write IR fixtures to ir/specs/ and ir/endpoints/
5. Generate migration SQL from spec
6. Run migrations via `smorty migrate`
7. Run indexer via `smorty index` (populates from mocked RPC)
8. Start server via `smorty serve`
9. Wait for /health to return 200
10. Run tests
11. Cleanup on drop
```

**RPC Mocking:**

```rust
async fn mount_rpc_cassettes(mock_server: &MockServer) -> Result<()> {
    // eth_blockNumber - returns current block
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"method": "eth_blockNumber"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response))
        .mount(mock_server).await;

    // eth_getLogs - returns WETH transfer events
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"method": "eth_getLogs"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(&logs_response))
        .mount(mock_server).await;

    // eth_getBlockByNumber - returns block timestamps
    // ...
}
```

**Tests:**

| Test | Endpoint | Validates |
|------|----------|-----------|
| `test_e2e_health_endpoint` | `GET /health` | Full server health check |
| `test_e2e_swagger_ui` | `GET /swagger-ui/` | Swagger UI from real server |
| `test_e2e_openapi_spec` | `GET /api-docs/openapi.json` | Generated spec includes dynamic endpoint |
| `test_e2e_weth_transfers_endpoint` | `GET /api/weth/transfers` | Dynamic endpoint executes SQL, returns JSON |
| `test_e2e_weth_transfers_with_limit` | `GET /api/weth/transfers?limit=1` | Query parameter handling |

---

## Continuous Integration

### File: `.github/workflows/ci.yml`

### Pipeline Stages

```yaml
jobs:
  check:
    steps:
      - cargo fmt --all -- --check
      - cargo clippy --all-targets --all-features

  build:
    needs: check
    steps:
      - cargo build --release

  test:
    needs: check
    steps:
      - cargo test --all-features -- --test-threads=1
```

### Test Execution

- **Format Check** - Ensures consistent code style
- **Clippy** - Static analysis for common mistakes
- **Tests** - All tests including e2e (Docker available in GitHub Actions)
- **Serial Execution** - `--test-threads=1` prevents race conditions in migration tests

---

## Test Fixtures

### Integration Test Fixtures

**ABIs** (`tests/integration/fixtures/abi/`):
- `weth.json` - Wrapped Ether contract
- `uni.json` - Uniswap governance token
- `uniswap_v3_pool.json` - V3 liquidity pool
- `uniswap_v3_factory.json` - V3 pool factory
- `usdc.json` - USD Coin (reference)

**Cassettes** (`tests/integration/fixtures/cassettes/`):

Event IR cassettes (recorded OpenAI responses):
- `weth_transfer.json`
- `weth_deposit.json`
- `uni_transfer.json`
- `uni_delegate_votes.json`
- `v3_pool_swap.json`
- `v3_pool_mint.json`
- `v3_factory_pool_created.json`

Endpoint IR cassettes:
- `endpoint_weth_transfers.json`
- `endpoint_v3_swaps_by_pool.json`
- `endpoint_cross_contract_whales.json`
- `endpoint_swap_volume_hourly.json`

### E2E Test Fixtures

**Spec IR** (`tests/e2e/fixtures/weth_spec_ir.json`):
```json
{
  "event_name": "Transfer",
  "event_signature": "Transfer(address,address,uint256)",
  "table_schema": {
    "table_name": "weth_transfers",
    "columns": [
      {"name": "id", "type": "BIGSERIAL PRIMARY KEY"},
      {"name": "block_number", "type": "BIGINT NOT NULL"},
      ...
    ]
  }
}
```

**Endpoint IR** (`tests/e2e/fixtures/weth_transfers_endpoint.json`):
```json
{
  "endpoint_path": "/api/weth/transfers",
  "method": "GET",
  "query_params": [
    {"name": "address", "type": "Option<String>", "default": "null"},
    {"name": "limit", "type": "u32", "default": 50}
  ],
  "sql_query": "SELECT ... WHERE ($1::TEXT IS NULL OR src = $1::TEXT OR dst = $1::TEXT) ORDER BY block_timestamp DESC LIMIT $2"
}
```

**RPC Cassettes** (`tests/e2e/fixtures/cassettes/rpc/`):
- `eth_blockNumber.json` - Current block height
- `eth_getLogs_weth.json` - WETH Transfer events (500+ events)
- `eth_getBlockByNumber.json` - Block timestamps for each event

---

## Running Tests

### Quick Reference

```bash
# All unit tests
cargo test

# Integration tests only (mocked OpenAI)
cargo test --test integration

# Lightweight e2e (no Docker)
cargo test --test e2e

# Full e2e with testcontainers (Docker required)
cargo test --features e2e --test e2e -- --test-threads=1

# Everything
cargo test --features e2e -- --test-threads=1
```

### Recording New Cassettes

**OpenAI cassettes** (requires API key):
```bash
OPENAI_API_KEY=sk-xxx cargo test --test integration record_ -- --ignored --nocapture
```

**RPC cassettes** (requires Ethereum node):
```bash
ETH_RPC_URL=https://eth-mainnet.g.alchemy.com/v2/xxx \
  cargo test --features e2e --test e2e record_rpc -- --ignored --nocapture
```

### Troubleshooting

**Tests hang or timeout:**
- Ensure Docker is running (for e2e tests)
- Check `--test-threads=1` is set (migration tests require serial execution)

**Cassette not found:**
- Verify fixture path matches test expectation
- Run recording tests to regenerate

**Type mismatch errors in SQL:**
- Check parameter casts in endpoint IR SQL queries
- Ensure `$N::TEXT` casts for nullable string parameters

---

## Appendix: Test Count Summary

| Category | Count | Location |
|----------|-------|----------|
| Unit tests | 66 | `src/*.rs` |
| Integration tests | 11 | `tests/integration/` |
| E2E tests (light) | 4 | `tests/e2e/health_test.rs` |
| E2E tests (full) | 5 | `tests/e2e/full_flow_test.rs` |
| **Total** | **86** | |
