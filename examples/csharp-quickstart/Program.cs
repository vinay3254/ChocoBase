using System;
using System.Threading.Tasks;
using ChocoBase;

class Program
{
    static async Task Main(string[] args)
    {
        Console.WriteLine("🍫 ChocoBase C# / .NET Quickstart (Unity / MAUI / ASP.NET)");

        var client = ChocoClient.CreateClient("http://localhost:8080", "anon_dev_token");

        // 1. Auth: Sign up
        var authResp = await client.Auth.SignUpAsync("dotnet_dev", "secure_password_123");
        Console.WriteLine($"Auth status: {authResp.User?.Username ?? "anon"} (token: {authResp.AccessToken?.Substring(0, 10)}...)");

        // 2. PostgREST: Query table
        var rows = await client.From("analytics").Select("id, metric, val").Limit(5).ExecuteAsync();
        Console.WriteLine($"Retrieved {rows.Count} rows from analytics table.");

        // 3. Storage: Generate signed URL
        var signedUrl = await client.Storage.From("assets").CreateSignedUrlAsync("model_3d.obj", 3600);
        Console.WriteLine($"Signed URL: {signedUrl}");

        // 4. Edge Functions: Invoke
        var fnResp = await client.Functions.InvokeAsync("game-state", new { player_id = "p123", action = "jump" });
        Console.WriteLine($"Edge Function result: {fnResp}");

        Console.WriteLine("✅ C# Quickstart completed successfully!");
    }
}
