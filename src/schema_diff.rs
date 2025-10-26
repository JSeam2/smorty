use crate::schema_state::{ColumnState, IndexState, SchemaState, TableState};
use std::collections::{HashMap, HashSet};

/// Represents changes between two schema states
#[derive(Debug, Clone, PartialEq)]
pub struct SchemaDiff {
    /// Tables that exist in new state but not in old state
    pub tables_added: Vec<TableState>,
    /// Tables that exist in old state but not in new state
    pub tables_dropped: Vec<String>,
    /// Tables that exist in both but have changes
    pub tables_modified: Vec<TableDiff>,
}

/// Represents changes to a single table
#[derive(Debug, Clone, PartialEq)]
pub struct TableDiff {
    pub table_name: String,
    pub columns_added: Vec<ColumnState>,
    pub columns_dropped: Vec<String>,
    pub columns_modified: Vec<ColumnModification>,
    pub indexes_added: Vec<IndexState>,
    pub indexes_dropped: Vec<String>,
}

/// Represents a modification to a column
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnModification {
    pub column_name: String,
    pub old_type: String,
    pub new_type: String,
}

impl SchemaDiff {
    /// Compute the difference between two schema states
    pub fn compute(old_state: &SchemaState, new_state: &SchemaState) -> Self {
        let old_tables: HashSet<String> = old_state.tables.keys().cloned().collect();
        let new_tables: HashSet<String> = new_state.tables.keys().cloned().collect();

        // Find added and dropped tables
        let added_table_names: Vec<String> = new_tables.difference(&old_tables).cloned().collect();
        let dropped_table_names: Vec<String> = old_tables.difference(&new_tables).cloned().collect();

        let tables_added: Vec<TableState> = added_table_names
            .iter()
            .filter_map(|name| new_state.get_table(name).cloned())
            .collect();

        let tables_dropped = dropped_table_names;

        // Find modified tables (tables that exist in both states)
        let common_tables: HashSet<String> = old_tables.intersection(&new_tables).cloned().collect();
        let mut tables_modified = Vec::new();

        for table_name in common_tables {
            let old_table = old_state.get_table(&table_name).unwrap();
            let new_table = new_state.get_table(&table_name).unwrap();

            let table_diff = Self::compute_table_diff(old_table, new_table);

            // Only include if there are actual changes
            if table_diff.has_changes() {
                tables_modified.push(table_diff);
            }
        }

        Self {
            tables_added,
            tables_dropped,
            tables_modified,
        }
    }

    /// Compute differences for a single table
    fn compute_table_diff(old_table: &TableState, new_table: &TableState) -> TableDiff {
        // Build maps for easier lookup
        let old_columns: HashMap<String, &ColumnState> = old_table
            .columns
            .iter()
            .map(|c| (c.name.clone(), c))
            .collect();

        let new_columns: HashMap<String, &ColumnState> = new_table
            .columns
            .iter()
            .map(|c| (c.name.clone(), c))
            .collect();

        let old_indexes: HashMap<String, &IndexState> = old_table
            .indexes
            .iter()
            .map(|i| (i.name.clone(), i))
            .collect();

        let new_indexes: HashMap<String, &IndexState> = new_table
            .indexes
            .iter()
            .map(|i| (i.name.clone(), i))
            .collect();

        // Compute column changes
        let old_col_names: HashSet<String> = old_columns.keys().cloned().collect();
        let new_col_names: HashSet<String> = new_columns.keys().cloned().collect();

        let columns_added: Vec<ColumnState> = new_col_names
            .difference(&old_col_names)
            .filter_map(|name| new_columns.get(name).map(|c| (*c).clone()))
            .collect();

        let columns_dropped: Vec<String> = old_col_names
            .difference(&new_col_names)
            .cloned()
            .collect();

        // Check for modified columns (same name, different type)
        let common_columns: HashSet<String> = old_col_names.intersection(&new_col_names).cloned().collect();
        let mut columns_modified = Vec::new();

        for col_name in common_columns {
            let old_col = old_columns.get(&col_name).unwrap();
            let new_col = new_columns.get(&col_name).unwrap();

            if old_col.column_type != new_col.column_type {
                columns_modified.push(ColumnModification {
                    column_name: col_name,
                    old_type: old_col.column_type.clone(),
                    new_type: new_col.column_type.clone(),
                });
            }
        }

        // Compute index changes
        let old_idx_names: HashSet<String> = old_indexes.keys().cloned().collect();
        let new_idx_names: HashSet<String> = new_indexes.keys().cloned().collect();

        let indexes_added: Vec<IndexState> = new_idx_names
            .difference(&old_idx_names)
            .filter_map(|name| new_indexes.get(name).map(|i| (*i).clone()))
            .collect();

        let indexes_dropped: Vec<String> = old_idx_names
            .difference(&new_idx_names)
            .cloned()
            .collect();

        TableDiff {
            table_name: new_table.name.clone(),
            columns_added,
            columns_dropped,
            columns_modified,
            indexes_added,
            indexes_dropped,
        }
    }

    /// Check if there are any changes
    pub fn has_changes(&self) -> bool {
        !self.tables_added.is_empty()
            || !self.tables_dropped.is_empty()
            || !self.tables_modified.is_empty()
    }

    /// Check if this is an initial migration (no previous state)
    pub fn is_initial(&self) -> bool {
        !self.tables_added.is_empty()
            && self.tables_dropped.is_empty()
            && self.tables_modified.is_empty()
    }
}

impl TableDiff {
    /// Check if there are any changes to this table
    pub fn has_changes(&self) -> bool {
        !self.columns_added.is_empty()
            || !self.columns_dropped.is_empty()
            || !self.columns_modified.is_empty()
            || !self.indexes_added.is_empty()
            || !self.indexes_dropped.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema_state::TableSource;

    fn create_test_table(name: &str, columns: Vec<(&str, &str)>, indexes: Vec<(&str, &str)>) -> TableState {
        let mut table = TableState {
            name: name.to_string(),
            source: TableSource {
                contract_name: "TestContract".to_string(),
                spec_name: "TestEvent".to_string(),
            },
            columns: Vec::new(),
            indexes: Vec::new(),
        };

        for (col_name, col_type) in columns {
            table.add_column(ColumnState::new(col_name.to_string(), col_type.to_string()));
        }

        for (idx_name, idx_def) in indexes {
            table.add_index(IndexState::new(idx_name.to_string(), idx_def.to_string()));
        }

        table
    }

    #[test]
    fn test_no_changes() {
        let mut old_state = SchemaState::new();
        let table = create_test_table(
            "users",
            vec![("id", "BIGSERIAL PRIMARY KEY"), ("name", "TEXT NOT NULL")],
            vec![("idx_name", "CREATE INDEX idx_name ON users(name)")],
        );
        old_state.add_table(table.clone());

        let mut new_state = SchemaState::new();
        new_state.add_table(table);

        let diff = SchemaDiff::compute(&old_state, &new_state);

        assert!(!diff.has_changes());
        assert_eq!(diff.tables_added.len(), 0);
        assert_eq!(diff.tables_dropped.len(), 0);
        assert_eq!(diff.tables_modified.len(), 0);
    }

    #[test]
    fn test_table_added() {
        let old_state = SchemaState::new();

        let mut new_state = SchemaState::new();
        let table = create_test_table(
            "users",
            vec![("id", "BIGSERIAL PRIMARY KEY"), ("name", "TEXT NOT NULL")],
            vec![],
        );
        new_state.add_table(table);

        let diff = SchemaDiff::compute(&old_state, &new_state);

        assert!(diff.has_changes());
        assert_eq!(diff.tables_added.len(), 1);
        assert_eq!(diff.tables_added[0].name, "users");
        assert_eq!(diff.tables_dropped.len(), 0);
        assert_eq!(diff.tables_modified.len(), 0);
    }

    #[test]
    fn test_table_dropped() {
        let mut old_state = SchemaState::new();
        let table = create_test_table(
            "users",
            vec![("id", "BIGSERIAL PRIMARY KEY")],
            vec![],
        );
        old_state.add_table(table);

        let new_state = SchemaState::new();

        let diff = SchemaDiff::compute(&old_state, &new_state);

        assert!(diff.has_changes());
        assert_eq!(diff.tables_added.len(), 0);
        assert_eq!(diff.tables_dropped.len(), 1);
        assert_eq!(diff.tables_dropped[0], "users");
        assert_eq!(diff.tables_modified.len(), 0);
    }

    #[test]
    fn test_column_added() {
        let mut old_state = SchemaState::new();
        let old_table = create_test_table(
            "users",
            vec![("id", "BIGSERIAL PRIMARY KEY")],
            vec![],
        );
        old_state.add_table(old_table);

        let mut new_state = SchemaState::new();
        let new_table = create_test_table(
            "users",
            vec![
                ("id", "BIGSERIAL PRIMARY KEY"),
                ("email", "TEXT NOT NULL"),
            ],
            vec![],
        );
        new_state.add_table(new_table);

        let diff = SchemaDiff::compute(&old_state, &new_state);

        assert!(diff.has_changes());
        assert_eq!(diff.tables_modified.len(), 1);

        let table_diff = &diff.tables_modified[0];
        assert_eq!(table_diff.columns_added.len(), 1);
        assert_eq!(table_diff.columns_added[0].name, "email");
        assert_eq!(table_diff.columns_dropped.len(), 0);
        assert_eq!(table_diff.columns_modified.len(), 0);
    }

    #[test]
    fn test_column_dropped() {
        let mut old_state = SchemaState::new();
        let old_table = create_test_table(
            "users",
            vec![
                ("id", "BIGSERIAL PRIMARY KEY"),
                ("email", "TEXT NOT NULL"),
            ],
            vec![],
        );
        old_state.add_table(old_table);

        let mut new_state = SchemaState::new();
        let new_table = create_test_table(
            "users",
            vec![("id", "BIGSERIAL PRIMARY KEY")],
            vec![],
        );
        new_state.add_table(new_table);

        let diff = SchemaDiff::compute(&old_state, &new_state);

        assert!(diff.has_changes());
        assert_eq!(diff.tables_modified.len(), 1);

        let table_diff = &diff.tables_modified[0];
        assert_eq!(table_diff.columns_added.len(), 0);
        assert_eq!(table_diff.columns_dropped.len(), 1);
        assert_eq!(table_diff.columns_dropped[0], "email");
        assert_eq!(table_diff.columns_modified.len(), 0);
    }

    #[test]
    fn test_column_modified() {
        let mut old_state = SchemaState::new();
        let old_table = create_test_table(
            "users",
            vec![
                ("id", "BIGSERIAL PRIMARY KEY"),
                ("amount", "INTEGER NOT NULL"),
            ],
            vec![],
        );
        old_state.add_table(old_table);

        let mut new_state = SchemaState::new();
        let new_table = create_test_table(
            "users",
            vec![
                ("id", "BIGSERIAL PRIMARY KEY"),
                ("amount", "BIGINT NOT NULL"),
            ],
            vec![],
        );
        new_state.add_table(new_table);

        let diff = SchemaDiff::compute(&old_state, &new_state);

        assert!(diff.has_changes());
        assert_eq!(diff.tables_modified.len(), 1);

        let table_diff = &diff.tables_modified[0];
        assert_eq!(table_diff.columns_added.len(), 0);
        assert_eq!(table_diff.columns_dropped.len(), 0);
        assert_eq!(table_diff.columns_modified.len(), 1);
        assert_eq!(table_diff.columns_modified[0].column_name, "amount");
        assert_eq!(table_diff.columns_modified[0].old_type, "INTEGER NOT NULL");
        assert_eq!(table_diff.columns_modified[0].new_type, "BIGINT NOT NULL");
    }

    #[test]
    fn test_index_added() {
        let mut old_state = SchemaState::new();
        let old_table = create_test_table(
            "users",
            vec![("id", "BIGSERIAL PRIMARY KEY"), ("email", "TEXT NOT NULL")],
            vec![],
        );
        old_state.add_table(old_table);

        let mut new_state = SchemaState::new();
        let new_table = create_test_table(
            "users",
            vec![("id", "BIGSERIAL PRIMARY KEY"), ("email", "TEXT NOT NULL")],
            vec![("idx_email", "CREATE INDEX idx_email ON users(email)")],
        );
        new_state.add_table(new_table);

        let diff = SchemaDiff::compute(&old_state, &new_state);

        assert!(diff.has_changes());
        assert_eq!(diff.tables_modified.len(), 1);

        let table_diff = &diff.tables_modified[0];
        assert_eq!(table_diff.indexes_added.len(), 1);
        assert_eq!(table_diff.indexes_added[0].name, "idx_email");
        assert_eq!(table_diff.indexes_dropped.len(), 0);
    }

    #[test]
    fn test_index_dropped() {
        let mut old_state = SchemaState::new();
        let old_table = create_test_table(
            "users",
            vec![("id", "BIGSERIAL PRIMARY KEY"), ("email", "TEXT NOT NULL")],
            vec![("idx_email", "CREATE INDEX idx_email ON users(email)")],
        );
        old_state.add_table(old_table);

        let mut new_state = SchemaState::new();
        let new_table = create_test_table(
            "users",
            vec![("id", "BIGSERIAL PRIMARY KEY"), ("email", "TEXT NOT NULL")],
            vec![],
        );
        new_state.add_table(new_table);

        let diff = SchemaDiff::compute(&old_state, &new_state);

        assert!(diff.has_changes());
        assert_eq!(diff.tables_modified.len(), 1);

        let table_diff = &diff.tables_modified[0];
        assert_eq!(table_diff.indexes_added.len(), 0);
        assert_eq!(table_diff.indexes_dropped.len(), 1);
        assert_eq!(table_diff.indexes_dropped[0], "idx_email");
    }

    #[test]
    fn test_multiple_changes() {
        let mut old_state = SchemaState::new();
        let old_table1 = create_test_table(
            "users",
            vec![("id", "BIGSERIAL PRIMARY KEY"), ("name", "TEXT NOT NULL")],
            vec![],
        );
        let old_table2 = create_test_table(
            "posts",
            vec![("id", "BIGSERIAL PRIMARY KEY")],
            vec![],
        );
        old_state.add_table(old_table1);
        old_state.add_table(old_table2);

        let mut new_state = SchemaState::new();
        let new_table1 = create_test_table(
            "users",
            vec![
                ("id", "BIGSERIAL PRIMARY KEY"),
                ("name", "TEXT NOT NULL"),
                ("email", "TEXT NOT NULL"),
            ],
            vec![("idx_email", "CREATE INDEX idx_email ON users(email)")],
        );
        let new_table3 = create_test_table(
            "comments",
            vec![("id", "BIGSERIAL PRIMARY KEY")],
            vec![],
        );
        new_state.add_table(new_table1);
        new_state.add_table(new_table3);

        let diff = SchemaDiff::compute(&old_state, &new_state);

        assert!(diff.has_changes());

        // One table dropped (posts), one added (comments), one modified (users)
        assert_eq!(diff.tables_dropped.len(), 1);
        assert!(diff.tables_dropped.contains(&"posts".to_string()));

        assert_eq!(diff.tables_added.len(), 1);
        assert_eq!(diff.tables_added[0].name, "comments");

        assert_eq!(diff.tables_modified.len(), 1);
        assert_eq!(diff.tables_modified[0].table_name, "users");
        assert_eq!(diff.tables_modified[0].columns_added.len(), 1);
        assert_eq!(diff.tables_modified[0].indexes_added.len(), 1);
    }

    #[test]
    fn test_is_initial() {
        let old_state = SchemaState::new();

        let mut new_state = SchemaState::new();
        let table = create_test_table(
            "users",
            vec![("id", "BIGSERIAL PRIMARY KEY")],
            vec![],
        );
        new_state.add_table(table);

        let diff = SchemaDiff::compute(&old_state, &new_state);

        assert!(diff.is_initial());
    }
}
