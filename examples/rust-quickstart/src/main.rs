use chocobase::create_client;

fn main() {
    let url = std::env::var("CHOCOBASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());
    let key = std::env::var("CHOCOBASE_ANON_KEY").unwrap_or_else(|_| "anon_key_dev".to_string());

    println!("🚀 Initializing ChocoBase Rust Client at: {url}");
    let client = create_client(&url, &key);

    // 1. User Registration and Authentication
    println!("\n🔐 Registering user...");
    let auth_res = client.auth.sign_up("ferris@rust-lang.org", "ferris-secret-pass");
    println!("Auth response: {auth_res:?}");

    // 2. Database Queries with PostgREST query builder
    println!("\n📊 Building PostgREST table query...");
    let query = client.from("analytics").select("id, metric, value").eq("active", true).limit(5);
    println!("Query parameters: {:?}", query.params);
    let res = query.execute();
    println!("Query result: {res}");

    // 3. Object Storage signed URL creation
    println!("\n📦 Generating Object Storage signed URL...");
    let signed_url = client.storage.from("reports").create_signed_url("annual_audit.pdf", 3600);
    println!("Signed URL: {signed_url}");

    // 4. Serverless Edge Function Invocation
    println!("\n⚡ Invoking Edge Function...");
    let fn_res = client.functions.invoke("calculate-metrics", serde_json::json!({ "framework": "Rust" }));
    println!("Edge function response: {fn_res}");

    // 5. Realtime Channel Setup
    println!("\n📡 Setting up Realtime channel listener...");
    let channel = client.realtime.channel("public:analytics");
    channel.on("INSERT").subscribe();
    println!("Subscribed to channel: {}", channel.topic);

    println!("\n✅ Rust quickstart demonstration complete!");
}
