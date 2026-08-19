use chocobase::create_client;

#[test]
fn test_rust_client_initialization() {
    let client = create_client("http://localhost:8080", "anon-key-123");
    assert_eq!(client.base_url, "http://localhost:8080");
    assert_eq!(client.api_key, "anon-key-123");

    let query = client.from("profiles").select("id, name").eq("role", "member").limit(10);
    assert_eq!(query.params.get("select").unwrap(), "id, name");
    assert_eq!(query.params.get("role").unwrap(), "eq.member");
    assert_eq!(query.params.get("limit").unwrap(), "10");

    let signed_url = client.storage.from("avatars").create_signed_url("user.png", 3600);
    assert!(signed_url.contains("/sign/user.png?expires_in=3600"));
}
