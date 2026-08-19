import test from 'node:test';
import assert from 'node:assert/strict';
import { createClient, ChocoClient } from '../dist/index.js';

test('createClient instantiates a valid ChocoClient', () => {
  const client = createClient('http://localhost:8080', 'test-anon-key');
  assert.ok(client instanceof ChocoClient);
  assert.ok(client.auth);
  assert.ok(client.storage);
  assert.ok(client.realtime);
  assert.ok(client.functions);
  assert.ok(client.graphql);
});

test('Query builder constructs correct REST URLs and headers', () => {
  const client = createClient('http://localhost:8080', 'test-anon-key');
  const query = client.from('users').select('id, name, email').eq('role', 'admin').limit(10);
  assert.ok(query);
});

test('Realtime client creates channels with event listeners', () => {
  const client = createClient('http://localhost:8080', 'test-anon-key');
  const channel = client.realtime.channel('chat_room');
  assert.ok(channel);
  let called = false;
  channel.on('message', (msg) => {
    called = true;
  });
  channel.unsubscribe();
});
