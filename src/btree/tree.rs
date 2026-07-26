use crate::error::{BTreeError, StorageError};
use crate::storage::page::{PAGE_TYPE_INTERNAL, PAGE_TYPE_LEAF};
use crate::storage::pager::Pager;
use crate::btree::node::{InternalNode, LeafNode};

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
}
