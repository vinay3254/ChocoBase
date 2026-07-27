use crate::btree::tree::BTree;
use crate::error::{BTreeError, ExecError};
use crate::storage::pager::Pager;
use crate::types::row::encode_row;
use crate::types::schema::{IndexSchema, TableSchema};
use crate::types::value::{encode_composite_key, encode_key, Value};

pub fn insert_row(
    pager: &mut Pager,
    schema: &TableSchema,
    indexes: &[IndexSchema],
    row: &[Value],
) -> Result<(u32, Vec<(String, u32)>), ExecError> {
    for (col, v) in schema.columns.iter().zip(row.iter()) {
        if col.not_null && matches!(v, Value::Null) {
            return Err(ExecError::NotNullViolation(col.name.clone()));
        }
        if let Value::Text(s) = v {
            if s.contains('\0') {
                return Err(ExecError::InvalidValue(format!(
                    "{} contains an interior NUL byte, which cannot be represented in an order-preserving key",
                    col.name
                )));
            }
        }
    }
    let pk_idx = schema.primary_key_index();
    let key = encode_key(&row[pk_idx]);
    let payload = encode_row(schema, row);
    let mut bt = BTree::new(pager, schema.root_page);
    bt.insert(&key, &payload).map_err(|e| match e {
        BTreeError::DuplicateKey => ExecError::DuplicatePrimaryKey,
        other => ExecError::BTree(other),
    })?;
    let new_table_root = bt.root();

    let mut new_index_roots = Vec::new();
    for idx in indexes {
        let col_idx = schema.column_index(&idx.column).expect("index column must exist in table schema");
        let idx_key = encode_composite_key(&[row[col_idx].clone(), row[pk_idx].clone()]);
        let mut ibt = BTree::new(pager, idx.root_page);
        ibt.insert(&idx_key, &[])?;
        new_index_roots.push((idx.name.clone(), ibt.root()));
    }
    Ok((new_table_root, new_index_roots))
}

pub fn delete_row(
    pager: &mut Pager,
    schema: &TableSchema,
    indexes: &[IndexSchema],
    row: &[Value],
) -> Result<(u32, Vec<(String, u32)>), ExecError> {
    let pk_idx = schema.primary_key_index();
    let key = encode_key(&row[pk_idx]);
    let mut bt = BTree::new(pager, schema.root_page);
    bt.delete(&key)?;
    let new_table_root = bt.root();

    let mut new_index_roots = Vec::new();
    for idx in indexes {
        let col_idx = schema.column_index(&idx.column).expect("index column must exist in table schema");
        let idx_key = encode_composite_key(&[row[col_idx].clone(), row[pk_idx].clone()]);
        let mut ibt = BTree::new(pager, idx.root_page);
        ibt.delete(&idx_key)?;
        new_index_roots.push((idx.name.clone(), ibt.root()));
    }
    Ok((new_table_root, new_index_roots))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::btree::node::LeafNode;
    use crate::types::schema::Column;
    use crate::types::value::ColumnType;
    use tempfile::NamedTempFile;

    fn schema(root: u32) -> TableSchema {
        TableSchema {
            name: "t".into(),
            columns: vec![
                Column { name: "id".into(), ty: ColumnType::Integer, not_null: true, is_primary_key: true },
                Column { name: "name".into(), ty: ColumnType::Text, not_null: true, is_primary_key: false },
            ],
            root_page: root,
        }
    }

    #[test]
    fn inserts_row_and_reports_unchanged_root_for_small_table() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(root).unwrap());
        let s = schema(root);

        let (new_root, new_index_roots) = insert_row(&mut pager, &s, &[], &[Value::Integer(1), Value::Text("a".into())]).unwrap();
        assert_eq!(new_root, root);
        assert!(new_index_roots.is_empty());

        let mut bt = BTree::new(&mut pager, new_root);
        let payload = bt.search(&crate::types::value::encode_key(&Value::Integer(1))).unwrap().unwrap();
        assert_eq!(crate::types::row::decode_row(&s, &payload), vec![Value::Integer(1), Value::Text("a".into())]);
    }

    #[test]
    fn not_null_violation_is_rejected() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(root).unwrap());
        let s = schema(root);
        let err = insert_row(&mut pager, &s, &[], &[Value::Integer(1), Value::Null]).unwrap_err();
        assert!(matches!(err, ExecError::NotNullViolation(_)));
    }

    #[test]
    fn duplicate_primary_key_is_rejected() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(root).unwrap());
        let s = schema(root);
        insert_row(&mut pager, &s, &[], &[Value::Integer(1), Value::Text("a".into())]).unwrap();
        let err = insert_row(&mut pager, &s, &[], &[Value::Integer(1), Value::Text("b".into())]).unwrap_err();
        assert!(matches!(err, ExecError::DuplicatePrimaryKey));
    }

    #[test]
    fn interior_nul_in_text_is_rejected() {
        // TEXT keys are encoded as UTF-8 bytes + a 0x00 terminator (types/value.rs).
        // A value containing an embedded NUL byte would let the terminator-based
        // encoding stop early, breaking order-preservation for composite keys built
        // from it — see the design spec's storage section: "Text values may not
        // contain an interior NUL; this is validated at insert and rejected as
        // InvalidValue." This is that validation.
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(root).unwrap());
        let s = schema(root);
        let err = insert_row(&mut pager, &s, &[], &[Value::Integer(1), Value::Text("a\0b".into())]).unwrap_err();
        assert!(matches!(err, ExecError::InvalidValue(_)));
    }

    #[test]
    fn maintains_secondary_index_entries() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let table_root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(table_root).unwrap());
        let index_root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(index_root).unwrap());
        let s = schema(table_root);
        let idx = IndexSchema { name: "idx_name".into(), table: "t".into(), column: "name".into(), root_page: index_root };

        let (_, new_index_roots) = insert_row(&mut pager, &s, &[idx.clone()], &[Value::Integer(1), Value::Text("a".into())]).unwrap();
        let final_index_root = new_index_roots[0].1;

        let mut ibt = BTree::new(&mut pager, final_index_root);
        let idx_key = crate::types::value::encode_composite_key(&[Value::Text("a".into()), Value::Integer(1)]);
        assert_eq!(ibt.search(&idx_key).unwrap(), Some(vec![]));
    }

    #[test]
    fn deletes_row_and_its_index_entries() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let table_root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(table_root).unwrap());
        let index_root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(index_root).unwrap());
        let s = schema(table_root);
        let idx = IndexSchema { name: "idx_name".into(), table: "t".into(), column: "name".into(), root_page: index_root };

        let (table_root, roots) = insert_row(&mut pager, &s, &[idx.clone()], &[Value::Integer(1), Value::Text("a".into())]).unwrap();
        let mut s2 = s.clone();
        s2.root_page = table_root;
        let mut idx2 = idx.clone();
        idx2.root_page = roots[0].1;

        let (new_table_root, new_index_roots) =
            delete_row(&mut pager, &s2, &[idx2.clone()], &[Value::Integer(1), Value::Text("a".into())]).unwrap();

        let mut bt = BTree::new(&mut pager, new_table_root);
        assert_eq!(bt.search(&crate::types::value::encode_key(&Value::Integer(1))).unwrap(), None);

        let mut ibt = BTree::new(&mut pager, new_index_roots[0].1);
        let idx_key = crate::types::value::encode_composite_key(&[Value::Text("a".into()), Value::Integer(1)]);
        assert_eq!(ibt.search(&idx_key).unwrap(), None);
    }
}
