use crate::ai::IrGenerationResult;
use crate::config::Config;
use crate::ir::Ir;
use crate::schema_state::SchemaState;
use alloy::primitives::{Address, FixedBytes};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::rpc::types::{Filter, Log};
use anyhow::{Context, Result};
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use tokio::time::{Duration, interval};

/// Represents a single event spec to index
#[derive(Debug, Clone)]
struct IndexSpec {
    contract_name: String,
    spec_name: String,
    ir: IrGenerationResult,
}

/// Group of specs organized by chain for efficient indexing
#[derive(Debug)]
struct ChainGroup {
    chain: String,
    rpc_url: String,
    specs: Vec<IndexSpec>,
    min_start_block: u64,
}

/// Main indexer struct that manages the indexing process
pub struct Indexer {
    config: Arc<Config>,
    db_pool: PgPool,
    schema: SchemaState,
}

impl Indexer {
    /// Create a new indexer instance
    pub async fn new(config: &Config) -> Result<Self> {
        // Connect to the database
        let db_pool = PgPool::connect(&config.database.uri)
            .await
            .context("Failed to connect to database")?;

        // Load schema state
        let schema = SchemaState::load(Path::new("migrations/schema.json"))
            .context("Failed to load migrations/schema.json")?;

        Ok(Self {
            config: Arc::new(config.clone()),
            db_pool,
            schema,
        })
    }

    /// Start the indexer
    pub async fn start(&self, daemon: bool) -> Result<()> {
        tracing::info!("Loading IR files...");
        let ir_specs = Ir::load_all_ir_specs(&self.config)?;
        tracing::info!("Loaded {} IR specs", ir_specs.len());

        // Group specs by chain for efficient indexing
        let chain_groups = self.group_specs_by_chain(ir_specs)?;
        tracing::info!("Organized into {} chain groups", chain_groups.len());

        for group in &chain_groups {
            tracing::info!(
                "Chain '{}': {} specs, starting from block {}",
                group.chain,
                group.specs.len(),
                group.min_start_block
            );
        }

        if daemon {
            self.run_daemon_mode(chain_groups).await
        } else {
            self.run_once(chain_groups).await
        }
    }

    /// Group IR specs by chain for efficient processing
    fn group_specs_by_chain(
        &self,
        ir_specs: Vec<(String, String, IrGenerationResult)>,
    ) -> Result<Vec<ChainGroup>> {
        let mut chain_map: HashMap<String, Vec<IndexSpec>> = HashMap::new();

        // Group specs by chain
        for (contract_name, spec_name, ir) in ir_specs {
            let spec = IndexSpec {
                contract_name,
                spec_name,
                ir,
            };

            chain_map
                .entry(spec.ir.chain.clone())
                .or_insert_with(Vec::new)
                .push(spec);
        }

        // Convert to ChainGroup with RPC URLs and min start blocks
        let mut groups = Vec::new();
        for (chain, specs) in chain_map {
            let rpc_url = self.config.get_rpc_url(&chain)?.clone();

            // Find minimum start block across all specs for this chain
            let min_start_block = specs.iter().map(|s| s.ir.start_block).min().unwrap_or(0);

            groups.push(ChainGroup {
                chain,
                rpc_url,
                specs,
                min_start_block,
            });
        }

        Ok(groups)
    }

    /// Run indexer once (historical sync only)
    async fn run_once(&self, chain_groups: Vec<ChainGroup>) -> Result<()> {
        tracing::info!("Running indexer in one-time mode");

        for group in chain_groups {
            tracing::info!(
                "Indexing chain '{}' with {} specs",
                group.chain,
                group.specs.len()
            );

            if let Err(e) = self.index_chain_group(&group).await {
                tracing::error!("Failed to index chain '{}': {:?}", group.chain, e);
                return Err(e);
            }
        }

        tracing::info!("One-time indexing complete");
        Ok(())
    }

    /// Run indexer in daemon mode (continuous monitoring)
    async fn run_daemon_mode(&self, chain_groups: Vec<ChainGroup>) -> Result<()> {
        tracing::info!("Running indexer in daemon mode");

        // Create tasks for each chain
        let mut tasks = Vec::new();

        for group in chain_groups {
            let indexer = Self {
                config: Arc::clone(&self.config),
                db_pool: self.db_pool.clone(),
                schema: self.schema.clone(),
            };

            let task = tokio::spawn(async move {
                tracing::info!(
                    "Starting daemon for chain '{}' with {} specs",
                    group.chain,
                    group.specs.len()
                );

                // Poll every 12 seconds (approximately 1 block on Ethereum)
                let mut ticker = interval(Duration::from_secs(12));

                loop {
                    ticker.tick().await;

                    if let Err(e) = indexer.index_chain_group(&group).await {
                        tracing::error!("Error indexing chain '{}': {:?}", group.chain, e);
                        // Continue despite errors
                    }
                }
            });

            tasks.push(task);
        }

        // Wait for all tasks (they should run forever)
        #[allow(clippy::never_loop)] // Intentional: daemon tasks run forever
        for task in tasks {
            task.await?;
        }

        Ok(())
    }

    /// Index all specs for a single chain in one pass
    async fn index_chain_group(&self, group: &ChainGroup) -> Result<()> {
        // Create provider
        let provider = ProviderBuilder::new()
            .connect_http(group.rpc_url.parse().context("Invalid RPC URL")?)
            .root()
            .clone();

        // Get current block number
        let current_block = provider
            .get_block_number()
            .await
            .context("Failed to get current block number")?;

        // For each spec, check last indexed block and determine where to start
        // We need to find the MINIMUM start block to ensure we don't miss any events
        let mut spec_start_blocks: Vec<(usize, u64)> = Vec::new(); // (spec_index, start_block)

        for (idx, spec) in group.specs.iter().enumerate() {
            let last_indexed = self
                .get_last_indexed_block(&spec.ir.table_schema.table_name)
                .await?;

            let spec_start = if last_indexed > 0 {
                // Resume from where we left off
                last_indexed + 1
            } else {
                // Start from the configured start block
                spec.ir.start_block
            };

            spec_start_blocks.push((idx, spec_start));

            tracing::debug!(
                "  - {}/{}: starting from block {} (last indexed: {})",
                spec.contract_name,
                spec.spec_name,
                spec_start,
                if last_indexed > 0 {
                    last_indexed.to_string()
                } else {
                    "none".to_string()
                }
            );
        }

        // Find the minimum start block across all specs
        // This ensures we fetch logs from the earliest point needed
        let start_block = spec_start_blocks
            .iter()
            .map(|(_, block)| *block)
            .min()
            .unwrap_or(group.min_start_block);

        // If we're already caught up, nothing to do
        if start_block > current_block {
            tracing::debug!(
                "Already caught up for chain '{}' (current: {}, start: {})",
                group.chain,
                current_block,
                start_block
            );
            return Ok(());
        }

        tracing::info!(
            "Indexing chain '{}' from block {} to {} ({} blocks)",
            group.chain,
            start_block,
            current_block,
            current_block - start_block + 1
        );

        // Build a map of contract addresses to their specs
        let mut contract_spec_map: HashMap<Address, Vec<&IndexSpec>> = HashMap::new();
        for spec in &group.specs {
            let address =
                Address::from_str(&spec.ir.contract_address).context("Invalid contract address")?;
            contract_spec_map
                .entry(address)
                .or_insert_with(Vec::new)
                .push(spec);
        }

        // Collect all contract addresses
        let addresses: Vec<Address> = contract_spec_map.keys().copied().collect();

        // Fetch logs in chunks to avoid RPC limits
        const CHUNK_SIZE: u64 = 1000;
        let mut from_block = start_block;

        while from_block <= current_block {
            let to_block = std::cmp::min(from_block + CHUNK_SIZE - 1, current_block);

            tracing::debug!(
                "Fetching logs for chain '{}' from block {} to {}",
                group.chain,
                from_block,
                to_block
            );

            // Create filter for all contracts on this chain
            let filter = Filter::new()
                .address(addresses.clone())
                .from_block(from_block)
                .to_block(to_block);

            // Fetch logs
            let logs = provider
                .get_logs(&filter)
                .await
                .context("Failed to fetch logs")?;

            tracing::debug!("Found {} logs for chain '{}'", logs.len(), group.chain);

            // Process each log
            for log in logs {
                // Determine which spec(s) this log belongs to
                let address = log.address();
                if let Some(specs) = contract_spec_map.get(&address) {
                    // Match log to the correct spec based on event signature
                    for spec in specs.iter() {
                        // Check if this log matches this spec's event signature
                        if self.log_matches_spec(&log, &spec.ir) {
                            // Find the spec's start block from our tracking
                            let spec_start = group
                                .specs
                                .iter()
                                .position(|s| {
                                    s.contract_name == spec.contract_name
                                        && s.spec_name == spec.spec_name
                                })
                                .and_then(|idx| {
                                    spec_start_blocks
                                        .iter()
                                        .find(|(i, _)| *i == idx)
                                        .map(|(_, block)| *block)
                                })
                                .unwrap_or(0);

                            // Check if this log is within the range for this specific spec
                            if let Some(log_block) = log.block_number {
                                if log_block < spec_start {
                                    // Skip this log - it's before this spec's start block
                                    tracing::trace!(
                                        "Skipping log for {}/{} at block {} (spec starts at {})",
                                        spec.contract_name,
                                        spec.spec_name,
                                        log_block,
                                        spec_start
                                    );
                                    break;
                                }
                            }

                            if let Err(e) = self.process_log(&log, &spec.ir).await {
                                tracing::warn!(
                                    "Skipping log for {}/{} due to error (this can happen with unreliable chains): {:?}",
                                    spec.contract_name,
                                    spec.spec_name,
                                    e
                                );
                                // Continue processing other logs
                            }
                            // A log can only match one event signature, so break
                            break;
                        }
                    }
                }
            }

            from_block = to_block + 1;
        }

        tracing::info!(
            "Successfully indexed chain '{}' up to block {}",
            group.chain,
            current_block
        );

        Ok(())
    }

    /// Check if a log matches a spec's event signature
    fn log_matches_spec(&self, log: &Log, ir: &IrGenerationResult) -> bool {
        // The first topic is the event signature hash
        if log.topics().is_empty() {
            return false;
        }

        let event_signature_hash = self.calculate_event_signature_hash(&ir.event_signature);
        let log_topic = log.topics()[0];

        event_signature_hash == log_topic
    }

    /// Calculate the Keccak-256 hash of an event signature
    fn calculate_event_signature_hash(&self, signature: &str) -> FixedBytes<32> {
        use alloy::primitives::keccak256;
        keccak256(signature.as_bytes())
    }

    /// Get the last indexed block number for a table
    async fn get_last_indexed_block(&self, table_name: &str) -> Result<u64> {
        let query = format!(
            "SELECT COALESCE(MAX(block_number), 0) as max_block FROM {}",
            table_name
        );

        let row = sqlx::query(&query)
            .fetch_one(&self.db_pool)
            .await
            .context("Failed to query last indexed block")?;

        let max_block: i64 = row.try_get("max_block")?;

        Ok(max_block as u64)
    }

    /// Process a single log and insert into database
    async fn process_log(&self, log: &Log, ir: &IrGenerationResult) -> Result<()> {
        // Get block details - if any are missing, skip this log gracefully
        let block_number = match log.block_number {
            Some(bn) => bn,
            None => {
                return Err(anyhow::anyhow!("Log missing block number"));
            }
        };

        let block_timestamp = match log.block_timestamp {
            Some(ts) => ts,
            None => {
                return Err(anyhow::anyhow!("Log missing block timestamp"));
            }
        };

        let tx_hash = match log.transaction_hash {
            Some(hash) => hash,
            None => {
                return Err(anyhow::anyhow!("Log missing transaction hash"));
            }
        };

        let log_index = match log.log_index {
            Some(idx) => idx,
            None => {
                return Err(anyhow::anyhow!("Log missing log index"));
            }
        };

        // Decode event data (returns field name -> value)
        let decoded_values = match self.decode_event_data(log, ir) {
            Ok(values) => values,
            Err(e) => {
                return Err(anyhow::anyhow!("Failed to decode event data: {}", e));
            }
        };

        // Get the table schema from migrations/schema.json
        let table_schema = match self.schema.get_table(&ir.table_schema.table_name) {
            Some(schema) => schema,
            None => {
                return Err(anyhow::anyhow!(
                    "Table '{}' not found in migrations/schema.json",
                    ir.table_schema.table_name
                ));
            }
        };

        // Build a map of field names to their order in indexed_fields
        let mut field_order: HashMap<&str, usize> = HashMap::new();
        for (idx, field) in ir.indexed_fields.iter().enumerate() {
            field_order.insert(&field.name, idx);
        }

        // Build INSERT query using actual column names from schema
        let mut columns = vec![
            "block_number".to_string(),
            "block_timestamp".to_string(),
            "transaction_hash".to_string(),
            "log_index".to_string(),
        ];

        let mut values: Vec<String> = vec![
            block_number.to_string(),
            block_timestamp.to_string(),
            format!("'{:#x}'", tx_hash),
            log_index.to_string(),
        ];

        // Add event-specific fields using the column names from migrations/schema.json
        // Iterate through columns in the schema (excluding standard columns)
        for column in &table_schema.columns {
            if !matches!(
                column.name.as_str(),
                "id" | "block_number" | "block_timestamp" | "transaction_hash" | "log_index"
            ) {
                // Find the corresponding value from decoded_values
                // We need to match by position since field names might differ
                let field_idx = columns.len() - 4; // Offset by the 4 standard columns
                if field_idx < decoded_values.len() {
                    columns.push(column.name.clone());
                    values.push(decoded_values[field_idx].1.clone());
                }
            }
        }

        let insert_query = format!(
            "INSERT INTO {} ({}) VALUES ({}) ON CONFLICT DO NOTHING",
            ir.table_schema.table_name,
            columns.join(", "),
            values.join(", ")
        );

        match sqlx::query(&insert_query).execute(&self.db_pool).await {
            Ok(_) => {}
            Err(e) => {
                return Err(anyhow::anyhow!("Failed to insert log into database: {}", e));
            }
        }

        tracing::debug!(
            "Inserted log for {} at block {} (tx: {:#x})",
            ir.event_name,
            block_number,
            tx_hash
        );

        Ok(())
    }

    /// Decode event data from a log
    /// This uses alloy's built-in ABI decoding capabilities
    fn decode_event_data(
        &self,
        log: &Log,
        ir: &IrGenerationResult,
    ) -> Result<Vec<(String, String)>> {
        let mut result = Vec::new();

        // Topics: [event_signature, indexed_param_1, indexed_param_2, ...]
        // Data: concatenated non-indexed parameters

        let topics = log.topics();
        let mut topic_index = 1; // Skip first topic (event signature)

        let data = log.data().data.clone();
        let mut data_offset = 0;

        for field in &ir.indexed_fields {
            let value_str = if field.indexed && topic_index < topics.len() {
                // Indexed field - get from topics
                let topic = topics[topic_index];
                topic_index += 1;

                // Format based on Solidity type
                self.format_topic_value(&topic, &field.solidity_type)?
            } else {
                // Non-indexed field - get from data
                let value =
                    self.extract_data_value(&data, &mut data_offset, &field.solidity_type)?;
                value
            };

            result.push((field.name.clone(), value_str));
        }

        Ok(result)
    }

    /// Format a topic value based on its Solidity type
    fn format_topic_value(&self, topic: &FixedBytes<32>, solidity_type: &str) -> Result<String> {
        let value = match solidity_type {
            "address" => {
                // Address is stored in last 20 bytes of the topic
                let bytes = &topic.as_slice()[12..];
                let addr = Address::from_slice(bytes);
                format!("'{:#x}'", addr)
            }
            "bool" => {
                // Bool is stored as 0 or 1
                let is_true = topic.as_slice().iter().any(|&b| b != 0);
                if is_true {
                    "true".to_string()
                } else {
                    "false".to_string()
                }
            }
            t if t.starts_with("uint") || t.starts_with("int") => {
                // Integer types - convert to decimal string
                let value = alloy::primitives::U256::from_be_bytes(topic.0);
                format!("'{}'", value)
            }
            t if t.starts_with("bytes") => {
                // Fixed-size bytes
                format!("'{:#x}'", topic)
            }
            _ => {
                // Default: hex representation
                format!("'{:#x}'", topic)
            }
        };

        Ok(value)
    }

    /// Extract a value from the data field
    fn extract_data_value(
        &self,
        data: &[u8],
        offset: &mut usize,
        solidity_type: &str,
    ) -> Result<String> {
        // All values in data are 32-byte aligned
        const WORD_SIZE: usize = 32;

        if *offset + WORD_SIZE > data.len() {
            // Not enough data - return NULL
            return Ok("NULL".to_string());
        }

        let word = &data[*offset..*offset + WORD_SIZE];
        *offset += WORD_SIZE;

        let value = match solidity_type {
            "address" => {
                // Address is in last 20 bytes
                let bytes = &word[12..];
                let addr = Address::from_slice(bytes);
                format!("'{:#x}'", addr)
            }
            "bool" => {
                // Bool is in last byte
                let is_true = word.iter().any(|&b| b != 0);
                if is_true {
                    "true".to_string()
                } else {
                    "false".to_string()
                }
            }
            t if t.starts_with("uint") || t.starts_with("int") => {
                // Integer types
                let value = alloy::primitives::U256::from_be_slice(word);
                format!("'{}'", value)
            }
            t if t.starts_with("bytes") && t.len() > 5 => {
                // Fixed-size bytes (bytesN)
                if let Ok(size) = t[5..].parse::<usize>() {
                    let bytes = &word[..size];
                    format!("'\\x{}'", hex::encode(bytes))
                } else {
                    format!("'\\x{}'", hex::encode(word))
                }
            }
            "string" | "bytes" => {
                // Dynamic types - would need to handle offsets
                // For now, return hex representation
                format!("'\\x{}'", hex::encode(word))
            }
            _ => {
                // Default: hex representation
                format!("'\\x{}'", hex::encode(word))
            }
        };

        Ok(value)
    }
}
