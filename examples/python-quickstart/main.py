import os
import sys

# Ensure local SDK is in path when running directly
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "../../sdk/chocobase-py/src")))

from chocobase import create_client

CHOCOBASE_URL = os.getenv("CHOCOBASE_URL", "http://localhost:8080")
CHOCOBASE_ANON_KEY = os.getenv("CHOCOBASE_ANON_KEY", "anon_key_dev")

def main():
    print(f"🚀 Initializing ChocoBase Python Client at: {CHOCOBASE_URL}")
    client = create_client(CHOCOBASE_URL, CHOCOBASE_ANON_KEY)

    # 1. User Registration and Authentication
    print("\n🔐 Registering user...")
    auth_resp = client.auth.sign_up("developer@python.org", "python-secure-pass")
    print("User registration info:", auth_resp)

    # 2. Database Queries with PostgREST query builder
    print("\n📊 Querying database tables...")
    query = client.table("profiles").select("id, username, created_at").eq("role", "member").limit(5)
    print("Prepared query params:", query.params)
    res = query.execute()
    print("Query result:", res)

    # 3. Object Storage signed URL creation
    print("\n📦 Object Storage signed URL creation...")
    signed_url_res = client.storage.from_("avatars").create_signed_url("profile.jpg", expires_in=3600)
    print("Generated signed URL:", signed_url_res)

    # 4. Serverless Edge Function Invocation
    print("\n⚡ Invoking Edge Function...")
    fn_res = client.functions.invoke("hello-world", {"body": {"framework": "Python"}})
    print("Edge function response:", fn_res)

    # 5. Realtime Channel Subscription
    print("\n📡 Setting up Realtime channel listener...")
    channel = client.realtime.channel("public:posts").on("INSERT", lambda p: print("Realtime event:", p)).subscribe()
    print(f"Subscribed to topic: {channel.topic}")

    print("\n✅ Python quickstart demonstration complete!")

if __name__ == "__main__":
    main()
