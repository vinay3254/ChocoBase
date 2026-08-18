use crate::types::schema::TableSchema;
use crate::types::value::{ColumnType, Value};

pub fn encode_row(schema: &TableSchema, values: &[Value]) -> Vec<u8> {
    let bitmap_len = schema.columns.len().div_ceil(8);
    let mut bitmap = vec![0u8; bitmap_len];
    for (i, v) in values.iter().enumerate() {
        if matches!(v, Value::Null) {
            bitmap[i / 8] |= 1 << (i % 8);
        }
    }

    let mut out = bitmap;
    for v in values {
        match v {
            Value::Null => {}
            Value::Integer(i) => out.extend(&i.to_le_bytes()),
            Value::Boolean(b) => out.push(if *b { 1 } else { 0 }),
            Value::Text(s) | Value::Json(s) => {
                assert!(
                    s.len() <= u16::MAX as usize,
                    "text or JSON value of {} bytes exceeds the {}-byte length-prefix limit",
                    s.len(),
                    u16::MAX
                );
                out.extend(&(s.len() as u16).to_le_bytes());
                out.extend(s.as_bytes());
            }
        }
    }
    out
}

pub fn decode_row(schema: &TableSchema, data: &[u8]) -> Vec<Value> {
    let bitmap_len = schema.columns.len().div_ceil(8);
    let bitmap = &data[0..bitmap_len];
    let mut pos = bitmap_len;
    let mut values = Vec::with_capacity(schema.columns.len());

    for (i, col) in schema.columns.iter().enumerate() {
        let is_null = bitmap[i / 8] & (1 << (i % 8)) != 0;
        if is_null {
            values.push(Value::Null);
            continue;
        }
        match col.ty {
            ColumnType::Integer => {
                let v = i64::from_le_bytes(data[pos..pos + 8].try_into().unwrap());
                pos += 8;
                values.push(Value::Integer(v));
            }
            ColumnType::Boolean => {
                values.push(Value::Boolean(data[pos] != 0));
                pos += 1;
            }
            ColumnType::Text => {
                let len = u16::from_le_bytes(data[pos..pos + 2].try_into().unwrap()) as usize;
                pos += 2;
                let s = String::from_utf8(data[pos..pos + len].to_vec()).unwrap();
                pos += len;
                values.push(Value::Text(s));
            }
            ColumnType::Json => {
                let len = u16::from_le_bytes(data[pos..pos + 2].try_into().unwrap()) as usize;
                pos += 2;
                let s = String::from_utf8(data[pos..pos + len].to_vec()).unwrap();
                pos += len;
                values.push(Value::Json(s));
            }
        }
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::schema::Column;

    fn schema() -> TableSchema {
        TableSchema {
            name: "t".into(),
            columns: vec![
                Column {
                    name: "id".into(),
                    ty: ColumnType::Integer,
                    not_null: true,
                    is_primary_key: true,
                },
                Column {
                    name: "name".into(),
                    ty: ColumnType::Text,
                    not_null: false,
                    is_primary_key: false,
                },
                Column {
                    name: "active".into(),
                    ty: ColumnType::Boolean,
                    not_null: false,
                    is_primary_key: false,
                },
            ],
            root_page: 0,
            rls_enabled: false,
        }
    }

    #[test]
    fn roundtrip_no_nulls() {
        let s = schema();
        let values = vec![
            Value::Integer(42),
            Value::Text("hi".into()),
            Value::Boolean(true),
        ];
        let encoded = encode_row(&s, &values);
        let decoded = decode_row(&s, &encoded);
        assert_eq!(decoded, values);
    }

    #[test]
    #[should_panic(expected = "exceeds the")]
    fn text_exceeding_u16_length_prefix_panics_instead_of_silently_truncating() {
        let s = schema();
        let huge = "x".repeat(u16::MAX as usize + 1);
        let values = vec![Value::Integer(1), Value::Text(huge), Value::Null];
        encode_row(&s, &values);
    }

    #[test]
    fn roundtrip_with_nulls() {
        let s = schema();
        let values = vec![Value::Integer(1), Value::Null, Value::Null];
        let encoded = encode_row(&s, &values);
        let decoded = decode_row(&s, &encoded);
        assert_eq!(decoded, values);
    }

    use proptest::prelude::*;

    proptest! {
        #[test]
        fn roundtrip_random_values(
            id in any::<i64>(),
            name in prop::option::of("[a-zA-Z0-9]{0,20}"),
            active in prop::option::of(any::<bool>()),
        ) {
            let s = schema();
            let values = vec![
                Value::Integer(id),
                name.map(Value::Text).unwrap_or(Value::Null),
                active.map(Value::Boolean).unwrap_or(Value::Null),
            ];
            let encoded = encode_row(&s, &values);
            let decoded = decode_row(&s, &encoded);
            prop_assert_eq!(decoded, values);
        }
    }
}
