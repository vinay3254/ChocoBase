# @chocobase/chocobase-js

Official isomorphic TypeScript & JavaScript client for **ChocoBase** — the open-source, embedded & distributed Supabase alternative.

## Installation

```bash
npm install @chocobase/chocobase-js
```

## Quick Start

```typescript
import { createClient } from '@chocobase/chocobase-js';

const chocobase = createClient('http://localhost:8080', 'your-anon-or-service-key');

// 1. Querying data (PostgREST filtering)
const { data: products, error } = await chocobase
  .from('products')
  .select('id, title, price')
  .gte('price', 50)
  .order('price', { ascending: false })
  .limit(10);

console.log(products);

// 2. Inserting data
const { data, error: insertError } = await chocobase
  .from('products')
  .insert({ id: 1, title: 'Mechanical Keyboard', price: 99 });

// 3. User Authentication
const { data: authData, error: authError } = await chocobase.auth.signUp({
  username: 'alice',
  password: 'super-secure-password',
});

// 4. Object Storage
await chocobase.storage.createBucket('avatars', { public: true });
await chocobase.storage.from('avatars').upload('user-1.png', fileData);
const { data: url } = chocobase.storage.from('avatars').getPublicUrl('user-1.png');
```

## Supported Features

- **Database**: Full PostgREST query grammar (`eq`, `neq`, `gt`, `gte`, `lt`, `lte`, `like`, `ilike`, `is`, `in`, `order`, `limit`, `range`).
- **Auth**: User signup, token issuance, password verification, session refresh.
- **Storage**: S3-compatible bucket creation, object uploads, downloads, public URLs, deletion.
- **RPC**: Database stored function execution.
