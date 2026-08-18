//! ChocoBase Daemon Server Binary (`chocod`)
//!
//! Accepts incoming client connections over TCP & HTTP REST, executes queries via `SharedDatabase`,
//! and manages session lifetimes.

use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;

use dbengine::{HttpServer, Server, ServerConfig};

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let mut bind_addr: SocketAddr = "127.0.0.1:5433".parse().unwrap();
    let mut http_addr: Option<SocketAddr> = Some("127.0.0.1:8080".parse().unwrap());
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
            "--no-http" => {
                http_addr = None;
                i += 1;
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
                println!("Usage: chocod [options] [DB_PATH]");
                println!();
                println!("Options:");
                println!("  -b, --bind <ADDR:PORT>        TCP wire protocol bind address (default: 127.0.0.1:5433)");
                println!("      --http-bind <ADDR:PORT>   HTTP REST & Studio bind address (default: 127.0.0.1:8080)");
                println!(
                    "      --no-http                 Disable HTTP gateway and Studio dashboard"
                );
                println!(
                    "  -d, --db <PATH>               Path to database file (default: chocobase.db)"
                );
                println!("  -h, --help                    Print help message");
                return ExitCode::SUCCESS;
            }
            other => {
                if !other.starts_with('-') {
                    db_path = PathBuf::from(other);
                    i += 1;
                } else {
                    eprintln!("unknown argument '{other}'. Use --help for usage.");
                    return ExitCode::FAILURE;
                }
            }
        }
    }

    println!(
        "Starting ChocoBase TCP wire server on {bind_addr} (db: {})",
        db_path.display()
    );
    let config = ServerConfig::new(bind_addr, db_path.clone());
    let server = match Server::new(config) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("failed to initialize database server: {e}");
            return ExitCode::FAILURE;
        }
    };

    let _http_server = if let Some(http_addr) = http_addr {
        println!("Starting ChocoBase HTTP REST gateway & Studio on http://{http_addr}/dashboard");
        let db = server.db();
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
            println!("\nShutting down ChocoBase gracefully...");
        }
    }

    ExitCode::SUCCESS
}
