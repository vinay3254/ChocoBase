//! ChocoBase Daemon Server Binary (`chocod`)
//!
//! Accepts incoming client connections over TCP & HTTP REST, executes queries via `SharedDatabase`,
//! and manages session lifetimes.

use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;

use dbengine::{HttpServer, Server, ServerConfig, SharedDatabase};

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let mut bind_addr: SocketAddr = "127.0.0.1:8765".parse().unwrap();
    let mut http_addr: Option<SocketAddr> = None;
    let mut db_path = PathBuf::from("chocobase.db");

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--bind" | "-b" => {
                if i + 1 < args.len() {
                    match args[i + 1].parse() {
                        Ok(addr) => bind_addr = addr,
                        Err(e) => {
                            eprintln!("invalid bind address '{}': {e}", args[i + 1]);
                            return ExitCode::FAILURE;
                        }
                    }
                    i += 2;
                } else {
                    eprintln!("missing argument for --bind");
                    return ExitCode::FAILURE;
                }
            }
            "--http-bind" => {
                if i + 1 < args.len() {
                    match args[i + 1].parse() {
                        Ok(addr) => http_addr = Some(addr),
                        Err(e) => {
                            eprintln!("invalid http bind address '{}': {e}", args[i + 1]);
                            return ExitCode::FAILURE;
                        }
                    }
                    i += 2;
                } else {
                    eprintln!("missing argument for --http-bind");
                    return ExitCode::FAILURE;
                }
            }
            "--db" | "-d" => {
                if i + 1 < args.len() {
                    db_path = PathBuf::from(&args[i + 1]);
                    i += 2;
                } else {
                    eprintln!("missing argument for --db");
                    return ExitCode::FAILURE;
                }
            }
            "--help" | "-h" => {
                println!("ChocoBase Server Daemon (chocod)");
                println!("Usage: chocod [options]");
                println!();
                println!("Options:");
                println!("  -b, --bind <ADDR:PORT>        TCP wire protocol bind address (default: 127.0.0.1:8765)");
                println!("      --http-bind <ADDR:PORT>   HTTP REST gateway bind address (e.g. 127.0.0.1:8080)");
                println!("  -d, --db <PATH>               Path to database file (default: chocobase.db)");
                println!("  -h, --help                    Print help message");
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("unknown argument '{other}'. Use --help for usage.");
                return ExitCode::FAILURE;
            }
        }
    }

    println!("Starting ChocoBase TCP wire server on {bind_addr} (db: {})", db_path.display());
    let config = ServerConfig::new(bind_addr, db_path.clone());
    let server = match Server::new(config) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("failed to initialize database server: {e}");
            return ExitCode::FAILURE;
        }
    };

    let _http_server = if let Some(http_addr) = http_addr {
        println!("Starting ChocoBase HTTP REST gateway on {http_addr}");
        let db = match SharedDatabase::open(&db_path) {
            Ok(db) => db,
            Err(_) => match SharedDatabase::create(&db_path) {
                Ok(db) => db,
                Err(e) => {
                    eprintln!("failed to initialize HTTP database handle: {e}");
                    return ExitCode::FAILURE;
                }
            },
        };
        match HttpServer::bind(http_addr, db).await {
            Ok((http_srv, _)) => Some(http_srv),
            Err(e) => {
                eprintln!("failed to bind HTTP gateway: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        None
    };

    tokio::select! {
        res = server.run() => {
            if let Err(e) = res {
                eprintln!("server error: {e}");
                return ExitCode::FAILURE;
            }
        }
        _ = tokio::signal::ctrl_c() => {
            println!("\nShutdown signal received, shutting down cleanly...");
            server.shutdown();
        }
    }

    ExitCode::SUCCESS
}
