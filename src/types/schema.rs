use crate::types::value::ColumnType;

#[derive(Debug, Clone)]
pub struct Column {
    pub name: String,
    pub ty: ColumnType,
    pub not_null: bool,
    pub is_primary_key: bool,
}

#[derive(Debug, Clone)]
pub struct TableSchema {
    pub name: String,
    pub columns: Vec<Column>,
    pub root_page: u32,
}

#[derive(Debug, Clone)]
pub struct IndexSchema {
    pub name: String,
    pub table: String,
    pub column: String,
    pub root_page: u32,
}

impl TableSchema {
    pub fn primary_key_index(&self) -> usize {
        self.columns
            .iter()
            .position(|c| c.is_primary_key)
            .expect("table schema must have exactly one primary key column")
    }

    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> TableSchema {
        TableSchema {
            name: "users".into(),
            columns: vec![
                Column { name: "id".into(), ty: ColumnType::Integer, not_null: true, is_primary_key: true },
                Column { name: "name".into(), ty: ColumnType::Text, not_null: false, is_primary_key: false },
            ],
            root_page: 2,
        }
    }

    #[test]
    fn finds_primary_key_index() {
        assert_eq!(sample().primary_key_index(), 0);
    }

    #[test]
    fn finds_column_by_name() {
        assert_eq!(sample().column_index("name"), Some(1));
        assert_eq!(sample().column_index("missing"), None);
    }
}
