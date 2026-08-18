use tempfile::NamedTempFile;

use dbengine::types::value::Value;
use dbengine::{Database, ExecResult};

#[test]
fn inner_join_returns_matching_rows() {
    let file = NamedTempFile::new().unwrap();
    let mut db = Database::create(file.path()).unwrap();

    db.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)").unwrap();
    db.execute("CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER, item TEXT)").unwrap();

    db.execute("INSERT INTO users (id, name) VALUES (1, 'Alice'), (2, 'Bob'), (3, 'Charlie')").unwrap();
    db.execute("INSERT INTO orders (id, user_id, item) VALUES (10, 1, 'Book'), (20, 1, 'Pen'), (30, 2, 'Laptop')").unwrap();

    let res = db.execute(
        "SELECT users.name, orders.item FROM users INNER JOIN orders ON users.id = orders.user_id ORDER BY orders.id"
    ).unwrap();

    match res {
        ExecResult::Rows { columns, rows } => {
            assert_eq!(columns, vec!["users.name".to_string(), "orders.item".to_string()]);
            assert_eq!(
                rows,
                vec![
                    vec![Value::Text("Alice".into()), Value::Text("Book".into())],
                    vec![Value::Text("Alice".into()), Value::Text("Pen".into())],
                    vec![Value::Text("Bob".into()), Value::Text("Laptop".into())],
                ]
            );
        }
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn left_join_includes_unmatched_left_rows_with_nulls() {
    let file = NamedTempFile::new().unwrap();
    let mut db = Database::create(file.path()).unwrap();

    db.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)").unwrap();
    db.execute("CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER, item TEXT)").unwrap();

    db.execute("INSERT INTO users (id, name) VALUES (1, 'Alice'), (2, 'Bob'), (3, 'Charlie')").unwrap();
    db.execute("INSERT INTO orders (id, user_id, item) VALUES (10, 1, 'Book')").unwrap();

    let res = db.execute(
        "SELECT users.name, orders.item FROM users LEFT JOIN orders ON users.id = orders.user_id ORDER BY users.id"
    ).unwrap();

    match res {
        ExecResult::Rows { rows, .. } => {
            assert_eq!(
                rows,
                vec![
                    vec![Value::Text("Alice".into()), Value::Text("Book".into())],
                    vec![Value::Text("Bob".into()), Value::Null],
                    vec![Value::Text("Charlie".into()), Value::Null],
                ]
            );
        }
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn join_with_where_predicate_and_limit() {
    let file = NamedTempFile::new().unwrap();
    let mut db = Database::create(file.path()).unwrap();

    db.execute("CREATE TABLE customers (id INTEGER PRIMARY KEY, region TEXT)").unwrap();
    db.execute("CREATE TABLE sales (id INTEGER PRIMARY KEY, cust_id INTEGER, amount INTEGER)").unwrap();

    db.execute("INSERT INTO customers (id, region) VALUES (1, 'North'), (2, 'South'), (3, 'North')").unwrap();
    db.execute("INSERT INTO sales (id, cust_id, amount) VALUES (101, 1, 500), (102, 2, 300), (103, 3, 700)").unwrap();

    let res = db.execute(
        "SELECT sales.id, sales.amount FROM customers INNER JOIN sales ON customers.id = sales.cust_id WHERE customers.region = 'North' ORDER BY sales.amount DESC LIMIT 1"
    ).unwrap();

    match res {
        ExecResult::Rows { rows, .. } => {
            assert_eq!(rows, vec![vec![Value::Integer(103), Value::Integer(700)]]);
        }
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn scalar_aggregations_count_sum_avg_min_max() {
    let file = NamedTempFile::new().unwrap();
    let mut db = Database::create(file.path()).unwrap();

    db.execute("CREATE TABLE emp (id INTEGER PRIMARY KEY, salary INTEGER)").unwrap();
    db.execute("INSERT INTO emp (id, salary) VALUES (1, 100), (2, 200), (3, 300), (4, 400)").unwrap();

    let res = db.execute("SELECT COUNT(*), SUM(salary), AVG(salary), MIN(salary), MAX(salary) FROM emp").unwrap();
    match res {
        ExecResult::Rows { columns, rows } => {
            assert_eq!(columns.len(), 5);
            assert_eq!(
                rows,
                vec![vec![
                    Value::Integer(4),
                    Value::Integer(1000),
                    Value::Integer(250),
                    Value::Integer(100),
                    Value::Integer(400)
                ]]
            );
        }
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn group_by_aggregations() {
    let file = NamedTempFile::new().unwrap();
    let mut db = Database::create(file.path()).unwrap();

    db.execute("CREATE TABLE employees (id INTEGER PRIMARY KEY, dept TEXT, salary INTEGER)").unwrap();
    db.execute("INSERT INTO employees (id, dept, salary) VALUES (1, 'Engineering', 100), (2, 'Engineering', 200), (3, 'Sales', 300)").unwrap();

    let res = db.execute("SELECT dept, COUNT(*), SUM(salary) FROM employees GROUP BY dept").unwrap();
    match res {
        ExecResult::Rows { rows, .. } => {
            assert_eq!(rows.len(), 2);
            assert_eq!(
                rows,
                vec![
                    vec![Value::Text("Engineering".into()), Value::Integer(2), Value::Integer(300)],
                    vec![Value::Text("Sales".into()), Value::Integer(1), Value::Integer(300)],
                ]
            );
        }
        other => panic!("unexpected result: {other:?}"),
    }
}
