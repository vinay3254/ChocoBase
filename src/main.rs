use std::env;
use std::net::SocketAddr;
use std::path::Path;
use std::process::ExitCode;

use dbengine::engine::{Database, SharedDatabase};
use dbengine::http::HttpServer;

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 || args[1] == "--help" || args[1] == "-h" || args[1] == "help" {
        println!("ChocoBase - High-Performance Database Platform");
        println!();
        println!("Usage:");
        println!("  dbengine server [bind_addr] [database_file]   Start the HTTP REST / GraphQL / Storage / Realtime server");
        println!("  dbengine repl [database_file]                 Launch interactive SQL terminal");
        println!("  dbengine <database_file>                      Launch interactive SQL terminal");
        println!();
        println!("Examples:");
        println!("  dbengine server 127.0.0.1:8080 dev.db");
        println!("  dbengine repl dev.db");
        return ExitCode::SUCCESS;
    }

    if args[1] == "server" {
        let addr_str = args.get(2).map(|s| s.as_str()).unwrap_or("127.0.0.1:8080");
        let db_file = args.get(3).map(|s| s.as_str()).unwrap_or("chocobase.db");

        let addr: SocketAddr = match addr_str.parse() {
            Ok(a) => a,
            Err(e) => {
                eprintln!("invalid socket address '{addr_str}': {e}");
                return ExitCode::FAILURE;
            }
        };

        let path = Path::new(db_file);
        let shared_db = match SharedDatabase::open(path) {
            Ok(db) => db,
            Err(_) => match SharedDatabase::create(path) {
                Ok(db) => db,
                Err(e) => {
                    eprintln!("error opening database '{db_file}': {e}");
                    return ExitCode::FAILURE;
                }
            },
        };

        println!("⚡ ChocoBase Server listening on http://{addr}");
        println!("📂 Database file: {db_file}");
        println!("🚀 REST Gateway: http://{addr}/rest/v1");
        println!("📊 Health Check: http://{addr}/v1/health");
        println!("Press Ctrl+C to stop.");

        let (server, _bound_addr) = match HttpServer::bind(addr, shared_db.clone()).await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error starting server on {addr}: {e}");
                return ExitCode::FAILURE;
            }
        };

        let _ = tokio::signal::ctrl_c().await;
        println!("\nShutting down ChocoBase...");
        server.shutdown();
        return ExitCode::SUCCESS;
    }

    // Default to REPL mode
    let db_path_str = if args[1] == "repl" {
        args.get(2).map(|s| s.as_str()).unwrap_or("chocobase.db")
    } else {
        &args[1]
    };

    let path = Path::new(db_path_str);
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
