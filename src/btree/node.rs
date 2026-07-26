use crate::storage::page::{Page, PAGE_SIZE, PAGE_TYPE_INTERNAL, PAGE_TYPE_LEAF};

pub const NODE_HEADER_SIZE: usize = 12;

#[derive(Debug, Clone, PartialEq)]
pub struct LeafEntry {
    pub key: Vec<u8>,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct LeafNode {
    pub entries: Vec<LeafEntry>,
    pub next_leaf: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InternalEntry {
    pub key: Vec<u8>,
    pub left_child: u32,
}

#[derive(Debug, Clone)]
pub struct InternalNode {
    pub entries: Vec<InternalEntry>,
    pub rightmost_child: u32,
}

impl LeafNode {
    pub fn decode(page: &Page) -> Self {
        assert_eq!(page.page_type(), PAGE_TYPE_LEAF);
        let num_cells = page.read_u16(2) as usize;
        let next_leaf = page.read_u32(8);
        let mut entries = Vec::with_capacity(num_cells);
        for i in 0..num_cells {
            let slot_offset = NODE_HEADER_SIZE + i * 2;
            let cell_offset = page.read_u16(slot_offset) as usize;
            let key_len = page.read_u16(cell_offset) as usize;
            let key = page.read_bytes(cell_offset + 2, key_len).to_vec();
            let payload_len_offset = cell_offset + 2 + key_len;
            let payload_len = page.read_u16(payload_len_offset) as usize;
            let payload = page.read_bytes(payload_len_offset + 2, payload_len).to_vec();
            entries.push(LeafEntry { key, payload });
        }
        LeafNode { entries, next_leaf }
    }

    pub fn encode(&self, page: &mut Page) {
        page.data = [0u8; PAGE_SIZE];
        page.write_u8(0, PAGE_TYPE_LEAF);
        page.write_u16(2, self.entries.len() as u16);
        page.write_u32(8, self.next_leaf);

        let mut cell_end = PAGE_SIZE;
        for (i, e) in self.entries.iter().enumerate() {
            let cell_len = 2 + e.key.len() + 2 + e.payload.len();
            let cell_offset = cell_end - cell_len;
            page.write_u16(cell_offset, e.key.len() as u16);
            page.write_bytes(cell_offset + 2, &e.key);
            page.write_u16(cell_offset + 2 + e.key.len(), e.payload.len() as u16);
            page.write_bytes(cell_offset + 2 + e.key.len() + 2, &e.payload);
            page.write_u16(NODE_HEADER_SIZE + i * 2, cell_offset as u16);
            cell_end = cell_offset;
        }
        page.write_u16(4, (NODE_HEADER_SIZE + self.entries.len() * 2) as u16);
        page.write_u16(6, cell_end as u16);
    }

    pub fn encoded_size(&self) -> usize {
        let cells: usize = self.entries.iter().map(|e| 2 + e.key.len() + 2 + e.payload.len()).sum();
        NODE_HEADER_SIZE + self.entries.len() * 2 + cells
    }
}

impl InternalNode {
    pub fn decode(page: &Page) -> Self {
        assert_eq!(page.page_type(), PAGE_TYPE_INTERNAL);
        let num_cells = page.read_u16(2) as usize;
        let rightmost_child = page.read_u32(8);
        let mut entries = Vec::with_capacity(num_cells);
        for i in 0..num_cells {
            let slot_offset = NODE_HEADER_SIZE + i * 2;
            let cell_offset = page.read_u16(slot_offset) as usize;
            let key_len = page.read_u16(cell_offset) as usize;
            let key = page.read_bytes(cell_offset + 2, key_len).to_vec();
            let left_child = page.read_u32(cell_offset + 2 + key_len);
            entries.push(InternalEntry { key, left_child });
        }
        InternalNode { entries, rightmost_child }
    }

    pub fn encode(&self, page: &mut Page) {
        page.data = [0u8; PAGE_SIZE];
        page.write_u8(0, PAGE_TYPE_INTERNAL);
        page.write_u16(2, self.entries.len() as u16);
        page.write_u32(8, self.rightmost_child);

        let mut cell_end = PAGE_SIZE;
        for (i, e) in self.entries.iter().enumerate() {
            let cell_len = 2 + e.key.len() + 4;
            let cell_offset = cell_end - cell_len;
            page.write_u16(cell_offset, e.key.len() as u16);
            page.write_bytes(cell_offset + 2, &e.key);
            page.write_u32(cell_offset + 2 + e.key.len(), e.left_child);
            page.write_u16(NODE_HEADER_SIZE + i * 2, cell_offset as u16);
            cell_end = cell_offset;
        }
        page.write_u16(4, (NODE_HEADER_SIZE + self.entries.len() * 2) as u16);
        page.write_u16(6, cell_end as u16);
    }

    pub fn encoded_size(&self) -> usize {
        let cells: usize = self.entries.iter().map(|e| 2 + e.key.len() + 4).sum();
        NODE_HEADER_SIZE + self.entries.len() * 2 + cells
    }

    pub fn child_for_key(&self, key: &[u8]) -> u32 {
        for e in &self.entries {
            if key < e.key.as_slice() {
                return e.left_child;
            }
        }
        self.rightmost_child
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaf_encode_decode_roundtrip() {
        let node = LeafNode {
            entries: vec![
                LeafEntry { key: vec![1, 2], payload: vec![9, 9, 9] },
                LeafEntry { key: vec![1, 2, 3], payload: vec![] },
            ],
            next_leaf: 42,
        };
        let mut page = Page::zeroed();
        node.encode(&mut page);
        let decoded = LeafNode::decode(&page);
        assert_eq!(decoded.entries, node.entries);
        assert_eq!(decoded.next_leaf, 42);
        assert_eq!(page.page_type(), PAGE_TYPE_LEAF);
    }

    #[test]
    fn internal_encode_decode_roundtrip() {
        let node = InternalNode {
            entries: vec![
                InternalEntry { key: vec![5], left_child: 1 },
                InternalEntry { key: vec![10], left_child: 2 },
            ],
            rightmost_child: 3,
        };
        let mut page = Page::zeroed();
        node.encode(&mut page);
        let decoded = InternalNode::decode(&page);
        assert_eq!(decoded.entries, node.entries);
        assert_eq!(decoded.rightmost_child, 3);
        assert_eq!(page.page_type(), PAGE_TYPE_INTERNAL);
    }

    #[test]
    fn child_for_key_routes_correctly() {
        let node = InternalNode {
            entries: vec![
                InternalEntry { key: vec![5], left_child: 100 },
                InternalEntry { key: vec![10], left_child: 200 },
            ],
            rightmost_child: 300,
        };
        assert_eq!(node.child_for_key(&[1]), 100);
        assert_eq!(node.child_for_key(&[5]), 200);
        assert_eq!(node.child_for_key(&[7]), 200);
        assert_eq!(node.child_for_key(&[10]), 300);
        assert_eq!(node.child_for_key(&[99]), 300);
    }
}
