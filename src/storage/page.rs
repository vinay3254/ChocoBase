pub const PAGE_SIZE: usize = 4096;
pub const PAGE_TYPE_INTERNAL: u8 = 1;
pub const PAGE_TYPE_LEAF: u8 = 2;
pub const PAGE_TYPE_FREE: u8 = 3;

#[derive(Clone)]
pub struct Page {
    pub data: [u8; PAGE_SIZE],
}

impl Page {
    pub fn zeroed() -> Self {
        Page {
            data: [0u8; PAGE_SIZE],
        }
    }

    pub fn read_u8(&self, offset: usize) -> u8 {
        self.data[offset]
    }

    pub fn write_u8(&mut self, offset: usize, v: u8) {
        self.data[offset] = v;
    }

    pub fn read_u16(&self, offset: usize) -> u16 {
        u16::from_be_bytes([self.data[offset], self.data[offset + 1]])
    }

    pub fn write_u16(&mut self, offset: usize, v: u16) {
        self.data[offset..offset + 2].copy_from_slice(&v.to_be_bytes());
    }

    pub fn read_u32(&self, offset: usize) -> u32 {
        u32::from_be_bytes(self.data[offset..offset + 4].try_into().unwrap())
    }

    pub fn write_u32(&mut self, offset: usize, v: u32) {
        self.data[offset..offset + 4].copy_from_slice(&v.to_be_bytes());
    }

    pub fn read_bytes(&self, offset: usize, len: usize) -> &[u8] {
        &self.data[offset..offset + len]
    }

    pub fn write_bytes(&mut self, offset: usize, bytes: &[u8]) {
        self.data[offset..offset + bytes.len()].copy_from_slice(bytes);
    }

    pub fn page_type(&self) -> u8 {
        self.read_u8(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_write_roundtrip() {
        let mut page = Page::zeroed();
        page.write_u8(0, 7);
        page.write_u16(1, 1000);
        page.write_u32(3, 70000);
        page.write_bytes(7, b"hello");
        assert_eq!(page.read_u8(0), 7);
        assert_eq!(page.read_u16(1), 1000);
        assert_eq!(page.read_u32(3), 70000);
        assert_eq!(page.read_bytes(7, 5), b"hello");
    }
}
