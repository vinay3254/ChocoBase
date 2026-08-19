from chocobase import create_client, ChocoClient

def test_client_initialization():
    client = create_client("http://localhost:8080", "anon-key-123")
    assert isinstance(client, ChocoClient)
    assert client.auth is not None
    assert client.postgrest is not None
    assert client.storage is not None
    assert client.functions is not None
    assert client.realtime is not None

def test_query_builder():
    client = create_client("http://localhost:8080", "anon-key-123")
    query = client.from_("profiles").select("id, username").eq("active", True).limit(10)
    assert query.params["select"] == "id, username"
    assert query.params["active"] == "eq.True"
    assert query.params["limit"] == "10"

def test_realtime_channel():
    client = create_client("http://localhost:8080", "anon-key-123")
    ch = client.realtime.channel("room_1").on("INSERT", lambda p: None).subscribe()
    assert ch.topic == "room_1"
    assert len(ch.listeners) == 1
