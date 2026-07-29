pub mod meta;

use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

use crate::engine::{Database, ExecResult};
use crate::types::value::Value;

pub fn format_value(v: &Value) -> String {
    match v {
        Value::Integer(i) => i.to_string(),
        Value::Text(s) => s.clone(),
        Value::Boolean(b) => b.to_string(),
        Value::Null => "NULL".to_string(),
    }
}

pub fn format_result(result: &ExecResult) -> String {
    match result {
        ExecResult::Ok => "OK".to_string(),
        ExecResult::Modified(n) => format!("{n} row(s) modified"),
        ExecResult::Rows { columns, rows } => {
            let mut out = String::new();
            out.push_str(&columns.join(" | "));
            out.push('\n');
            for row in rows {
                let cells: Vec<String> = row.iter().map(format_value).collect();
                out.push_str(&cells.join(" | "));
                out.push('\n');
            }
            out.push_str(&format!("({} row(s))", rows.len()));
            out
        }
    }
}

pub fn run(mut db: Database) {
    let mut rl = DefaultEditor::new().expect("failed to initialize line editor");
    let mut buffer = String::new();

    loop {
        let prompt = if buffer.is_empty() { "dbengine> " } else { "     ...> " };
        match rl.readline(prompt) {
            Ok(line) => {
                let _ = rl.add_history_entry(line.as_str());
                let trimmed = line.trim();

                if buffer.is_empty() && trimmed.starts_with('.') {
                    if trimmed == ".exit" {
                        break;
                    }
                    println!("{}", meta::dispatch(&mut db, &trimmed[1..]));
                    continue;
                }

                buffer.push_str(&line);
                buffer.push('\n');
                if buffer.trim_end().ends_with(';') {
                    db.reset_read_counter();
                    match db.execute(buffer.trim()) {
                        Ok(result) => println!("{}", format_result(&result)),
                        Err(e) => println!("error: {e}"),
                    }
                    buffer.clear();
                }
            }
            Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => break,
            Err(e) => {
                println!("error: {e}");
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_scalar_values() {
        assert_eq!(format_value(&Value::Integer(5)), "5");
        assert_eq!(format_value(&Value::Text("hi".into())), "hi");
        assert_eq!(format_value(&Value::Boolean(true)), "true");
        assert_eq!(format_value(&Value::Null), "NULL");
    }

    #[test]
    fn formats_ok_result() {
        assert_eq!(format_result(&ExecResult::Ok), "OK");
    }

    #[test]
    fn formats_modified_result() {
        assert_eq!(format_result(&ExecResult::Modified(3)), "3 row(s) modified");
    }

    #[test]
    fn formats_rows_result_with_header_and_count() {
        let result = ExecResult::Rows {
            columns: vec!["id".into(), "name".into()],
            rows: vec![
                vec![Value::Integer(1), Value::Text("a".into())],
                vec![Value::Integer(2), Value::Null],
            ],
        };
        let text = format_result(&result);
        assert!(text.contains("id | name"));
        assert!(text.contains("1 | a"));
        assert!(text.contains("2 | NULL"));
        assert!(text.contains("(2 row(s))"));
    }
}
