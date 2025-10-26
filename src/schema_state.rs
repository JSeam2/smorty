use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Represents the state of a database schema at a point in time
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SchemaState {
    /// Map of table_name -> TableState
    pub tables: HashMap<String, TableState>,
    /// Timestamp when this state was captured
    pub timestamp: String,
}

/// State of a single table
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TableState {
    /// Table name
    pub name: String,
    /// Contract and spec that generated this table
    pub source: TableSource,
    /// Columns in this table
    pub columns: Vec<ColumnState>,
    /// Indexes on this table
    pub indexes: Vec<IndexState>,
}

/// Source information for a table
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TableSource {
    pub contract_name: String,
    pub spec_name: String,
}

/// State of a single column
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ColumnState {
    pub name: String,
    pub column_type: String,
}

/// State of a single index
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IndexState {
    pub name: String,
    pub definition: String,
}

impl SchemaState {
    /// Create a new empty schema state
    pub fn new() -> Self {
        Self {
            tables: HashMap::new(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Load schema state from file
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::new());
        }

        let content = fs::read_to_string(path)
            .context(format!("Failed to read schema state file: {:?}", path))?;

        let state: SchemaState = serde_json::from_str(&content)
            .context("Failed to parse schema state JSON")?;

        Ok(state)
    }

    /// Save schema state to file
    pub fn save(&self, path: &Path) -> Result<()> {
        let content = serde_json::to_string_pretty(self)
            .context("Failed to serialize schema state")?;

        fs::write(path, content)
            .context(format!("Failed to write schema state file: {:?}", path))?;

        Ok(())
    }

    /// Add or update a table in the schema state
    pub fn add_table(&mut self, table: TableState) {
        self.tables.insert(table.name.clone(), table);
    }

    /// Remove a table from the schema state
    pub fn remove_table(&mut self, table_name: &str) {
        self.tables.remove(table_name);
    }

    /// Get a table by name
    pub fn get_table(&self, table_name: &str) -> Option<&TableState> {
        self.tables.get(table_name)
    }
}

impl Default for SchemaState {
    fn default() -> Self {
        Self::new()
    }
}

impl TableState {
    /// Create a new table state
    pub fn new(name: String, contract_name: String, spec_name: String) -> Self {
        Self {
            name,
            source: TableSource {
                contract_name,
                spec_name,
            },
            columns: Vec::new(),
            indexes: Vec::new(),
        }
    }

    /// Add a column to the table
    pub fn add_column(&mut self, column: ColumnState) {
        self.columns.push(column);
    }

    /// Add an index to the table
    pub fn add_index(&mut self, index: IndexState) {
        self.indexes.push(index);
    }

    /// Find a column by name
    pub fn get_column(&self, name: &str) -> Option<&ColumnState> {
        self.columns.iter().find(|c| c.name == name)
    }

    /// Find an index by name
    pub fn get_index(&self, name: &str) -> Option<&IndexState> {
        self.indexes.iter().find(|i| i.name == name)
    }
}

impl ColumnState {
    /// Create a new column state
    pub fn new(name: String, column_type: String) -> Self {
        Self { name, column_type }
    }
}

impl IndexState {
    /// Create a new index state
    pub fn new(name: String, definition: String) -> Self {
        Self { name, definition }
    }

    /// Extract index name from CREATE INDEX statement
    pub fn extract_index_name(create_index_sql: &str) -> Option<String> {
        // Parse "CREATE INDEX idx_name ON table(...)"
        let parts: Vec<&str> = create_index_sql.split_whitespace().collect();
        if parts.len() >= 3 && parts[0] == "CREATE" && parts[1] == "INDEX" {
            return Some(parts[2].to_string());
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_schema_state_new() {
        let state = SchemaState::new();
        assert!(state.tables.is_empty());
        assert!(!state.timestamp.is_empty());
    }

    #[test]
    fn test_schema_state_save_and_load() {
        let temp_dir = TempDir::new().unwrap();
        let state_file = temp_dir.path().join("schema_state.json");

        // Create a state with some data
        let mut state = SchemaState::new();
        let mut table = TableState::new(
            "test_table".to_string(),
            "TestContract".to_string(),
            "TestEvent".to_string(),
        );
        table.add_column(ColumnState::new("id".to_string(), "BIGSERIAL PRIMARY KEY".to_string()));
        table.add_column(ColumnState::new("name".to_string(), "TEXT NOT NULL".to_string()));
        table.add_index(IndexState::new(
            "idx_name".to_string(),
            "CREATE INDEX idx_name ON test_table(name)".to_string(),
        ));
        state.add_table(table);

        // Save
        state.save(&state_file).unwrap();
        assert!(state_file.exists());

        // Load
        let loaded_state = SchemaState::load(&state_file).unwrap();
        assert_eq!(loaded_state.tables.len(), 1);

        let loaded_table = loaded_state.get_table("test_table").unwrap();
        assert_eq!(loaded_table.name, "test_table");
        assert_eq!(loaded_table.columns.len(), 2);
        assert_eq!(loaded_table.indexes.len(), 1);
    }

    #[test]
    fn test_schema_state_load_nonexistent() {
        let temp_dir = TempDir::new().unwrap();
        let state_file = temp_dir.path().join("nonexistent.json");

        let state = SchemaState::load(&state_file).unwrap();
        assert!(state.tables.is_empty());
    }

    #[test]
    fn test_table_state_operations() {
        let mut table = TableState::new(
            "users".to_string(),
            "UserContract".to_string(),
            "UserCreated".to_string(),
        );

        // Add columns
        table.add_column(ColumnState::new("id".to_string(), "BIGSERIAL PRIMARY KEY".to_string()));
        table.add_column(ColumnState::new("email".to_string(), "TEXT NOT NULL".to_string()));

        // Find columns
        assert!(table.get_column("id").is_some());
        assert!(table.get_column("email").is_some());
        assert!(table.get_column("nonexistent").is_none());

        // Add index
        table.add_index(IndexState::new(
            "idx_email".to_string(),
            "CREATE INDEX idx_email ON users(email)".to_string(),
        ));

        // Find index
        assert!(table.get_index("idx_email").is_some());
        assert!(table.get_index("nonexistent").is_none());
    }

    #[test]
    fn test_index_name_extraction() {
        let sql = "CREATE INDEX idx_test ON table_name(column)";
        let name = IndexState::extract_index_name(sql);
        assert_eq!(name, Some("idx_test".to_string()));

        let invalid_sql = "SELECT * FROM table";
        let name = IndexState::extract_index_name(invalid_sql);
        assert_eq!(name, None);
    }

    #[test]
    fn test_add_and_remove_table() {
        let mut state = SchemaState::new();

        let table = TableState::new(
            "test_table".to_string(),
            "TestContract".to_string(),
            "TestEvent".to_string(),
        );

        state.add_table(table);
        assert_eq!(state.tables.len(), 1);
        assert!(state.get_table("test_table").is_some());

        state.remove_table("test_table");
        assert_eq!(state.tables.len(), 0);
        assert!(state.get_table("test_table").is_none());
    }
}
