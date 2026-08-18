use crate::types::schema::{Column, IndexSchema, PolicyCmd, PolicySchema, TableSchema};
use crate::types::value::ColumnType;

pub const KIND_TABLE: u8 = 1;
pub const KIND_INDEX: u8 = 2;
pub const KIND_POLICY: u8 = 3;

const TYPE_INTEGER: u8 = 1;
const TYPE_TEXT: u8 = 2;
const TYPE_BOOLEAN: u8 = 3;
const TYPE_JSON: u8 = 4;

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
        ColumnType::Json => TYPE_JSON,
    }
}

fn type_from_tag(tag: u8) -> ColumnType {
    match tag {
        TYPE_INTEGER => ColumnType::Integer,
        TYPE_TEXT => ColumnType::Text,
        TYPE_BOOLEAN => ColumnType::Boolean,
        TYPE_JSON => ColumnType::Json,
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
    out.push(schema.rls_enabled as u8);
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
        columns.push(Column {
            name: cname,
            ty,
            not_null,
            is_primary_key,
        });
    }
    let rls_enabled = if pos < data.len() {
        data[pos] != 0
    } else {
        false
    };
    TableSchema {
        name,
        columns,
        root_page,
        rls_enabled,
    }
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
    IndexSchema {
        name,
        table,
        column,
        root_page,
    }
}

pub fn encode_policy_record(schema: &PolicySchema) -> Vec<u8> {
    let mut out = vec![KIND_POLICY];
    write_string(&mut out, &schema.name);
    write_string(&mut out, &schema.table);
    let cmd_tag = match schema.cmd {
        PolicyCmd::Select => 0,
        PolicyCmd::Insert => 1,
        PolicyCmd::Update => 2,
        PolicyCmd::Delete => 3,
        PolicyCmd::All => 4,
    };
    out.push(cmd_tag);
    let using_json = serde_json::to_string(&schema.using_expr).unwrap_or_default();
    write_string(&mut out, &using_json);
    let check_json = serde_json::to_string(&schema.with_check).unwrap_or_default();
    write_string(&mut out, &check_json);
    out
}

pub fn decode_policy_record(data: &[u8]) -> PolicySchema {
    assert_eq!(data[0], KIND_POLICY);
    let mut pos = 1;
    let (name, next) = read_string(data, pos);
    pos = next;
    let (table, next) = read_string(data, pos);
    pos = next;
    let cmd = match data[pos] {
        0 => PolicyCmd::Select,
        1 => PolicyCmd::Insert,
        2 => PolicyCmd::Update,
        3 => PolicyCmd::Delete,
        _ => PolicyCmd::All,
    };
    pos += 1;
    let (using_json, next) = read_string(data, pos);
    pos = next;
    let using_expr = serde_json::from_str(&using_json).ok().flatten();
    let (check_json, _) = read_string(data, pos);
    let with_check = serde_json::from_str(&check_json).ok().flatten();

    PolicySchema {
        name,
        table,
        cmd,
        using_expr,
        with_check,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_record_roundtrip() {
        let schema = TableSchema {
            name: "users".into(),
            columns: vec![
                Column {
                    name: "id".into(),
                    ty: ColumnType::Integer,
                    not_null: true,
                    is_primary_key: true,
                },
                Column {
                    name: "email".into(),
                    ty: ColumnType::Text,
                    not_null: false,
                    is_primary_key: false,
                },
            ],
            root_page: 7,
            rls_enabled: true,
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
        assert!(decoded.rls_enabled);
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
