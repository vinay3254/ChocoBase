# ChocoBase 🍫

> **The Modern, Open-Source, Embedded & Distributed Supabase Alternative.**  
> Built from the ground up in 100% pure, safe Rust.

ChocoBase is an all-in-one database and backend platform providing ACID-compliant storage, PostgreSQL wire compatibility, a PostgREST-compliant HTTP gateway, Argon2id authentication with Row-Level Security (RLS), S3-compatible object storage with signed download URLs, realtime database changefeeds & presence channels, and serverless edge functions.

---

## ⚡ Key Platform Capabilities

- 🧱 **ACID Relational Storage Engine**: Page-based storage (4KB pages), B+Tree table/index storage, rollback journaling with crash recovery, and multi-reader/single-writer concurrency locking.
- 🐘 **PostgreSQL Wire Compatibility**: Connect seamlessly using official PostgreSQL drivers and ORMs (`psql`, `tokio-postgres`, `node-postgres`, `psycopg2`, `sqlx`, etc.) on port `5433`.
- 🌐 **PostgREST-Compliant HTTP REST Gateway**: Auto-generated REST CRUD APIs with rich filtering (`eq`, `neq`, `gt`, `gte`, `lt`, `lte`, `like`, `ilike`, `is.null`, `in`), pagination (`limit`, `offset`), and column projection.
- 🔐 **Production Auth & Row-Level Security (RLS)**: Cryptographically hardened Argon2id password hashing, HMAC-SHA256 JWT token issuance and rotation, fail-closed anonymous access models, and SQL `CREATE POLICY` / `ALTER TABLE ENABLE ROW LEVEL SECURITY`.
- 📦 **S3-Compatible Object Storage**: Bucket creation, object uploads/downloads, public/private access rules, and time-limited HMAC-SHA256 signed URLs with expiration enforcement.
- ⚡ **Realtime Engine**: Push-based database mutation changefeeds (`INSERT`, `UPDATE`, `DELETE`), topic broadcast channels, and collaborative presence synchronization.
- ⚡ **Serverless Edge Functions**: Embedded runtime supporting function deployment (`POST /v1/functions/v1/deploy`) and execution (`POST /v1/functions/v1/{name}`) with isolated memory and timeout guards.
- 💾 **Point-in-Time Backup & Restoration**: Logical database SQL dumping (`dump_database` / `GET /v1/admin/dump`) and atomic transactional restoration (`restore_database` / `POST /v1/admin/restore`).
- 🖥️ **Embedded Studio Dashboard**: Out-of-the-box browser web console for schema inspection, query execution, table browsing, and system health metrics on `http://localhost:8080/dashboard`.
- 📦 **First-Party TypeScript/JavaScript SDK**: `@chocobase/chocobase-js` providing familiar `.from()`, `.auth`, `.storage`, and `.rpc` client APIs.

---

## 🚀 Quick Start

### 1. Build and Start the Daemon

```bash
cargo build --release
./target/release/chocod serve --db chocobase.db
```

This launches:
- **PostgreSQL TCP Wire Server**: `127.0.0.1:5433`
- **HTTP REST Gateway & Studio Dashboard**: `http://127.0.0.1:8080/dashboard`

---

## 💻 CLI Commands (`chocod`)

| Command | Description |
|---|---|
| `chocod serve [--bind 127.0.0.1:5433] [--http-bind 127.0.0.1:8080] [-d db.db]` | Start the multi-protocol server daemon (default) |
| `chocod dump [-d db.db] [-o backup.sql]` | Export full database schema and row data as a portable SQL snapshot |
| `chocod restore <backup.sql> [-d db.db]` | Transactionally restore database from a SQL snapshot |
| `chocod migrate <migrations_dir> [-d db.db]` | Apply pending `.sql` schema migrations in version order |
| `chocod user create <username> <password> [--role admin\|user] [-d db.db]` | Directly provision users in the database |

---

## 🔌 Connecting with Clients & Drivers

### Connecting with `psql`
```bash
psql -h 127.0.0.1 -p 5433 -U postgres -d chocobase
```

### Querying with TypeScript / JavaScript SDK (`@chocobase/chocobase-js`)
```typescript
import { createClient } from '@chocobase/chocobase-js';

const chocobase = createClient('http://localhost:8080', 'your-anon-or-service-key');

// 1. PostgREST filtering & pagination
const { data, error } = await chocobase
  .from('products')
  .select('id, name, price')
  .gte('price', 50)
  .order('price', { ascending: false })
  .limit(10);

// 2. Auth Sign-Up
const { data: user } = await chocobase.auth.signUp({
  username: 'alice',
  password: 'secure-password',
});

// 3. Object Storage
await chocobase.storage.createBucket('avatars', { public: true });
await chocobase.storage.from('avatars').upload('avatar.png', fileBuffer);
```

---

## 🧪 Testing and Verification

Run the entire 167-test automated verification suite:

```bash
cargo test
```

Check code formatting:
```bash
cargo fmt -- --check
```

---

## 📄 Architecture Overview

```text
HTTP / Postgres Wire Listeners (Port 8080 / 5433)
                │
┌───────────────▼──────────────────────────┐
│          SharedDatabase                  │
│  - LockManager (Shared/Exclusive locks)  │
│  - ExecutionContext (Admin/Auth/Anon)    │
│  - Realtime & Broadcast Channel Manager  │
│  - Serverless Function Registry          │
└───────────────┬──────────────────────────┘
                │
┌───────────────▼──────────────────────────┐
│             Database                     │
│  - SQL Parser & Planner (Rules/Seeks)    │
│  - Volcano-Model Execution Engine        │
│  - Row-Level Security Policy Engine      │
│  - System Catalog (_users, _migrations)  │
│  - B+Tree Table & Index Storage          │
│  - Pager (4KB pages, Rollback Journal)   │
└──────────────────────────────────────────┘
```

---

## 📜 License

MIT
