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
        path: Vec<u32>,
        parent_no: u32,
        mut node: InternalNode,
    ) -> Result<(), BTreeError> {
        let old_rightmost = node.rightmost_child;
        let n = node.entries.len();
        let s = n / 2;

        let mut right_entries = node.entries.split_off(s);
        let promote = right_entries.remove(0);

        let left_rightmost = promote.left_child;
        let right_no = self.pager.allocate_page()?;

        let right_node = InternalNode { entries: right_entries, rightmost_child: old_rightmost };
        let left_node = InternalNode { entries: node.entries, rightmost_child: left_rightmost };

        left_node.encode(self.pager.get_page_mut(parent_no)?);
        right_node.encode(self.pager.get_page_mut(right_no)?);

        self.insert_into_parent(path, parent_no, promote.key, right_no)
    }

    pub fn cursor_start(&mut self) -> Result<crate::btree::cursor::Cursor, BTreeError> {
        let mut current = self.root;
        loop {
            let page = self.pager.get_page(current)?;
            match page.page_type() {
                crate::storage::page::PAGE_TYPE_LEAF => break,
                crate::storage::page::PAGE_TYPE_INTERNAL => {
                    let node = InternalNode::decode(page);
                    current = node.entries.first().map(|e| e.left_child).unwrap_or(node.rightmost_child);
                }
                t => {
                    return Err(BTreeError::Storage(StorageError::CorruptPage(
                        current,
                        format!("unexpected page type {t}"),
                    )))
                }
            }
        }
        let page = self.pager.get_page(current)?;
        let node = LeafNode::decode(page);
        Ok(crate::btree::cursor::Cursor::from_leaf(node.entries, 0, node.next_leaf))
    }

    pub fn cursor_seek(&mut self, key: &[u8]) -> Result<crate::btree::cursor::Cursor, BTreeError> {
        let (_, leaf_no) = self.descend(key)?;
        let page = self.pager.get_page(leaf_no)?;
        let node = LeafNode::decode(page);
        let start_idx = node.entries.partition_point(|e| e.key.as_slice() < key);
        Ok(crate::btree::cursor::Cursor::from_leaf(node.entries, start_idx, node.next_leaf))
    }

    pub fn check_invariants(&mut self) -> Result<(), String> {
        self.check_node(self.root, None, None).map(|_| ())
    }

    fn check_node(&mut self, page_no: u32, lower: Option<&[u8]>, upper: Option<&[u8]>) -> Result<usize, String> {
        let page = self.pager.get_page(page_no).map_err(|e| e.to_string())?;
        match page.page_type() {
            crate::storage::page::PAGE_TYPE_LEAF => {
                let node = LeafNode::decode(page);
                for w in node.entries.windows(2) {
                    if w[0].key >= w[1].key {
                        return Err(format!("leaf {page_no}: keys out of order"));
                    }
                }
                if let (Some(e), Some(lo)) = (node.entries.first(), lower) {
                    if e.key.as_slice() < lo {
                        return Err(format!("leaf {page_no}: key below lower bound"));
                    }
                }
                if let (Some(e), Some(hi)) = (node.entries.last(), upper) {
                    if e.key.as_slice() >= hi {
                        return Err(format!("leaf {page_no}: key at/above upper bound"));
                    }
                }
                Ok(0)
            }
            crate::storage::page::PAGE_TYPE_INTERNAL => {
                let node = InternalNode::decode(page);
                if node.entries.is_empty() {
                    return Err(format!("internal {page_no}: no entries"));
                }
                let mut depths = Vec::new();
                let mut lo = lower;
                for e in &node.entries {
                    depths.push(self.check_node(e.left_child, lo, Some(e.key.as_slice()))?);
                    lo = Some(e.key.as_slice());
                }
                depths.push(self.check_node(node.rightmost_child, lo, upper)?);
                if depths.iter().any(|&d| d != depths[0]) {
                    return Err(format!("internal {page_no}: children at unequal depth {depths:?}"));
                }
                Ok(depths[0] + 1)
            }
            t => Err(format!("page {page_no}: unexpected page type {t}")),
        }
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
        // Large keys (~700 bytes) mean a leaf or internal node fills after only
        // ~5 entries, so a multi-level split cascade (through split_internal)
        // is reached in well under 100 inserts, instead of needing tens of
        // thousands of small sequential keys to overflow a 255-entry internal
        // root. The zero-padded numeric prefix preserves ascending order.
        for i in 0..100i32 {
            let key = format!("{i:06}{}", "x".repeat(700)).into_bytes();
            bt.insert(&key, b"v").unwrap();
        }
        assert_ne!(bt.root(), root, "enough inserts must force at least one split");
        for i in 0..100i32 {
            let key = format!("{i:06}{}", "x".repeat(700)).into_bytes();
            assert_eq!(bt.search(&key).unwrap(), Some(b"v".to_vec()));
        }
    }

    #[test]
    fn invariants_hold_after_many_inserts() {
        let (mut pager, root) = empty_tree();
        let mut bt = BTree::new(&mut pager, root);
        for i in 0..500i64 {
            let k = ((i * 37) % 500) as u64; // insertion order != key order
            bt.insert(&k.to_be_bytes(), format!("v{k}").as_bytes()).ok();
        }
        bt.check_invariants().unwrap();
    }

    use proptest::prelude::*;
    use proptest::collection::vec as pvec;

    proptest! {
        #[test]
        fn insert_and_scan_matches_sorted_unique_keys(keys in pvec(0u32..2000, 1..300)) {
            let (mut pager, root) = empty_tree();
            let mut bt = BTree::new(&mut pager, root);
            let mut inserted = std::collections::BTreeSet::new();
            for k in &keys {
                let key_bytes = k.to_be_bytes();
                if bt.insert(&key_bytes, b"v").is_ok() {
                    inserted.insert(*k);
                }
            }
            bt.check_invariants().unwrap();
            for k in &inserted {
                prop_assert_eq!(bt.search(&k.to_be_bytes()).unwrap(), Some(b"v".to_vec()));
            }
        }
    }
}
