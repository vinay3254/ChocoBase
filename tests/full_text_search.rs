use dbengine::engine::{Database, ExecResult};
use dbengine::types::value::Value;
use tempfile::NamedTempFile;

#[test]
fn test_full_text_search_matching_and_prefix_search() {
    let file = NamedTempFile::new().unwrap();
    let mut db = Database::create(file.path()).unwrap();

    db.execute("CREATE TABLE posts (id INTEGER PRIMARY KEY, title TEXT, content TEXT)")
        .unwrap();

    db.execute(
        "INSERT INTO posts (id, title, content) VALUES \
         (1, 'Rust Engine', 'Building a high-performance database in Rust with async Tokio and WAL.'), \
         (2, 'Supabase Architecture', 'Supabase integrates PostgreSQL, Realtime WebSocket broadcast, and Auth JWT.'), \
         (3, 'Italian Cuisine', 'Making authentic handmade sourdough pizza with fresh mozzarella and basil.')",
    )
    .unwrap();

    // 1. Exact term search (case-insensitive)
    let res = db
        .execute("SELECT id, title FROM posts WHERE FTS_MATCH(content, 'database rust')")
        .unwrap();

    match res {
        ExecResult::Rows { rows, .. } => {
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0][0], Value::Integer(1));
            assert_eq!(rows[0][1], Value::Text("Rust Engine".into()));
        }
        other => panic!("unexpected result: {other:?}"),
    }

    // 2. Prefix search (e.g. 'broad*')
    let res = db
        .execute("SELECT id, title FROM posts WHERE MATCHES(content, 'realtime broad*')")
        .unwrap();

    match res {
        ExecResult::Rows { rows, .. } => {
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0][0], Value::Integer(2));
            assert_eq!(rows[0][1], Value::Text("Supabase Architecture".into()));
        }
        other => panic!("unexpected result: {other:?}"),
    }

    // 3. Non-matching query returns 0 rows
    let res = db
        .execute("SELECT id, title FROM posts WHERE FTS_MATCH(content, 'quantum computing')")
        .unwrap();

    match res {
        ExecResult::Rows { rows, .. } => {
            assert_eq!(rows.len(), 0);
        }
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn test_full_text_search_ranking_and_ordering() {
    let file = NamedTempFile::new().unwrap();
    let mut db = Database::create(file.path()).unwrap();

    db.execute("CREATE TABLE docs (id INTEGER PRIMARY KEY, body TEXT)")
        .unwrap();

    db.execute(
        "INSERT INTO docs (id, body) VALUES \
         (1, 'Database systems use SQL queries for data access.'), \
         (2, 'Database database database design for high concurrency database engine.'), \
         (3, 'Unrelated document about baking bread.')",
    )
    .unwrap();

    // Doc 2 has higher term frequency for 'database' than Doc 1
    let res = db
        .execute("SELECT id, FTS_RANK(body, 'database') AS score FROM docs WHERE FTS_MATCH(body, 'database') ORDER BY score DESC")
        .unwrap();

    match res {
        ExecResult::Rows { rows, .. } => {
            assert_eq!(rows.len(), 2);
            // Doc 2 must be ranked first
            assert_eq!(rows[0][0], Value::Integer(2));
            assert_eq!(rows[1][0], Value::Integer(1));

            if let (Value::Float(score2), Value::Float(score1)) = (&rows[0][1], &rows[1][1]) {
                assert!(
                    score2 > score1,
                    "expected score2 ({score2}) > score1 ({score1})"
                );
            } else {
                panic!("expected float scores");
            }
        }
        other => panic!("unexpected result: {other:?}"),
    }
}
