# frozen_string_literal: true

require_relative "../../sdk/chocobase-rb/lib/chocobase"

puts "🍫 ChocoBase Ruby & Rails Quickstart"

client = ChocoBase.create_client("http://localhost:8080", "anon_dev_token")

# 1. Auth: Sign up
auth = client.auth.sign_up("ruby_dev", "secure_password_123")
puts "Auth user: #{auth.dig('user', 'username') || 'anon'}"

# 2. Database: Query PostgREST
items = client.from("inventory").select("id, item, qty").limit(5).execute
puts "Fetched #{items.length} inventory items."

# 3. Storage: Signed URL
signed_url = client.storage.from("backups").create_signed_url("dump.tar.gz", expires_in: 3600)
puts "Signed download URL: #{signed_url}"

# 4. Edge Functions: Invoke
res = client.functions.invoke("process-order", { order_id: 1042, total: 99.50 })
puts "Function output: #{res}"

puts "✅ Ruby Quickstart completed successfully!"
