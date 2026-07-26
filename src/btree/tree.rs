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

    const MIN_ENTRIES: usize = 2;

    pub fn delete(&mut self, key: &[u8]) -> Result<bool, BTreeError> {
        let (path, leaf_no) = self.descend(key)?;
        let page = self.pager.get_page(leaf_no)?;
        let mut node = LeafNode::decode(page);
        let pos = match node.entries.iter().position(|e| e.key.as_slice() == key) {
            Some(p) => p,
            None => return Ok(false),
        };
        node.entries.remove(pos);
        node.encode(self.pager.get_page_mut(leaf_no)?);

        if path.is_empty() {
            return Ok(true);
        }
        self.rebalance_leaf(path, leaf_no)?;
        Ok(true)
    }

    fn find_siblings(&mut self, parent_no: u32, child_no: u32) -> Result<(Option<u32>, Option<u32>), BTreeError> {
        let page = self.pager.get_page(parent_no)?;
        let parent = InternalNode::decode(page);
        let children: Vec<u32> = parent
            .entries
            .iter()
            .map(|e| e.left_child)
            .chain(std::iter::once(parent.rightmost_child))
            .collect();
        let idx = children.iter().position(|&c| c == child_no).expect("child must belong to parent");
        let left = if idx > 0 { Some(children[idx - 1]) } else { None };
        let right = if idx + 1 < children.len() { Some(children[idx + 1]) } else { None };
        Ok((left, right))
    }

    fn update_separator(&mut self, parent_no: u32, left_child: u32, new_key: Vec<u8>) -> Result<(), BTreeError> {
        let page = self.pager.get_page(parent_no)?;
        let mut parent = InternalNode::decode(page);
        let i = parent.entries.iter().position(|e| e.left_child == left_child).unwrap();
        parent.entries[i].key = new_key;
        parent.encode(self.pager.get_page_mut(parent_no)?);
        Ok(())
    }

    fn rebalance_leaf(&mut self, mut path: Vec<u32>, node_no: u32) -> Result<(), BTreeError> {
        let node = LeafNode::decode(self.pager.get_page(node_no)?);
        if node.entries.len() >= Self::MIN_ENTRIES {
            return Ok(());
        }
        let parent_no = path.pop().unwrap();
        let (left_sib, right_sib) = self.find_siblings(parent_no, node_no)?;

        if let Some(right_no) = right_sib {
            let right = LeafNode::decode(self.pager.get_page(right_no)?);
            if right.entries.len() > Self::MIN_ENTRIES {
                return self.borrow_from_right_leaf(parent_no, node_no, right_no);
            }
        }
        if let Some(left_no) = left_sib {
            let left = LeafNode::decode(self.pager.get_page(left_no)?);
            if left.entries.len() > Self::MIN_ENTRIES {
                return self.borrow_from_left_leaf(parent_no, left_no, node_no);
            }
        }
        if let Some(right_no) = right_sib {
            return self.merge_leaves(path, parent_no, node_no, right_no);
        }
        if let Some(left_no) = left_sib {
            return self.merge_leaves(path, parent_no, left_no, node_no);
        }
        Ok(())
    }

    fn borrow_from_right_leaf(&mut self, parent_no: u32, node_no: u32, right_no: u32) -> Result<(), BTreeError> {
        let mut node = LeafNode::decode(self.pager.get_page(node_no)?);
        let mut right = LeafNode::decode(self.pager.get_page(right_no)?);
        let moved = right.entries.remove(0);
        let new_separator = right.entries[0].key.clone();
        node.entries.push(moved);
        node.encode(self.pager.get_page_mut(node_no)?);
        right.encode(self.pager.get_page_mut(right_no)?);
        self.update_separator(parent_no, node_no, new_separator)
    }

    fn borrow_from_left_leaf(&mut self, parent_no: u32, left_no: u32, node_no: u32) -> Result<(), BTreeError> {
        let mut left = LeafNode::decode(self.pager.get_page(left_no)?);
        let mut node = LeafNode::decode(self.pager.get_page(node_no)?);
        let moved = left.entries.pop().unwrap();
        let new_separator = moved.key.clone();
        node.entries.insert(0, moved);
        left.encode(self.pager.get_page_mut(left_no)?);
        node.encode(self.pager.get_page_mut(node_no)?);
        self.update_separator(parent_no, left_no, new_separator)
    }

    fn merge_leaves(&mut self, path: Vec<u32>, parent_no: u32, left_no: u32, right_no: u32) -> Result<(), BTreeError> {
        let mut left = LeafNode::decode(self.pager.get_page(left_no)?);
        let right = LeafNode::decode(self.pager.get_page(right_no)?);
        left.entries.extend(right.entries);
        left.next_leaf = right.next_leaf;
        left.encode(self.pager.get_page_mut(left_no)?);
        self.pager.free_page(right_no)?;

        let page = self.pager.get_page(parent_no)?;
        let mut parent = InternalNode::decode(page);
        let i = parent.entries.iter().position(|e| e.left_child == left_no).unwrap();
        parent.entries.remove(i);
        if parent.rightmost_child == right_no {
            parent.rightmost_child = left_no;
        } else {
            let j = parent.entries.iter().position(|e| e.left_child == right_no).unwrap();
            parent.entries[j].left_child = left_no;
        }
        parent.encode(self.pager.get_page_mut(parent_no)?);

        self.rebalance_internal(path, parent_no)
    }

    fn rebalance_internal(&mut self, mut path: Vec<u32>, node_no: u32) -> Result<(), BTreeError> {
        if node_no == self.root {
            let node = InternalNode::decode(self.pager.get_page(node_no)?);
            if node.entries.is_empty() {
                self.root = node.rightmost_child;
            }
            return Ok(());
        }
        let node = InternalNode::decode(self.pager.get_page(node_no)?);
        if node.entries.len() >= Self::MIN_ENTRIES {
            return Ok(());
        }
        let parent_no = path.pop().unwrap();
        let (left_sib, right_sib) = self.find_siblings(parent_no, node_no)?;

        if let Some(right_no) = right_sib {
            let right = InternalNode::decode(self.pager.get_page(right_no)?);
            if right.entries.len() > Self::MIN_ENTRIES {
                return self.borrow_from_right_internal(parent_no, node_no, right_no);
            }
        }
        if let Some(left_no) = left_sib {
            let left = InternalNode::decode(self.pager.get_page(left_no)?);
            if left.entries.len() > Self::MIN_ENTRIES {
                return self.borrow_from_left_internal(parent_no, left_no, node_no);
            }
        }
        if let Some(right_no) = right_sib {
            return self.merge_internal(path, parent_no, node_no, right_no);
        }
        if let Some(left_no) = left_sib {
            return self.merge_internal(path, parent_no, left_no, node_no);
        }
        Ok(())
    }

    fn borrow_from_right_internal(&mut self, parent_no: u32, node_no: u32, right_no: u32) -> Result<(), BTreeError> {
        let mut node = InternalNode::decode(self.pager.get_page(node_no)?);
        let mut right = InternalNode::decode(self.pager.get_page(right_no)?);
        let page = self.pager.get_page(parent_no)?;
        let mut parent = InternalNode::decode(page);
        let sep_idx = parent.entries.iter().position(|e| e.left_child == node_no).unwrap();
        let sep_key = parent.entries[sep_idx].key.clone();

        let moved = right.entries.remove(0);
        node.entries.push(InternalEntry { key: sep_key, left_child: node.rightmost_child });
        node.rightmost_child = moved.left_child;
        parent.entries[sep_idx].key = moved.key;

        node.encode(self.pager.get_page_mut(node_no)?);
        right.encode(self.pager.get_page_mut(right_no)?);
        parent.encode(self.pager.get_page_mut(parent_no)?);
        Ok(())
    }

    fn borrow_from_left_internal(&mut self, parent_no: u32, left_no: u32, node_no: u32) -> Result<(), BTreeError> {
        let mut left = InternalNode::decode(self.pager.get_page(left_no)?);
        let mut node = InternalNode::decode(self.pager.get_page(node_no)?);
        let page = self.pager.get_page(parent_no)?;
        let mut parent = InternalNode::decode(page);
        let sep_idx = parent.entries.iter().position(|e| e.left_child == left_no).unwrap();
        let sep_key = parent.entries[sep_idx].key.clone();

        let moved_child = left.rightmost_child;
        let last_entry = left.entries.pop().unwrap();
        left.rightmost_child = last_entry.left_child;

        node.entries.insert(0, InternalEntry { key: sep_key, left_child: moved_child });
        parent.entries[sep_idx].key = last_entry.key;

        left.encode(self.pager.get_page_mut(left_no)?);
        node.encode(self.pager.get_page_mut(node_no)?);
        parent.encode(self.pager.get_page_mut(parent_no)?);
        Ok(())
    }

    fn merge_internal(&mut self, path: Vec<u32>, parent_no: u32, left_no: u32, right_no: u32) -> Result<(), BTreeError> {
        let mut left = InternalNode::decode(self.pager.get_page(left_no)?);
        let right = InternalNode::decode(self.pager.get_page(right_no)?);
        let page = self.pager.get_page(parent_no)?;
        let mut parent = InternalNode::decode(page);
        let sep_idx = parent.entries.iter().position(|e| e.left_child == left_no).unwrap();
        let sep_key = parent.entries[sep_idx].key.clone();

        left.entries.push(InternalEntry { key: sep_key, left_child: left.rightmost_child });
        left.entries.extend(right.entries);
        left.rightmost_child = right.rightmost_child;
        left.encode(self.pager.get_page_mut(left_no)?);
        self.pager.free_page(right_no)?;

        parent.entries.remove(sep_idx);
        if parent.rightmost_child == right_no {
            parent.rightmost_child = left_no;
        } else {
            let j = parent.entries.iter().position(|e| e.left_child == right_no).unwrap();
            parent.entries[j].left_child = left_no;
        }
        parent.encode(self.pager.get_page_mut(parent_no)?);

        self.rebalance_internal(path, parent_no)
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
    fn delete_removes_key_from_single_leaf() {
        let (mut pager, root) = empty_tree();
        let mut bt = BTree::new(&mut pager, root);
        bt.insert(&[1], b"a").unwrap();
        bt.insert(&[2], b"b").unwrap();
        assert!(bt.delete(&[1]).unwrap());
        assert_eq!(bt.search(&[1]).unwrap(), None);
        assert_eq!(bt.search(&[2]).unwrap(), Some(b"b".to_vec()));
    }

    #[test]
    fn delete_missing_key_returns_false() {
        let (mut pager, root) = empty_tree();
        let mut bt = BTree::new(&mut pager, root);
        bt.insert(&[1], b"a").unwrap();
        assert!(!bt.delete(&[99]).unwrap());
    }

    #[test]
    fn delete_causing_leaf_underflow_borrows_or_merges_and_stays_valid() {
        let (mut pager, root) = empty_tree();
        let mut bt = BTree::new(&mut pager, root);
        // Large keys (~700 bytes, same technique as Task 10's split test) so a
        // leaf holds only ~5 entries before splitting: 30 keys force several
        // leaf splits, and deleting all but the last 5 forces the earlier
        // leaves below MIN_ENTRIES, genuinely exercising borrow/merge rather
        // than a tree that never splits in the first place (small 4-byte keys
        // fit 300+ entries in a single root leaf, as the reviewer verified by
        // executing the original version of this test).
        let key = |i: u32| format!("{i:06}{}", "x".repeat(700)).into_bytes();
        for i in 0..30u32 {
            bt.insert(&key(i), b"v").unwrap();
        }
        for i in 0..25u32 {
            assert!(bt.delete(&key(i)).unwrap());
        }
        bt.check_invariants().unwrap();
        for i in 0..25u32 {
            assert_eq!(bt.search(&key(i)).unwrap(), None);
        }
        for i in 25..30u32 {
            assert_eq!(bt.search(&key(i)).unwrap(), Some(b"v".to_vec()));
        }
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

    proptest! {
        #[test]
        fn insert_delete_sequence_stays_consistent(
            ops in pvec((0u32..200, any::<bool>()), 1..400)
        ) {
            let (mut pager, root) = empty_tree();
            let mut bt = BTree::new(&mut pager, root);
            let mut model = std::collections::BTreeSet::new();

            for (k, is_insert) in ops {
                if is_insert {
                    if bt.insert(&k.to_be_bytes(), b"v").is_ok() {
                        model.insert(k);
                    }
                } else if bt.delete(&k.to_be_bytes()).unwrap() {
                    model.remove(&k);
                }
                bt.check_invariants().unwrap();
            }

            for k in &model {
                prop_assert_eq!(bt.search(&k.to_be_bytes()).unwrap(), Some(b"v".to_vec()));
            }
            let mut cursor = bt.cursor_start().unwrap();
            let mut scanned = Vec::new();
            while let Some((key, _)) = cursor.next(&mut pager).unwrap() {
                scanned.push(u32::from_be_bytes(key.try_into().unwrap()));
            }
            let expected: Vec<u32> = model.iter().copied().collect();
            prop_assert_eq!(scanned, expected);
        }
    }
}
