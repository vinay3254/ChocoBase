use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ColumnType {
    Integer,
    Text,
    Boolean,
    Json,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Value {
    Integer(i64),
    Text(String),
    Boolean(bool),
    Json(String),
    Null,
}

pub fn encode_key(v: &Value) -> Vec<u8> {
    match v {
        Value::Integer(i) => {
            let flipped = (*i as u64) ^ 0x8000_0000_0000_0000;
            flipped.to_be_bytes().to_vec()
        }
        Value::Boolean(b) => vec![if *b { 1 } else { 0 }],
        Value::Text(s) | Value::Json(s) => {
            let mut out = s.as_bytes().to_vec();
            out.push(0);
            out
        }
        Value::Null => panic!("NULL cannot be encoded as a key"),
    }
}

pub fn encode_composite_key(parts: &[Value]) -> Vec<u8> {
    let mut out = Vec::new();
    for p in parts {
        out.extend(encode_key(p));
    }
    out
}

pub fn sql_cmp(a: &Value, b: &Value) -> std::cmp::Ordering {
    match (a, b) {
        (Value::Integer(x), Value::Integer(y)) => x.cmp(y),
        (Value::Text(x), Value::Text(y)) => x.cmp(y),
        (Value::Json(x), Value::Json(y)) => x.cmp(y),
        (Value::Boolean(x), Value::Boolean(y)) => x.cmp(y),
        _ => panic!(
            "cannot compare values of different types: {:?} vs {:?}",
            a, b
        ),
    }
}

pub fn sql_cmp_nullable(a: &Value, b: &Value) -> std::cmp::Ordering {
    match (a, b) {
        (Value::Null, Value::Null) => std::cmp::Ordering::Equal,
        (Value::Null, _) => std::cmp::Ordering::Less,
        (_, Value::Null) => std::cmp::Ordering::Greater,
        _ => sql_cmp(a, b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_encoding_preserves_order_across_sign() {
        let neg = encode_key(&Value::Integer(-5));
        let zero = encode_key(&Value::Integer(0));
        let pos = encode_key(&Value::Integer(5));
        assert!(neg < zero);
        assert!(zero < pos);
    }

    #[test]
    fn integer_encoding_extremes() {
        let min = encode_key(&Value::Integer(i64::MIN));
        let max = encode_key(&Value::Integer(i64::MAX));
        assert!(min < max);
    }

    #[test]
    fn text_encoding_preserves_lexicographic_order() {
        let a = encode_key(&Value::Text("apple".into()));
        let b = encode_key(&Value::Text("banana".into()));
        let ab = encode_key(&Value::Text("app".into()));
        assert!(a < b);
        assert!(ab < a, "a proper prefix must sort before its extension");
    }

    #[test]
    fn boolean_encoding_orders_false_before_true() {
        assert!(encode_key(&Value::Boolean(false)) < encode_key(&Value::Boolean(true)));
    }

    #[test]
    fn composite_key_concatenates_parts_in_order() {
        let k1 = encode_composite_key(&[Value::Integer(1), Value::Text("x".into())]);
        let k2 = encode_composite_key(&[Value::Integer(1), Value::Text("y".into())]);
        assert!(k1 < k2);
    }

    use proptest::prelude::*;

    proptest! {
        #[test]
        fn integer_key_order_matches_sql_order(a in any::<i64>(), b in any::<i64>()) {
            let va = Value::Integer(a);
            let vb = Value::Integer(b);
            let byte_order = encode_key(&va).cmp(&encode_key(&vb));
            let sql_order = sql_cmp(&va, &vb);
            prop_assert_eq!(byte_order, sql_order);
        }

        #[test]
        fn text_key_order_matches_sql_order(a in "[a-zA-Z0-9]{0,12}", b in "[a-zA-Z0-9]{0,12}") {
            let va = Value::Text(a);
            let vb = Value::Text(b);
            let byte_order = encode_key(&va).cmp(&encode_key(&vb));
            let sql_order = sql_cmp(&va, &vb);
            prop_assert_eq!(byte_order, sql_order);
        }
    }
}
