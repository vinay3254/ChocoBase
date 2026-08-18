pub mod record;

use crate::btree::node::{InternalNode, LeafNode};
use crate::btree::tree::BTree;
use crate::error::{BTreeError, DbError, PlanError, StorageError};
use crate::storage::page::{PAGE_TYPE_INTERNAL, PAGE_TYPE_LEAF};
use crate::storage::pager::Pager;
use crate::types::schema::{IndexSchema, PolicySchema, TableSchema};
use crate::types::value::{encode_key, Value};
use record::*;

pub struct Catalog {
    root: u32,
}

impl Catalog {
    pub fn bootstrap(pager: &mut Pager) -> Result<Catalog, DbError> {
        if pager.catalog_root() == 0 {
            let root_page = pager.allocate_page()?;
            LeafNode {
                entries: vec![],
                next_leaf: 0,
            }
            .encode(pager.get_page_mut(root_page)?);
            pager.set_catalog_root(root_page)?;
        }
        Ok(Catalog {
            root: pager.catalog_root(),
        })
    }

    fn table_key(name: &str) -> Vec<u8> {
        encode_key(&Value::Text(format!("table:{name}")))
    }

    fn index_key(name: &str) -> Vec<u8> {
        encode_key(&Value::Text(format!("index:{name}")))
    }

    pub fn create_table(&mut self, pager: &mut Pager, schema: &TableSchema) -> Result<(), DbError> {
        let key = Self::table_key(&schema.name);
        let mut bt = BTree::new(pager, self.root);
        bt.insert(&key, &encode_table_record(schema))
            .map_err(|e| match e {
                BTreeError::DuplicateKey => {
                    DbError::Plan(PlanError::TableAlreadyExists(schema.name.clone()))
                }
                other => DbError::BTree(other),
            })?;
        self.root = bt.root();
        pager.set_catalog_root(self.root)?;
        Ok(())
    }

    pub fn get_table(
        &mut self,
        pager: &mut Pager,
        name: &str,
    ) -> Result<Option<TableSchema>, DbError> {
        let key = Self::table_key(name);
        let mut bt = BTree::new(pager, self.root);
        Ok(bt.search(&key)?.map(|p| decode_table_record(&p)))
    }

    pub fn update_table_root(
        &mut self,
        pager: &mut Pager,
        name: &str,
        new_root: u32,
    ) -> Result<(), DbError> {
        let mut schema = self
            .get_table(pager, name)?
            .ok_or_else(|| PlanError::NoSuchTable(name.to_string()))?;
        schema.root_page = new_root;
        let key = Self::table_key(name);
        let mut bt = BTree::new(pager, self.root);
        bt.delete(&key)?;
        bt.insert(&key, &encode_table_record(&schema))?;
        self.root = bt.root();
        pager.set_catalog_root(self.root)?;
        Ok(())
    }

    pub fn update_table_schema(
        &mut self,
        pager: &mut Pager,
        schema: &TableSchema,
    ) -> Result<(), DbError> {
        let key = Self::table_key(&schema.name);
        let mut bt = BTree::new(pager, self.root);
        let _ = bt.delete(&key);
        bt.insert(&key, &encode_table_record(schema))?;
        self.root = bt.root();
        pager.set_catalog_root(self.root)?;
        Ok(())
    }

    pub fn rename_table(
        &mut self,
        pager: &mut Pager,
        old_name: &str,
        new_name: &str,
    ) -> Result<(), DbError> {
        if self.get_table(pager, new_name)?.is_some() {
            return Err(DbError::Plan(PlanError::TableAlreadyExists(
                new_name.to_string(),
            )));
        }
        let mut schema = self
            .get_table(pager, old_name)?
            .ok_or_else(|| PlanError::NoSuchTable(old_name.to_string()))?;
        schema.name = new_name.to_string();
        let old_key = Self::table_key(old_name);
        let new_key = Self::table_key(new_name);
        let mut bt = BTree::new(pager, self.root);
        bt.delete(&old_key)?;
        bt.insert(&new_key, &encode_table_record(&schema))?;
        self.root = bt.root();
        pager.set_catalog_root(self.root)?;
        Ok(())
    }

    pub fn drop_table(&mut self, pager: &mut Pager, name: &str) -> Result<(), DbError> {
        let schema = self
            .get_table(pager, name)?
            .ok_or_else(|| PlanError::NoSuchTable(name.to_string()))?;
        for idx in self.list_indexes_for_table(pager, name)? {
            self.drop_index(pager, &idx.name)?;
        }
        walk_and_free(pager, schema.root_page)?;
        let key = Self::table_key(name);
        let mut bt = BTree::new(pager, self.root);
        bt.delete(&key)?;
        self.root = bt.root();
        pager.set_catalog_root(self.root)?;
        Ok(())
    }

    pub fn create_index(&mut self, pager: &mut Pager, schema: &IndexSchema) -> Result<(), DbError> {
        let key = Self::index_key(&schema.name);
        let mut bt = BTree::new(pager, self.root);
        bt.insert(&key, &encode_index_record(schema))
            .map_err(|e| match e {
                BTreeError::DuplicateKey => {
                    DbError::Plan(PlanError::IndexAlreadyExists(schema.name.clone()))
                }
                other => DbError::BTree(other),
            })?;
        self.root = bt.root();
        pager.set_catalog_root(self.root)?;
        Ok(())
    }

    pub fn get_index(
        &mut self,
        pager: &mut Pager,
        name: &str,
    ) -> Result<Option<IndexSchema>, DbError> {
        let key = Self::index_key(name);
        let mut bt = BTree::new(pager, self.root);
        Ok(bt.search(&key)?.map(|p| decode_index_record(&p)))
    }

    pub fn update_index_root(
        &mut self,
        pager: &mut Pager,
        name: &str,
        new_root: u32,
    ) -> Result<(), DbError> {
        let mut schema = self
            .get_index(pager, name)?
            .ok_or_else(|| PlanError::NoSuchIndex(name.to_string()))?;
        schema.root_page = new_root;
        let key = Self::index_key(name);
        let mut bt = BTree::new(pager, self.root);
        bt.delete(&key)?;
        bt.insert(&key, &encode_index_record(&schema))?;
        self.root = bt.root();
        pager.set_catalog_root(self.root)?;
        Ok(())
    }

    pub fn drop_index(&mut self, pager: &mut Pager, name: &str) -> Result<(), DbError> {
        let schema = self
            .get_index(pager, name)?
            .ok_or_else(|| PlanError::NoSuchIndex(name.to_string()))?;
        walk_and_free(pager, schema.root_page)?;
        let key = Self::index_key(name);
        let mut bt = BTree::new(pager, self.root);
        bt.delete(&key)?;
        self.root = bt.root();
        pager.set_catalog_root(self.root)?;
        Ok(())
    }

    pub fn list_tables(&mut self, pager: &mut Pager) -> Result<Vec<String>, DbError> {
        let mut cursor = {
            let mut bt = BTree::new(pager, self.root);
            bt.cursor_start()?
        };
        let mut names = Vec::new();
        while let Some((_, payload)) = cursor.next(pager)? {
            if record_kind(&payload) == 1 {
                names.push(decode_table_record(&payload).name);
            }
        }
        Ok(names)
    }

    pub fn list_indexes_for_table(
        &mut self,
        pager: &mut Pager,
        table: &str,
    ) -> Result<Vec<IndexSchema>, DbError> {
        let mut cursor = {
            let mut bt = BTree::new(pager, self.root);
            bt.cursor_start()?
        };
        let mut result = Vec::new();
        while let Some((_, payload)) = cursor.next(pager)? {
            if record_kind(&payload) == 2 {
                let idx = decode_index_record(&payload);
                if idx.table == table {
                    result.push(idx);
                }
            }
        }
        Ok(result)
    }

    fn policy_key(name: &str) -> Vec<u8> {
        encode_key(&Value::Text(format!("policy:{name}")))
    }

    pub fn set_table_rls(
        &mut self,
        pager: &mut Pager,
        table: &str,
        enabled: bool,
    ) -> Result<(), DbError> {
        let mut schema = self
            .get_table(pager, table)?
            .ok_or_else(|| PlanError::NoSuchTable(table.to_string()))?;
        schema.rls_enabled = enabled;
        let key = Self::table_key(table);
        let mut bt = BTree::new(pager, self.root);
        bt.delete(&key)?;
        bt.insert(&key, &encode_table_record(&schema))?;
        self.root = bt.root();
        pager.set_catalog_root(self.root)?;
        Ok(())
    }

    pub fn create_policy(
        &mut self,
        pager: &mut Pager,
        policy: &PolicySchema,
    ) -> Result<(), DbError> {
        let key = Self::policy_key(&policy.name);
        let mut bt = BTree::new(pager, self.root);
        bt.insert(&key, &encode_policy_record(policy))
            .map_err(|e| match e {
                BTreeError::DuplicateKey => {
                    DbError::Plan(PlanError::TableAlreadyExists(policy.name.clone()))
                }
                other => DbError::BTree(other),
            })?;
        self.root = bt.root();
        pager.set_catalog_root(self.root)?;
        Ok(())
    }

    pub fn drop_policy(&mut self, pager: &mut Pager, name: &str) -> Result<(), DbError> {
        let key = Self::policy_key(name);
        let mut bt = BTree::new(pager, self.root);
        bt.delete(&key)?;
        self.root = bt.root();
        pager.set_catalog_root(self.root)?;
        Ok(())
    }

    pub fn list_policies_for_table(
        &mut self,
        pager: &mut Pager,
        table: &str,
    ) -> Result<Vec<PolicySchema>, DbError> {
        let mut cursor = {
            let mut bt = BTree::new(pager, self.root);
            bt.cursor_start()?
        };
        let mut result = Vec::new();
        while let Some((_, payload)) = cursor.next(pager)? {
            if record_kind(&payload) == KIND_POLICY {
                let pol = decode_policy_record(&payload);
                if pol.table == table {
                    result.push(pol);
                }
            }
        }
        Ok(result)
    }
}

fn walk_and_free(pager: &mut Pager, page_no: u32) -> Result<(), StorageError> {
    let page_type = pager.get_page(page_no)?.page_type();
    if page_type == PAGE_TYPE_INTERNAL {
        let node = InternalNode::decode(pager.get_page(page_no)?);
        let children: Vec<u32> = node
            .entries
            .iter()
            .map(|e| e.left_child)
            .chain(std::iter::once(node.rightmost_child))
            .collect();
        for c in children {
            walk_and_free(pager, c)?;
        }
    } else if page_type != PAGE_TYPE_LEAF {
        return Ok(());
    }
    pager.free_page(page_no)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::schema::Column;
    use crate::types::value::ColumnType;
    use tempfile::NamedTempFile;

    fn sample_schema(name: &str) -> TableSchema {
        TableSchema {
            name: name.into(),
            columns: vec![Column {
                name: "id".into(),
                ty: ColumnType::Integer,
                not_null: true,
                is_primary_key: true,
            }],
            root_page: 0, // filled in by the caller after allocating a table root
            rls_enabled: false,
        }
    }

    #[test]
    fn create_and_get_table_roundtrips() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let mut catalog = Catalog::bootstrap(&mut pager).unwrap();

        let table_root = pager.allocate_page().unwrap();
        LeafNode {
            entries: vec![],
            next_leaf: 0,
        }
        .encode(pager.get_page_mut(table_root).unwrap());
        let mut schema = sample_schema("users");
        schema.root_page = table_root;
        catalog.create_table(&mut pager, &schema).unwrap();

        let fetched = catalog.get_table(&mut pager, "users").unwrap().unwrap();
        assert_eq!(fetched.name, "users");
        assert_eq!(fetched.root_page, table_root);
        assert!(catalog.get_table(&mut pager, "missing").unwrap().is_none());
    }

    #[test]
    fn create_duplicate_table_errors() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let mut catalog = Catalog::bootstrap(&mut pager).unwrap();
        let mut schema = sample_schema("users");
        schema.root_page = pager.allocate_page().unwrap();
        LeafNode {
            entries: vec![],
            next_leaf: 0,
        }
        .encode(pager.get_page_mut(schema.root_page).unwrap());
        catalog.create_table(&mut pager, &schema).unwrap();
        let err = catalog.create_table(&mut pager, &schema).unwrap_err();
        assert!(matches!(
            err,
            DbError::Plan(PlanError::TableAlreadyExists(_))
        ));
    }

    #[test]
    fn drop_table_removes_it_and_frees_pages() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let mut catalog = Catalog::bootstrap(&mut pager).unwrap();
        let mut schema = sample_schema("users");
        schema.root_page = pager.allocate_page().unwrap();
        LeafNode {
            entries: vec![],
            next_leaf: 0,
        }
        .encode(pager.get_page_mut(schema.root_page).unwrap());
        catalog.create_table(&mut pager, &schema).unwrap();

        catalog.drop_table(&mut pager, "users").unwrap();
        assert!(catalog.get_table(&mut pager, "users").unwrap().is_none());
    }

    #[test]
    fn list_tables_returns_created_names() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let mut catalog = Catalog::bootstrap(&mut pager).unwrap();
        for name in ["a", "b", "c"] {
            let mut schema = sample_schema(name);
            schema.root_page = pager.allocate_page().unwrap();
            LeafNode {
                entries: vec![],
                next_leaf: 0,
            }
            .encode(pager.get_page_mut(schema.root_page).unwrap());
            catalog.create_table(&mut pager, &schema).unwrap();
        }
        let mut names = catalog.list_tables(&mut pager).unwrap();
        names.sort();
        assert_eq!(names, vec!["a", "b", "c"]);
    }

    #[test]
    fn create_index_get_and_drop() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let mut catalog = Catalog::bootstrap(&mut pager).unwrap();
        let idx_root = pager.allocate_page().unwrap();
        LeafNode {
            entries: vec![],
            next_leaf: 0,
        }
        .encode(pager.get_page_mut(idx_root).unwrap());
        let idx = IndexSchema {
            name: "idx_id".into(),
            table: "users".into(),
            column: "id".into(),
            root_page: idx_root,
        };
        catalog.create_index(&mut pager, &idx).unwrap();

        let fetched = catalog.get_index(&mut pager, "idx_id").unwrap().unwrap();
        assert_eq!(fetched.table, "users");

        let for_table = catalog.list_indexes_for_table(&mut pager, "users").unwrap();
        assert_eq!(for_table.len(), 1);

        catalog.drop_index(&mut pager, "idx_id").unwrap();
        assert!(catalog.get_index(&mut pager, "idx_id").unwrap().is_none());
    }

    #[test]
    fn update_table_root_persists_new_root() {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let mut catalog = Catalog::bootstrap(&mut pager).unwrap();
        let mut schema = sample_schema("users");
        schema.root_page = pager.allocate_page().unwrap();
        LeafNode {
            entries: vec![],
            next_leaf: 0,
        }
        .encode(pager.get_page_mut(schema.root_page).unwrap());
        catalog.create_table(&mut pager, &schema).unwrap();

        let new_root = pager.allocate_page().unwrap();
        catalog
            .update_table_root(&mut pager, "users", new_root)
            .unwrap();
        let fetched = catalog.get_table(&mut pager, "users").unwrap().unwrap();
        assert_eq!(fetched.root_page, new_root);
    }
}
