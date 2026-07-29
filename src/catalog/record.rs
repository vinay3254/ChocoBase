use crate::types::schema::{Column, IndexSchema, TableSchema};
use crate::types::value::ColumnType;

const KIND_TABLE: u8 = 1;
const KIND_INDEX: u8 = 2;
const TYPE_INTEGER: u8 = 1;
const TYPE_TEXT: u8 = 2;
const TYPE_BOOLEAN: u8 = 3;

pub fn record_kind(data: &[u8]) -> u8 {
    data[0]
}

fn write_string(out: &mut Vec<u8>, s: &str) {
    out.extend(&(s.len() as u16).to_le_bytes());
    out.extend(s.as_bytes());
}

fn read_string(data: &[u8], pos: usize) -> (String, usize) {
    let len = u16::from_le_bytes(data[pos..pos + 2].try_into().unwrap()) as usize;
    let s = String::from_utf8(data[pos + 2..pos + 2 + len].to_vec()).unwrap();
    (s, pos + 2 + len)
}

fn type_tag(ty: &ColumnType) -> u8 {
    match ty {
        ColumnType::Integer => TYPE_INTEGER,
        ColumnType::Text => TYPE_TEXT,
        ColumnType::Boolean => TYPE_BOOLEAN,
    }
}

fn type_from_tag(tag: u8) -> ColumnType {
    match tag {
        TYPE_INTEGER => ColumnType::Integer,
        TYPE_TEXT => ColumnType::Text,
        TYPE_BOOLEAN => ColumnType::Boolean,
        _ => panic!("unknown column type tag {tag}"),
    }
}

pub fn encode_table_record(schema: &TableSchema) -> Vec<u8> {
    let mut out = vec![KIND_TABLE];
    write_string(&mut out, &schema.name);
    out.extend(&schema.root_page.to_le_bytes());
    out.extend(&(schema.columns.len() as u16).to_le_bytes());
    for col in &schema.columns {
        write_string(&mut out, &col.name);
        out.push(type_tag(&col.ty));
        out.push(col.not_null as u8);
        out.push(col.is_primary_key as u8);
    }
    out
}

pub fn decode_table_record(data: &[u8]) -> TableSchema {
    assert_eq!(data[0], KIND_TABLE);
    let mut pos = 1;
    let (name, next) = read_string(data, pos);
    pos = next;
    let root_page = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap());
    pos += 4;
    let num_cols = u16::from_le_bytes(data[pos..pos + 2].try_into().unwrap()) as usize;
    pos += 2;
    let mut columns = Vec::with_capacity(num_cols);
    for _ in 0..num_cols {
        let (cname, next) = read_string(data, pos);
        pos = next;
        let ty = type_from_tag(data[pos]);
        pos += 1;
        let not_null = data[pos] != 0;
        pos += 1;
        let is_primary_key = data[pos] != 0;
        pos += 1;
        columns.push(Column { name: cname, ty, not_null, is_primary_key });
    }
    TableSchema { name, columns, root_page }
}

pub fn encode_index_record(schema: &IndexSchema) -> Vec<u8> {
    let mut out = vec![KIND_INDEX];
    write_string(&mut out, &schema.name);
    write_string(&mut out, &schema.table);
    write_string(&mut out, &schema.column);
    out.extend(&schema.root_page.to_le_bytes());
    out
}

pub fn decode_index_record(data: &[u8]) -> IndexSchema {
    assert_eq!(data[0], KIND_INDEX);
    let mut pos = 1;
    let (name, next) = read_string(data, pos);
    pos = next;
    let (table, next) = read_string(data, pos);
    pos = next;
    let (column, next) = read_string(data, pos);
    pos = next;
    let root_page = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap());
    IndexSchema { name, table, column, root_page }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_record_roundtrip() {
        let schema = TableSchema {
            name: "users".into(),
            columns: vec![
                Column { name: "id".into(), ty: ColumnType::Integer, not_null: true, is_primary_key: true },
                Column { name: "email".into(), ty: ColumnType::Text, not_null: false, is_primary_key: false },
            ],
            root_page: 7,
        };
        let encoded = encode_table_record(&schema);
        assert_eq!(record_kind(&encoded), KIND_TABLE);
        let decoded = decode_table_record(&encoded);
        assert_eq!(decoded.name, "users");
        assert_eq!(decoded.root_page, 7);
        assert_eq!(decoded.columns.len(), 2);
        assert_eq!(decoded.columns[0].name, "id");
        assert!(decoded.columns[0].is_primary_key);
        assert_eq!(decoded.columns[1].ty, ColumnType::Text);
    }

    #[test]
    fn index_record_roundtrip() {
        let schema = IndexSchema {
            name: "idx_email".into(),
            table: "users".into(),
            column: "email".into(),
            root_page: 12,
        };
        let encoded = encode_index_record(&schema);
        assert_eq!(record_kind(&encoded), KIND_INDEX);
        let decoded = decode_index_record(&encoded);
        assert_eq!(decoded.name, "idx_email");
        assert_eq!(decoded.table, "users");
        assert_eq!(decoded.column, "email");
        assert_eq!(decoded.root_page, 12);
    }
}
