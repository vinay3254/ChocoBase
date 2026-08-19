import { createClient } from '@chocobase/chocobase-js';

const CHOCOBASE_URL = process.env.CHOCOBASE_URL || 'http://localhost:8080';
const CHOCOBASE_ANON_KEY = process.env.CHOCOBASE_ANON_KEY || 'anon_key_dev';

async function main() {
  console.log('🚀 Connecting to ChocoBase at:', CHOCOBASE_URL);
  const client = createClient(CHOCOBASE_URL, CHOCOBASE_ANON_KEY);

  // 1. Authenticate user
  console.log('\n🔐 Authenticating user...');
  const { user, session, error: authError } = await client.auth.signUp(
    'developer@example.com',
    'super-secure-password'
  );
  if (authError) {
    console.log('Sign up result / info:', authError.message);
  } else {
    console.log('Signed in as:', user?.email);
  }

  // 2. Query data with PostgREST query builder
  console.log('\n📊 Querying database tables...');
  const { data: todos, error: queryError } = await client
    .from('todos')
    .select('id, title, completed, created_at')
    .eq('completed', false)
    .limit(5);

  if (queryError) {
    console.log('Query result / info:', queryError.message);
  } else {
    console.log('Retrieved todos:', todos);
  }

  // 3. Realtime event channel subscription
  console.log('\n📡 Subscribing to Realtime mutation channels...');
  const channel = client.realtime
    .channel('todos_feed')
    .on('INSERT', (payload) => {
      console.log('🔔 Realtime INSERT received:', payload);
    })
    .subscribe();

  console.log('Subscribed to channel:', channel);

  // 4. Object Storage Operations
  console.log('\n📦 Testing Object Storage...');
  const bucketName = 'avatars';
  const { data: signedUrlData, error: storageError } = await client.storage
    .from(bucketName)
    .createSignedUrl('profile.png', 3600);

  if (storageError) {
    console.log('Storage info:', storageError.message);
  } else {
    console.log('Signed download URL generated:', signedUrlData?.signedUrl);
  }

  // 5. Serverless Edge Functions
  console.log('\n⚡ Invoking Edge Function...');
  const { data: funcResult, error: funcError } = await client.functions.invoke(
    'hello-world',
    { body: { name: 'ChocoBase Developer' } }
  );

  if (funcError) {
    console.log('Function result / info:', funcError.message);
  } else {
    console.log('Function returned:', funcResult);
  }

  console.log('\n✅ Quickstart demonstration script complete!');
}

main().catch(console.error);
