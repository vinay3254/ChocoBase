import test from "node:test";
import assert from "node:assert/strict";
import { ChocoBaseClient, createClient } from "../src/index.js";

test("ChocoBaseClient initializes with all subsystem services", () => {
  const client = createClient("http://localhost:8080", "anon-key");
  assert.ok(client instanceof ChocoBaseClient);
  assert.ok(client.auth);
  assert.ok(client.storage);
  assert.ok(client.functions);
  assert.ok(client.graphql);
  assert.ok(client.realtime);
});

test("PostgREST query builder creates chained query expressions", () => {
  const client = createClient("http://localhost:8080", "anon-key");
  const query = client.from("profiles").select("id, username, created_at").eq("status", "active");
  assert.ok(query);
});
