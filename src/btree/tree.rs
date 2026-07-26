use crate::error::{BTreeError, StorageError};
use crate::storage::page::{PAGE_TYPE_INTERNAL, PAGE_TYPE_LEAF};
use crate::storage::pager::Pager;
use crate::btree::node::{InternalNode, LeafNode, InternalEntry, LeafEntry};

pub struct BTree<'a> {
    pager: &'a mut Pager,
    root: u32,
}

impl<'a> BTree<'a> {
    pub fn new(pager: &'a mut Pager, root: u32) -> Self {
        BTree { pager, root }
    }

    pub fn root(&self) -> u32 {
        self.root
    }

    fn descend(&mut self, key: &[u8]) -> Result<(Vec<u32>, u32), BTreeError> {
        let mut path = Vec::new();
        let mut current = self.root;
        loop {
            let page = self.pager.get_page(current)?;
            match page.page_type() {
                PAGE_TYPE_LEAF => return Ok((path, current)),
                PAGE_TYPE_INTERNAL => {
                    let node = InternalNode::decode(page);
                    path.push(current);
                    current = node.child_for_key(key);
                }
                t => {
                    return Err(BTreeError::Storage(StorageError::CorruptPage(
                        current,
                        format!("unexpected page type {t}"),
                    )))
                }
            }
        }
    }

    pub fn search(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>, BTreeError> {
        let (_, leaf_no) = self.descend(key)?;
        let page = self.pager.get_page(leaf_no)?;
        let node = LeafNode::decode(page);
        Ok(node.entries.iter().find(|e| e.key == key).map(|e| e.payload.clone()))
    }

    pub fn insert(&mut self, key: &[u8], payload: &[u8]) -> Result<(), BTreeError> {
        use crate::btree::node::NODE_HEADER_SIZE;
        use crate::storage::page::PAGE_SIZE;

        let single_entry_size = NODE_HEADER_SIZE + 2 + 2 + key.len() + 2 + payload.len();
        if single_entry_size > PAGE_SIZE {
            return Err(BTreeError::RowTooLarge(single_entry_size, PAGE_SIZE));
        }

        let (path, leaf_no) = self.descend(key)?;
        let page = self.pager.get_page(leaf_no)?;
        let mut node = LeafNode::decode(page);

        if node.entries.iter().any(|e| e.key == key) {
            return Err(BTreeError::DuplicateKey);
        }

        let pos = node.entries.partition_point(|e| e.key.as_slice() < key);
        node.entries.insert(pos, LeafEntry { key: key.to_vec(), payload: payload.to_vec() });

        if node.encoded_size() <= PAGE_SIZE {
            let page = self.pager.get_page_mut(leaf_no)?;
            node.encode(page);
            return Ok(());
        }

        let old_next = node.next_leaf;
        let mid = node.entries.len() / 2;
        let right_entries = node.entries.split_off(mid);
        let left_entries = node.entries;

        let right_no = self.pager.allocate_page()?;
        let separator = right_entries[0].key.clone();

        let left_node = LeafNode { entries: left_entries, next_leaf: right_no };
        let right_node = LeafNode { entries: right_entries, next_leaf: old_next };

        left_node.encode(self.pager.get_page_mut(leaf_no)?);
        right_node.encode(self.pager.get_page_mut(right_no)?);

        self.insert_into_parent(path, leaf_no, separator, right_no)
    }

    fn insert_into_parent(
        &mut self,
        mut path: Vec<u32>,
        left_child: u32,
        separator: Vec<u8>,
        right_child: u32,
    ) -> Result<(), BTreeError> {
        use crate::storage::page::PAGE_SIZE;

        match path.pop() {
            None => {
                let new_root_no = self.pager.allocate_page()?;
                let new_root = InternalNode {
                    entries: vec![InternalEntry { key: separator, left_child }],
                    rightmost_child: right_child,
                };
                new_root.encode(self.pager.get_page_mut(new_root_no)?);
                self.root = new_root_no;
                Ok(())
            }
            Some(parent_no) => {
                let page = self.pager.get_page(parent_no)?;
                let mut node = InternalNode::decode(page);

                if let Some(i) = node.entries.iter().position(|e| e.left_child == left_child) {
                    node.entries[i].left_child = right_child;
                    node.entries.insert(i, InternalEntry { key: separator, left_child });
                } else {
                    debug_assert_eq!(node.rightmost_child, left_child);
                    node.rightmost_child = right_child;
                    node.entries.push(InternalEntry { key: separator, left_child });
                }

                if node.encoded_size() <= PAGE_SIZE {
                    node.encode(self.pager.get_page_mut(parent_no)?);
                    return Ok(());
                }

                // Internal split: handled fully in Task 11.
                self.split_internal(path, parent_no, node)
            }
        }
    }

    fn split_internal(
        &mut self,
        _path: Vec<u32>,
        _parent_no: u32,
        _node: InternalNode,
    ) -> Result<(), BTreeError> {
        unimplemented!("implemented in Task 11")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    use crate::btree::node::{InternalEntry, LeafEntry};

    fn two_level_tree() -> (Pager, u32) {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let left = pager.allocate_page().unwrap();
        let right = pager.allocate_page().unwrap();
        let root = pager.allocate_page().unwrap();

        let left_node = LeafNode {
            entries: vec![LeafEntry { key: vec![1], payload: vec![b'a'] }],
            next_leaf: right,
        };
        left_node.encode(pager.get_page_mut(left).unwrap());

        let right_node = LeafNode {
            entries: vec![LeafEntry { key: vec![5], payload: vec![b'b'] }],
            next_leaf: 0,
        };
        right_node.encode(pager.get_page_mut(right).unwrap());

        let root_node = InternalNode {
            entries: vec![InternalEntry { key: vec![5], left_child: left }],
            rightmost_child: right,
        };
        root_node.encode(pager.get_page_mut(root).unwrap());

        pager.flush().unwrap();
        (pager, root)
    }

    #[test]
    fn search_finds_existing_key_across_levels() {
        let (mut pager, root) = two_level_tree();
        let mut bt = BTree::new(&mut pager, root);
        assert_eq!(bt.search(&[1]).unwrap(), Some(vec![b'a']));
        assert_eq!(bt.search(&[5]).unwrap(), Some(vec![b'b']));
    }

    #[test]
    fn search_returns_none_for_missing_key() {
        let (mut pager, root) = two_level_tree();
        let mut bt = BTree::new(&mut pager, root);
        assert_eq!(bt.search(&[3]).unwrap(), None);
    }

    fn empty_tree() -> (Pager, u32) {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(root).unwrap());
        pager.flush().unwrap();
        (pager, root)
    }

    #[test]
    fn insert_then_search_finds_key() {
        let (mut pager, root) = empty_tree();
        let mut bt = BTree::new(&mut pager, root);
        bt.insert(&[1], b"one").unwrap();
        bt.insert(&[2], b"two").unwrap();
        assert_eq!(bt.search(&[1]).unwrap(), Some(b"one".to_vec()));
        assert_eq!(bt.search(&[2]).unwrap(), Some(b"two".to_vec()));
    }

    #[test]
    fn insert_duplicate_key_errors() {
        let (mut pager, root) = empty_tree();
        let mut bt = BTree::new(&mut pager, root);
        bt.insert(&[1], b"one").unwrap();
        let err = bt.insert(&[1], b"again").unwrap_err();
        assert!(matches!(err, BTreeError::DuplicateKey));
    }

    #[test]
    fn insert_row_too_large_errors() {
        let (mut pager, root) = empty_tree();
        let mut bt = BTree::new(&mut pager, root);
        let huge_payload = vec![0u8; 5000];
        let err = bt.insert(&[1], &huge_payload).unwrap_err();
        assert!(matches!(err, BTreeError::RowTooLarge(_, _)));
    }

    #[test]
    fn inserting_enough_keys_splits_root_and_changes_it() {
        let (mut pager, root) = empty_tree();
        let mut bt = BTree::new(&mut pager, root);
        for i in 0..400i64 {
            bt.insert(&(i as u64).to_be_bytes(), format!("row-{i}").as_bytes()).unwrap();
        }
        assert_ne!(bt.root(), root, "enough inserts must force at least one split");
        for i in 0..400i64 {
            assert_eq!(
                bt.search(&(i as u64).to_be_bytes()).unwrap(),
                Some(format!("row-{i}").into_bytes())
            );
        }
    }
}
