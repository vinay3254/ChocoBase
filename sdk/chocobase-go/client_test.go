package chocobase

import "testing"

func TestClientInitialization(t *testing.T) {
	client := NewClient("http://localhost:8080", "anon-key-123")
	if client.BaseURL != "http://localhost:8080" {
		t.Errorf("expected http://localhost:8080, got %s", client.BaseURL)
	}
	if client.Auth == nil || client.Postgrest == nil || client.Storage == nil || client.Functions == nil || client.Realtime == nil {
		t.Error("expected all subsystems initialized")
	}
}

func TestQueryBuilder(t *testing.T) {
	client := NewClient("http://localhost:8080", "anon-key-123")
	q := client.From("users").Select("id, name").Eq("active", true).Limit(10)
	if q.Params["select"] != "id, name" {
		t.Errorf("expected 'id, name', got %s", q.Params["select"])
	}
	if q.Params["active"] != "eq.true" {
		t.Errorf("expected 'eq.true', got %s", q.Params["active"])
	}
	if q.Params["limit"] != "10" {
		t.Errorf("expected '10', got %s", q.Params["limit"])
	}
}
