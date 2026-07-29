use std::env;
use std::process::ExitCode;

use dbengine::Database;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let Some(path) = args.get(1) else {
        eprintln!("usage: dbengine <database-file>");
        return ExitCode::FAILURE;
    };

    let path = std::path::Path::new(path);
    let db = if path.exists() {
        Database::open(path)
    } else {
        Database::create(path)
    };

    match db {
        Ok(db) => {
            dbengine::repl::run(db);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error opening database: {e}");
            ExitCode::FAILURE
        }
    }
}
