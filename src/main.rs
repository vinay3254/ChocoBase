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
        print_usage();
        return ExitCode::SUCCESS;
    }

    let command = args[1].as_str();

    match command {
        "start" | "server" => {
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

            println!("⚡ ChocoBase Server running on http://{addr}");
            println!("📂 Database file: {db_file}");
            println!("🚀 REST Gateway: http://{addr}/rest/v1");
            println!("📊 Health Check: http://{addr}/healthz");
            println!("📈 Metrics Exporter: http://{addr}/metrics");
            println!("🖥️  Studio Dashboard: http://{addr}/");
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
            ExitCode::SUCCESS
        }
        "init" => {
            let dir_name = args.get(2).map(|s| s.as_str()).unwrap_or(".");
            let project_dir = Path::new(dir_name);
            let migrations_dir = project_dir.join("migrations");
            let seed_file = project_dir.join("seed.sql");

            let _ = std::fs::create_dir_all(&migrations_dir);
            if !seed_file.exists() {
                let default_seed = "-- Initial seed schema and data\nCREATE TABLE profiles (\n    id INTEGER PRIMARY KEY,\n    username TEXT NOT NULL,\n    email TEXT NOT NULL\n);\n";
                let _ = std::fs::write(&seed_file, default_seed);
            }

            println!("✨ Initialized ChocoBase project in '{}'", project_dir.display());
            println!("  Created: {}/", migrations_dir.display());
            println!("  Created: {}", seed_file.display());
            ExitCode::SUCCESS
        }
        "dump" => {
            let db_file = args.get(2).map(|s| s.as_str()).unwrap_or("chocobase.db");
            let path = Path::new(db_file);
            let mut db = match Database::open(path) {
                Ok(db) => db,
                Err(e) => {
                    eprintln!("error opening database '{db_file}': {e}");
                    return ExitCode::FAILURE;
                }
            };

            match dbengine::backup::dump_database(&mut db) {
                Ok(dump_sql) => {
                    print!("{dump_sql}");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("error generating database dump: {e}");
                    return ExitCode::FAILURE;
                }
            }
        }
        "restore" => {
            let sql_file = match args.get(2) {
                Some(f) => f,
                None => {
                    eprintln!("Usage: dbengine restore <dump_file.sql> [database_file]");
                    return ExitCode::FAILURE;
                }
            };
            let db_file = args.get(3).map(|s| s.as_str()).unwrap_or("chocobase.db");

            let sql_content = match std::fs::read_to_string(sql_file) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("error reading SQL file '{sql_file}': {e}");
                    return ExitCode::FAILURE;
                }
            };

            let path = Path::new(db_file);
            let mut db = match Database::open(path) {
                Ok(db) => db,
                Err(_) => match Database::create(path) {
                    Ok(db) => db,
                    Err(e) => {
                        eprintln!("error opening database '{db_file}': {e}");
                        return ExitCode::FAILURE;
                    }
                },
            };

            match dbengine::backup::restore_database(&mut db, &sql_content) {
                Ok(count) => {
                    println!("✅ Successfully restored database from '{sql_file}' ({count} statements executed)");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("error restoring database: {e}");
                    return ExitCode::FAILURE;
                }
            }
        }
        "migrate" => {
            let db_file = args.get(2).map(|s| s.as_str()).unwrap_or("chocobase.db");
            let migrations_dir = args.get(3).map(|s| s.as_str()).unwrap_or("migrations");

            let path = Path::new(db_file);
            let mut db = match Database::open(path) {
                Ok(db) => db,
                Err(_) => match Database::create(path) {
                    Ok(db) => db,
                    Err(e) => {
                        eprintln!("error opening database '{db_file}': {e}");
                        return ExitCode::FAILURE;
                    }
                },
            };

            match dbengine::migration::load_from_dir(migrations_dir) {
                Ok(migrations) => {
                    let mut runner = dbengine::MigrationRunner::new(&mut db);
                    match runner.apply_all(&migrations) {
                        Ok(applied) => {
                            println!("✅ Applied {} migration(s) successfully", applied.len());
                            for m in applied {
                                println!("  - v{} ({})", m.version, m.name);
                            }
                            ExitCode::SUCCESS
                        }
                        Err(e) => {
                            eprintln!("error applying migrations: {e}");
                            return ExitCode::FAILURE;
                        }
                    }
                }
                Err(e) => {
                    eprintln!("error reading migrations directory '{migrations_dir}': {e}");
                    return ExitCode::FAILURE;
                }
            }
        }
        "status" => {
            let db_file = args.get(2).map(|s| s.as_str()).unwrap_or("chocobase.db");
            let path = Path::new(db_file);
            let mut db = match Database::open(path) {
                Ok(db) => db,
                Err(e) => {
                    eprintln!("error opening database '{db_file}': {e}");
                    return ExitCode::FAILURE;
                }
            };

            let tables = db.list_tables();
            println!("📊 ChocoBase Database Status");
            println!("  File: {db_file}");
            println!("  Tables ({}):", tables.len());
            for table in &tables {
                let indexes = db.list_indexes(table);
                println!("    - {} (indexes: {})", table, indexes.len());
            }
            ExitCode::SUCCESS
        }
        "gen" | "generate" => {
            let _subcmd = args.get(2).map(|s| s.as_str()).unwrap_or("types");
            let lang = args.get(3).map(|s| s.as_str()).unwrap_or("typescript");
            let db_file = args.get(4).map(|s| s.as_str()).unwrap_or("chocobase.db");

            let path = Path::new(db_file);
            let mut db = match Database::open(path) {
                Ok(db) => db,
                Err(e) => {
                    eprintln!("error opening database '{db_file}': {e}");
                    return ExitCode::FAILURE;
                }
            };

            let tables = db.list_tables();
            if lang == "typescript" || lang == "ts" {
                println!("// Generated by ChocoBase CLI — Schema TypeScript Definitions\n");
                println!("export interface Database {{\n  public: {{\n    Tables: {{");
                for table in &tables {
                    if let Some(schema) = db.table_schema(table) {
                        println!("      {}: {{", table);
                        println!("        Row: {{");
                        for col in &schema.columns {
                            let ts_type = match col.ty {
                                dbengine::types::value::ColumnType::Integer => "number",
                                dbengine::types::value::ColumnType::Float => "number",
                                dbengine::types::value::ColumnType::Text => "string",
                                dbengine::types::value::ColumnType::Boolean => "boolean",
                                dbengine::types::value::ColumnType::Json => "Record<string, unknown>",
                                dbengine::types::value::ColumnType::Vector(_) => "number[]",
                            };
                            let nullable = if !col.not_null && !col.is_primary_key { " | null" } else { "" };
                            println!("          {}: {}{};", col.name, ts_type, nullable);
                        }
                        println!("        }};");
                        println!("        Insert: {{");
                        for col in &schema.columns {
                            let ts_type = match col.ty {
                                dbengine::types::value::ColumnType::Integer => "number",
                                dbengine::types::value::ColumnType::Float => "number",
                                dbengine::types::value::ColumnType::Text => "string",
                                dbengine::types::value::ColumnType::Boolean => "boolean",
                                dbengine::types::value::ColumnType::Json => "Record<string, unknown>",
                                dbengine::types::value::ColumnType::Vector(_) => "number[]",
                            };
                            let opt = if (!col.not_null) || col.is_primary_key || col.name == "id" { "?" } else { "" };
                            println!("          {}{}: {};", col.name, opt, ts_type);
                        }
                        println!("        }};");
                        println!("      }};");
                    }
                }
                println!("    }};\n  }};\n}};\n");
                ExitCode::SUCCESS
            } else if lang == "python" || lang == "py" {
                println!("# Generated by ChocoBase CLI — Schema Python TypedDicts\n");
                println!("from typing import TypedDict, Optional, List, Dict, Any\n");
                for table in &tables {
                    if let Some(schema) = db.table_schema(table) {
                        let capitalized = if !table.is_empty() {
                            format!("{}{}", table[..1].to_uppercase(), &table[1..])
                        } else {
                            "Table".to_string()
                        };
                        let class_name = format!("{capitalized}Row");
                        println!("class {class_name}(TypedDict):");
                        for col in &schema.columns {
                            let py_type = match col.ty {
                                dbengine::types::value::ColumnType::Integer => "int",
                                dbengine::types::value::ColumnType::Float => "float",
                                dbengine::types::value::ColumnType::Text => "str",
                                dbengine::types::value::ColumnType::Boolean => "bool",
                                dbengine::types::value::ColumnType::Json => "Dict[str, Any]",
                                dbengine::types::value::ColumnType::Vector(_) => "List[float]",
                            };
                            if !col.not_null && !col.is_primary_key {
                                println!("    {}: Optional[{}]", col.name, py_type);
                            } else {
                                println!("    {}: {}", col.name, py_type);
                            }
                        }
                        println!();
                    }
                }
                ExitCode::SUCCESS
            } else {
                eprintln!("unsupported language '{lang}' (supported: typescript, python)");
                ExitCode::FAILURE
            }
        }
        "repl" => {
            let db_file = args.get(2).map(|s| s.as_str()).unwrap_or("chocobase.db");
            let path = Path::new(db_file);
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
        other => {
            let path = Path::new(other);
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
                    eprintln!("error opening database '{other}': {e}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}

fn print_usage() {
    println!("ChocoBase - High-Performance Supabase-Compatible Database Platform");
    println!();
    println!("Usage:");
    println!("  dbengine start [bind_addr] [database_file]   Start the full platform (HTTP REST, Auth, Storage, Realtime, Studio)");
    println!("  dbengine init [directory]                    Initialize a new ChocoBase project directory");
    println!("  dbengine migrate [database_file] [dir]       Apply pending schema migrations");
    println!("  dbengine dump [database_file]                Export complete SQL DDL & DML database dump");
    println!("  dbengine restore <dump_file.sql> [db_file]   Restore database from SQL dump file");
    println!("  dbengine status [database_file]              Inspect database tables, indexes, and schema");
    println!("  dbengine repl [database_file]                Launch interactive SQL terminal");
    println!();
    println!("Examples:");
    println!("  dbengine start 127.0.0.1:8080 dev.db");
    println!("  dbengine dump dev.db > backup.sql");
    println!("  dbengine restore backup.sql dev.db");
    println!("  dbengine repl dev.db");
}
