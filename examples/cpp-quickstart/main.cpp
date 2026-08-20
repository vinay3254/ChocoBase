#include <iostream>
#include <chocobase/chocobase.hpp>

int main() {
    std::cout << "🍫 ChocoBase C++ Quickstart (Unreal Engine / Embedded / Desktop)\n";

    auto client = chocobase::create_client("http://localhost:8080", "anon_dev_token");

    // 1. PostgREST URL Query Construction
    auto query = client.from("telemetry")
        .select("id, sensor_id, temp, voltage")
        .eq("sensor_id", "s42")
        .limit(10);

    std::cout << "Constructed Query URL: " << query.build_url() << "\n";

    // 2. Storage Signed Endpoint
    auto storage_endpoint = client.storage.from("firmware").get_signed_url_endpoint("update_v2.bin");
    std::cout << "Storage Signed Endpoint: " << storage_endpoint << "\n";

    // 3. Edge Function Endpoint
    auto fn_endpoint = client.functions.get_invocation_url("device-heartbeat");
    std::cout << "Function Invocation URL: " << fn_endpoint << "\n";

    std::cout << "✅ C++ Quickstart completed successfully!\n";
    return 0;
}
