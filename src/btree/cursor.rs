use crate::error::BTreeError;
use crate::btree::node::LeafEntry;
use crate::storage::pager::Pager;

pub struct Cursor {
    index: usize,
    entries: Vec<LeafEntry>,
    next_leaf: u32,
    finished: bool,
}

impl Cursor {
    pub fn empty() -> Self {
        Cursor { index: 0, entries: Vec::new(), next_leaf: 0, finished: true }
    }

    pub fn next(&mut self, pager: &mut Pager) -> Result<Option<(Vec<u8>, Vec<u8>)>, BTreeError> {
        if self.finished {
            return Ok(None);
        }
        if self.index >= self.entries.len() {
            if self.next_leaf == 0 {
                self.finished = true;
                return Ok(None);
            }
            let page = pager.get_page(self.next_leaf)?;
            let node = crate::btree::node::LeafNode::decode(page);
            self.next_leaf = node.next_leaf;
            self.entries = node.entries;
            self.index = 0;
            return self.next(pager);
        }
        let e = self.entries[self.index].clone();
        self.index += 1;
        Ok(Some((e.key, e.payload)))
    }
}

impl Cursor {
    pub(crate) fn from_leaf(entries: Vec<LeafEntry>, index: usize, next_leaf: u32) -> Self {
        Cursor { index, entries, next_leaf, finished: false }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tree::BTree;
    use crate::storage::pager::Pager;
    use crate::btree::node::LeafNode;
    use tempfile::NamedTempFile;

    fn build(keys: &[u32]) -> (Pager, u32) {
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let initial_root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(initial_root).unwrap());
        let final_root = {
            let mut bt = BTree::new(&mut pager, initial_root);
            for &k in keys {
                bt.insert(&k.to_be_bytes(), format!("v{k}").as_bytes()).unwrap();
            }
            bt.root()
        };
        pager.flush().unwrap();
        (pager, final_root)
    }

    #[test]
    fn full_scan_returns_all_keys_in_order() {
        let (mut pager, root) = build(&[5, 1, 3, 2, 4]);
        let mut cursor = { BTree::new(&mut pager, root).cursor_start().unwrap() };
        let mut seen = Vec::new();
        while let Some((k, _)) = cursor.next(&mut pager).unwrap() {
            seen.push(u32::from_be_bytes(k.try_into().unwrap()));
        }
        assert_eq!(seen, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn seek_positions_at_first_key_greater_or_equal() {
        let (mut pager, root) = build(&[10, 20, 30, 40]);
        let mut cursor = { BTree::new(&mut pager, root).cursor_seek(&25u32.to_be_bytes()).unwrap() };
        let (k, _) = cursor.next(&mut pager).unwrap().unwrap();
        assert_eq!(u32::from_be_bytes(k.try_into().unwrap()), 30);
    }

    #[test]
    fn seek_with_prefix_key_lands_on_first_entry_with_that_prefix() {
        // Composite index keys later in this project are variable-length byte
        // strings, and an index seek's search key is a proper prefix of the
        // full stored keys it should match (see IndexSeek, Task 35). Rust's
        // lexicographic slice Ord treats a proper prefix as "less than" its
        // extension, so seeking [1,2] against stored keys [1,1] < [1,2,3] < [1,3]
        // must land on [1,2,3] -- not skip past it as if [1,2,3] were smaller
        // than the shorter search key [1,2].
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let initial_root = pager.allocate_page().unwrap();
        LeafNode { entries: vec![], next_leaf: 0 }.encode(pager.get_page_mut(initial_root).unwrap());
        let final_root = {
            let mut bt = BTree::new(&mut pager, initial_root);
            for key in [vec![1u8, 1], vec![1, 2, 3], vec![1, 3]] {
                bt.insert(&key, b"v").unwrap();
            }
            bt.root()
        };
        pager.flush().unwrap();

        let mut cursor = { BTree::new(&mut pager, final_root).cursor_seek(&[1, 2]).unwrap() };
        let (k, _) = cursor.next(&mut pager).unwrap().unwrap();
        assert_eq!(k, vec![1, 2, 3]);
    }
}
