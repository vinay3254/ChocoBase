//! ChocoBase Daemon & CLI Tooling (`chocod`)
//!
//! Provides the primary runtime daemon for TCP wire and HTTP REST protocols,
//! as well as administrative CLI operations for database migrations, dumps,
//! restorations, and user administration.

use std::env;
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;

use dbengine::{
    dump_database, restore_database, Database, HttpServer, Migration, MigrationRunner, Server,
    ServerConfig,
};

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() > 1 && (args[1] == "--help" || args[1] == "-h" || args[1] == "help") {
        print_usage();
        return ExitCode::SUCCESS;
    }

    let subcommand = if args.len() > 1 && !args[1].starts_with('-') {
        args[1].as_str()
    } else {
        "serve"
    };

    match subcommand {
        "serve" => run_serve(&args[1..]).await,
        "dump" => run_dump(&args[2..]),
        "restore" => run_restore(&args[2..]),
        "migrate" => run_migrate(&args[2..]),
        "user" => run_user(&args[2..]),
        _ => {
            // Treat as serve with arguments
            run_serve(&args[1..]).await
        }
    }
}

fn print_usage() {
    println!("ChocoBase CLI & Server Daemon (chocod)");
    println!("Usage: chocod <command> [options]");
    println!();
    println!("Commands:");
    println!("  serve                Start database daemon with TCP wire & HTTP REST (default)");
    println!("  dump                 Export a logical SQL snapshot of the database");
    println!("  restore <FILE>       Transactionally restore database from a SQL snapshot");
    println!("  migrate <DIR>        Apply all pending schema migrations from a directory");
    println!("  user create <U> <P>  Create a user account directly in the database");
    println!();
    println!("Server Options:");
    println!(
        "  -b, --bind <ADDR:PORT>        TCP wire protocol bind address (default: 127.0.0.1:5433)"
    );
    println!(
        "      --http-bind <ADDR:PORT>   HTTP REST & Studio bind address (default: 127.0.0.1:8080)"
    );
    println!("      --no-http                 Disable HTTP gateway and Studio dashboard");
    println!("  -d, --db <PATH>               Path to database file (default: chocobase.db)");
    println!("  -h, --help                    Print help message");
}

async fn run_serve(args: &[String]) -> ExitCode {
    let mut bind_addr: SocketAddr = "127.0.0.1:5433".parse().unwrap();
    let mut http_addr: Option<SocketAddr> = Some("127.0.0.1:8080".parse().unwrap());
    let mut db_path = PathBuf::from("chocobase.db");

    let mut i = 0;
    if !args.is_empty() && args[0] == "serve" {
        i = 1;
    }

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

fn run_dump(args: &[String]) -> ExitCode {
    let mut db_path = PathBuf::from("chocobase.db");
    let mut out_file: Option<PathBuf> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--db" | "-d" => {
                if i + 1 < args.len() {
                    db_path = PathBuf::from(&args[i + 1]);
                    i += 2;
                } else {
                    eprintln!("missing argument for --db");
                    return ExitCode::FAILURE;
                }
            }
            "--out" | "-o" => {
                if i + 1 < args.len() {
                    out_file = Some(PathBuf::from(&args[i + 1]));
                    i += 2;
                } else {
                    eprintln!("missing argument for --out");
                    return ExitCode::FAILURE;
                }
            }
            other => {
                if !other.starts_with('-') {
                    db_path = PathBuf::from(other);
                    i += 1;
                } else {
                    eprintln!("unknown argument '{other}'");
                    return ExitCode::FAILURE;
                }
            }
        }
    }

    let mut db = match Database::open(&db_path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("failed to open database '{}': {e}", db_path.display());
            return ExitCode::FAILURE;
        }
    };

    match dump_database(&mut db) {
        Ok(sql) => {
            if let Some(out_p) = out_file {
                if let Err(e) = fs::write(&out_p, &sql) {
                    eprintln!("failed to write dump file '{}': {e}", out_p.display());
                    return ExitCode::FAILURE;
                }
                println!("Dump written to {}", out_p.display());
            } else {
                print!("{sql}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("database dump failed: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_restore(args: &[String]) -> ExitCode {
    if args.is_empty() {
        eprintln!("Usage: chocod restore <FILE> [--db <PATH>]");
        return ExitCode::FAILURE;
    }

    let dump_path = PathBuf::from(&args[0]);
    let mut db_path = PathBuf::from("chocobase.db");

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--db" | "-d" => {
                if i + 1 < args.len() {
                    db_path = PathBuf::from(&args[i + 1]);
                    i += 2;
                } else {
                    eprintln!("missing argument for --db");
                    return ExitCode::FAILURE;
                }
            }
            _ => i += 1,
        }
    }

    let sql = match fs::read_to_string(&dump_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("failed to read dump file '{}': {e}", dump_path.display());
            return ExitCode::FAILURE;
        }
    };

    let mut db = match Database::open(&db_path).or_else(|_| Database::create(&db_path)) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("failed to open database '{}': {e}", db_path.display());
            return ExitCode::FAILURE;
        }
    };

    match restore_database(&mut db, &sql) {
        Ok(count) => {
            println!("Successfully restored {count} statements from snapshot.");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("restore failed: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_migrate(args: &[String]) -> ExitCode {
    if args.is_empty() {
        eprintln!("Usage: chocod migrate <DIR> [--db <PATH>]");
        return ExitCode::FAILURE;
    }

    let dir_path = PathBuf::from(&args[0]);
    let mut db_path = PathBuf::from("chocobase.db");

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--db" | "-d" => {
                if i + 1 < args.len() {
                    db_path = PathBuf::from(&args[i + 1]);
                    i += 2;
                } else {
                    eprintln!("missing argument for --db");
                    return ExitCode::FAILURE;
                }
            }
            _ => i += 1,
        }
    }

    let mut db = match Database::open(&db_path).or_else(|_| Database::create(&db_path)) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("failed to open database '{}': {e}", db_path.display());
            return ExitCode::FAILURE;
        }
    };

    let mut migrations = Vec::new();
    let entries = match fs::read_dir(&dir_path) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("failed to read directory '{}': {e}", dir_path.display());
            return ExitCode::FAILURE;
        }
    };

    for entry in entries.flatten() {
        let p = entry.path();
        if p.extension().map(|ext| ext == "sql").unwrap_or(false) {
            if let Some(fname) = p.file_stem().and_then(|s| s.to_str()) {
                if let Some((v_str, desc)) = fname.split_once('_') {
                    if let Ok(version) = v_str.parse::<i64>() {
                        if let Ok(sql) = fs::read_to_string(&p) {
                            migrations.push(Migration {
                                version,
                                name: desc.to_string(),
                                sql,
                            });
                        }
                    }
                }
            }
        }
    }

    migrations.sort_by_key(|m| m.version);

    let mut runner = MigrationRunner::new(&mut db);
    match runner.apply_all(&migrations) {
        Ok(applied) => {
            println!("Applied {} migration(s) successfully.", applied.len());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("migration failed: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_user(args: &[String]) -> ExitCode {
    if args.len() < 3 || args[0] != "create" {
        println!(
            "Usage: chocod user create <USERNAME> <PASSWORD> [--role admin|user] [--db <PATH>]"
        );
        return ExitCode::FAILURE;
    }

    let username = &args[1];
    let password = &args[2];
    let mut role = "user";
    let mut db_path = PathBuf::from("chocobase.db");

    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "--role" | "-r" => {
                if i + 1 < args.len() {
                    role = &args[i + 1];
                    i += 2;
                } else {
                    eprintln!("missing argument for --role");
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
            _ => i += 1,
        }
    }

    let mut db = match Database::open(&db_path).or_else(|_| Database::create(&db_path)) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("failed to open database '{}': {e}", db_path.display());
            return ExitCode::FAILURE;
        }
    };

    let sql = format!("CREATE USER {username} WITH PASSWORD '{password}' ROLE '{role}'");
    match db.execute(&sql) {
        Ok(_) => {
            println!("User '{username}' created successfully with role '{role}'.");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("failed to create user: {e}");
            ExitCode::FAILURE
        }
    }
}
