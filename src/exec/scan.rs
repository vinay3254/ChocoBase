use crate::btree::cursor::Cursor;
use crate::btree::tree::BTree;
use crate::error::ExecError;
use crate::exec::Operator;
use crate::storage::pager::Pager;
use crate::types::row::decode_row;
use crate::types::schema::TableSchema;
use crate::types::value::Value;

pub struct SeqScan {
    cursor: Cursor,
    root: u32,
    schema: TableSchema,
    started: bool,
}

impl SeqScan {
    pub fn new(schema: TableSchema) -> Self {
        let root = schema.root_page;
        SeqScan { cursor: Cursor::empty(), root, schema, started: false }
    }
}

impl Operator for SeqScan {
    fn next(&mut self, pager: &mut Pager) -> Result<Option<Vec<Value>>, ExecError> {
        if !self.started {
            self.cursor = { BTree::new(pager, self.root).cursor_start()? };
            self.started = true;
        }
        match self.cursor.next(pager)? {
            Some((_key, payload)) => Ok(Some(decode_row(&self.schema, &payload))),
            None => Ok(None),
        }
    }
}

pub struct TableSeek {
    root: u32,
    schema: TableSchema,
    key: Vec<u8>,
    done: bool,
}

impl TableSeek {
    pub fn new(schema: TableSchema, key: Vec<u8>) -> Self {
        let root = schema.root_page;
        TableSeek { root, schema, key, done: false }
    }
}

impl Operator for TableSeek {
    fn next(&mut self, pager: &mut Pager) -> Result<Option<Vec<Value>>, ExecError> {
        if self.done {
            return Ok(None);
        }
        self.done = true;
        let mut bt = BTree::new(pager, self.root);
        match bt.search(&self.key)? {
            Some(payload) => Ok(Some(decode_row(&self.schema, &payload))),
            None => Ok(None),
        }
    }
}

pub struct IndexSeek {
    index_root: u32,
    schema: TableSchema,
    prefix: Vec<u8>,
    cursor: Cursor,
    started: bool,
}

impl IndexSeek {
    pub fn new(schema: TableSchema, index_root: u32, prefix: Vec<u8>) -> Self {
        IndexSeek { index_root, schema, prefix, cursor: Cursor::empty(), started: false }
    }
}

impl Operator for IndexSeek {
    fn next(&mut self, pager: &mut Pager) -> Result<Option<Vec<Value>>, ExecError> {
        if !self.started {
            self.cursor = { BTree::new(pager, self.index_root).cursor_seek(&self.prefix)? };
            self.started = true;
        }
        loop {
            match self.cursor.next(pager)? {
                Some((key, _)) => {
                    if !key.starts_with(self.prefix.as_slice()) {
                        return Ok(None);
                    }
                    let pk_bytes = key[self.prefix.len()..].to_vec();
                    let mut table_bt = BTree::new(pager, self.schema.root_page);
                    if let Some(payload) = table_bt.search(&pk_bytes)? {
                        return Ok(Some(decode_row(&self.schema, &payload)));
                    }
                }
                None => return Ok(None),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::btree::node::LeafNode;
    use crate::types::schema::Column;
    use crate::types::value::ColumnType;
    use tempfile::NamedTempFile;

    fn schema_with_root(root: u32) -> TableSchema {
        TableSchema {
            name: "t".into(),
            columns: vec![Column { name: "id".into(), ty: ColumnType::Integer, not_null: true, is_primary_key: true }],
            root_page: root,
        }
    }

    #[test]
    fn scans_all_rows_in_key_order() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let initial_root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(initial_root).unwrap());
        let final_root = {
            let mut bt = BTree::new(&mut pager, initial_root);
            let schema = schema_with_root(initial_root);
            for i in [3, 1, 2] {
                let row = vec![Value::Integer(i)];
                bt.insert(
                    &crate::types::value::encode_key(&Value::Integer(i)),
                    &crate::types::row::encode_row(&schema, &row),
                )
                .unwrap();
            }
            bt.root()
        };

        let mut scan = SeqScan::new(schema_with_root(final_root));
        let mut seen = Vec::new();
        while let Some(row) = scan.next(&mut pager).unwrap() {
            seen.push(row[0].clone());
        }
        assert_eq!(seen, vec![Value::Integer(1), Value::Integer(2), Value::Integer(3)]);
    }

    #[test]
    fn table_seek_finds_single_row_by_primary_key() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let initial_root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(initial_root).unwrap());
        let schema = schema_with_root(initial_root);
        let final_root = {
            let mut bt = BTree::new(&mut pager, initial_root);
            for i in [1, 2, 3] {
                let row = vec![Value::Integer(i)];
                bt.insert(&crate::types::value::encode_key(&Value::Integer(i)), &crate::types::row::encode_row(&schema, &row)).unwrap();
            }
            bt.root()
        };

        let key = crate::types::value::encode_key(&Value::Integer(2));
        let mut seek = TableSeek::new(schema_with_root(final_root), key);
        assert_eq!(seek.next(&mut pager).unwrap(), Some(vec![Value::Integer(2)]));
        assert_eq!(seek.next(&mut pager).unwrap(), None, "seek yields at most one row");

        let missing_key = crate::types::value::encode_key(&Value::Integer(99));
        let mut seek_missing = TableSeek::new(schema_with_root(final_root), missing_key);
        assert_eq!(seek_missing.next(&mut pager).unwrap(), None);
    }

    #[test]
    fn index_seek_finds_row_by_indexed_value() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let table_root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(table_root).unwrap());
        let index_root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(index_root).unwrap());

        let schema = TableSchema {
            name: "t".into(),
            columns: vec![
                Column { name: "id".into(), ty: ColumnType::Integer, not_null: true, is_primary_key: true },
                Column { name: "name".into(), ty: ColumnType::Text, not_null: true, is_primary_key: false },
            ],
            root_page: table_root,
        };

        let rows = [(1, "a"), (2, "b"), (3, "a")];

        let final_table_root = {
            let mut tbt = BTree::new(&mut pager, table_root);
            for (id, name) in rows {
                let row = vec![Value::Integer(id), Value::Text(name.into())];
                tbt.insert(&crate::types::value::encode_key(&Value::Integer(id)), &crate::types::row::encode_row(&schema, &row)).unwrap();
            }
            tbt.root()
        };

        let final_index_root = {
            let mut ibt = BTree::new(&mut pager, index_root);
            for (id, name) in rows {
                let idx_key = crate::types::value::encode_composite_key(&[Value::Text(name.into()), Value::Integer(id)]);
                ibt.insert(&idx_key, &[]).unwrap();
            }
            ibt.root()
        };

        let mut final_schema = schema.clone();
        final_schema.root_page = final_table_root;
        let prefix = crate::types::value::encode_key(&Value::Text("a".into()));
        let mut seek = IndexSeek::new(final_schema, final_index_root, prefix);

        let mut seen = Vec::new();
        while let Some(row) = seek.next(&mut pager).unwrap() {
            seen.push(row[0].clone());
        }
        seen.sort_by_key(|v| match v { Value::Integer(n) => *n, _ => unreachable!() });
        assert_eq!(seen, vec![Value::Integer(1), Value::Integer(3)]);
    }
}
