import Foundation
import ChocoBase

print("🍫 ChocoBase Swift Quickstart (iOS / macOS)")

guard let url = URL(string: "http://localhost:8080") else { fatalError("Invalid URL") }
let client = createClient(url: url, apiKey: "anon_dev_token")

Task {
    do {
        // 1. Auth
        let auth = try await client.auth.signUp(username: "swift_dev", password: "secure_password_123")
        print("Authenticated user: \(auth.user?.username ?? "anon")")
        
        // 2. Query Database
        let rows = try await client.from("devices").select("id, model, os_version").limit(5).execute()
        print("Device rows: \(rows)")
        
        // 3. Storage
        if let signedUrl = try await client.storage.from("backups").createSignedUrl(path: "snapshot.zip") {
            print("Signed download URL: \(signedUrl)")
        }
        
        // 4. Functions
        let res = try await client.functions.invoke("analytics", body: ["event": "app_open"])
        print("Function output: \(res)")
        
        print("✅ Swift Quickstart finished successfully!")
    } catch {
        print("Error: \(error)")
    }
}
