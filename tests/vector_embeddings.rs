use dbengine::engine::{Database, ExecResult};
use dbengine::types::value::Value;
use tempfile::NamedTempFile;

#[test]
fn test_vector_creation_insertion_and_cosine_distance() {
    let file = NamedTempFile::new().unwrap();
    let mut db = Database::create(file.path()).unwrap();

    // 1. Create table with VECTOR(3)
    db.execute("CREATE TABLE embeddings (id INTEGER PRIMARY KEY, title TEXT, vec VECTOR(3))")
        .expect("table creation with vector column failed");

    // 2. Insert vectors
    db.execute(
        "INSERT INTO embeddings (id, title, vec) VALUES \
         (1, 'x-axis', '[1.0, 0.0, 0.0]'), \
         (2, 'y-axis', '[0.0, 1.0, 0.0]'), \
         (3, 'diagonal', '[0.7071, 0.7071, 0.0]')",
    )
    .expect("vector insert failed");

    // 3. Query similarity using COSINE_DISTANCE
    let res = db
        .execute("SELECT id, title, COSINE_DISTANCE(vec, '[1.0, 0.0, 0.0]') AS dist FROM embeddings ORDER BY dist ASC")
        .expect("cosine distance query failed");

    match res {
        ExecResult::Rows { columns, rows } => {
            assert_eq!(columns, vec!["id", "title", "dist"]);
            assert_eq!(rows.len(), 3);
            // Closest to [1, 0, 0] is id 1 (dist ~ 0.0)
            assert_eq!(rows[0][0], Value::Integer(1));
            assert_eq!(rows[0][1], Value::Text("x-axis".into()));
            if let Value::Float(d) = rows[0][2] {
                assert!(d.abs() < 1e-4, "expected dist ~ 0, got {d}");
            } else {
                panic!("expected float distance");
            }

            // Second closest is id 3 (dist ~ 0.2929)
            assert_eq!(rows[1][0], Value::Integer(3));

            // Furthest is id 2 (orthogonal, dist ~ 1.0)
            assert_eq!(rows[2][0], Value::Integer(2));
            if let Value::Float(d) = rows[2][2] {
                assert!((d - 1.0).abs() < 1e-4, "expected dist ~ 1.0, got {d}");
            } else {
                panic!("expected float distance");
            }
        }
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn test_vector_l2_and_inner_product_distances() {
    let file = NamedTempFile::new().unwrap();
    let mut db = Database::create(file.path()).unwrap();

    db.execute("CREATE TABLE points (id INTEGER PRIMARY KEY, v VECTOR(2))")
        .unwrap();

    db.execute(
        "INSERT INTO points (id, v) VALUES \
         (1, '[0.0, 0.0]'), \
         (2, '[3.0, 4.0]')",
    )
    .unwrap();

    // Euclidean / L2 distance between (0,0) and (3,4) is 5.0
    let res = db
        .execute("SELECT id, L2_DISTANCE(v, '[0.0, 0.0]') AS l2 FROM points WHERE id = 2")
        .unwrap();

    match res {
        ExecResult::Rows { rows, .. } => {
            if let Value::Float(d) = rows[0][1] {
                assert!((d - 5.0).abs() < 1e-5, "expected L2 distance 5.0, got {d}");
            } else {
                panic!("expected float distance");
            }
        }
        other => panic!("unexpected result: {other:?}"),
    }

    // Inner product of (3,4) and (2,1) is 3*2 + 4*1 = 10.0
    let res = db
        .execute("SELECT id, INNER_PRODUCT(v, '[2.0, 1.0]') AS ip FROM points WHERE id = 2")
        .unwrap();

    match res {
        ExecResult::Rows { rows, .. } => {
            if let Value::Float(ip) = rows[0][1] {
                assert!(
                    (ip - 10.0).abs() < 1e-5,
                    "expected inner product 10.0, got {ip}"
                );
            } else {
                panic!("expected float inner product");
            }
        }
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn test_vector_dimension_mismatch_rejected() {
    let file = NamedTempFile::new().unwrap();
    let mut db = Database::create(file.path()).unwrap();

    db.execute("CREATE TABLE items (id INTEGER PRIMARY KEY, embedding VECTOR(3))")
        .unwrap();

    // Inserting 2-element vector into VECTOR(3) column must fail
    let err = db.execute("INSERT INTO items (id, embedding) VALUES (1, '[1.0, 2.0]')");
    assert!(err.is_err(), "expected dimension mismatch error");
}
