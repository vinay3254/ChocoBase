using System;
using System.Collections.Generic;
using System.Net.Http;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;
using System.Threading.Tasks;

namespace ChocoBase
{
    public class User
    {
        [JsonPropertyName("id")]
        public long Id { get; set; }

        [JsonPropertyName("username")]
        public string Username { get; set; } = string.Empty;

        [JsonPropertyName("role")]
        public string Role { get; set; } = "user";
    }

    public class AuthResponse
    {
        [JsonPropertyName("access_token")]
        public string? AccessToken { get; set; }

        [JsonPropertyName("refresh_token")]
        public string? RefreshToken { get; set; }

        [JsonPropertyName("user")]
        public User? User { get; set; }

        [JsonPropertyName("error")]
        public string? Error { get; set; }
    }

    public class AuthClient
    {
        private readonly string _baseUrl;
        private readonly HttpClient _httpClient;
        private readonly Dictionary<string, string> _headers;

        public AuthClient(string baseUrl, HttpClient httpClient, Dictionary<string, string> headers)
        {
            _baseUrl = baseUrl;
            _httpClient = httpClient;
            _headers = headers;
        }

        public async Task<AuthResponse> SignUpAsync(string username, string password)
        {
            var url = $"{_baseUrl}/v1/auth/signup";
            var payload = JsonSerializer.Serialize(new { username, password });
            using var req = new HttpRequestMessage(HttpMethod.Post, url)
            {
                Content = new StringContent(payload, Encoding.UTF8, "application/json")
            };
            foreach (var h in _headers) req.Headers.TryAddWithoutValidation(h.Key, h.Value);

            var resp = await _httpClient.SendAsync(req);
            var content = await resp.Content.ReadAsStringAsync();
            return JsonSerializer.Deserialize<AuthResponse>(content) ?? new AuthResponse { Error = "Failed to deserialize response" };
        }

        public async Task<AuthResponse> SignInWithPasswordAsync(string username, string password)
        {
            var url = $"{_baseUrl}/v1/auth/token";
            var payload = JsonSerializer.Serialize(new { username, password });
            using var req = new HttpRequestMessage(HttpMethod.Post, url)
            {
                Content = new StringContent(payload, Encoding.UTF8, "application/json")
            };
            foreach (var h in _headers) req.Headers.TryAddWithoutValidation(h.Key, h.Value);

            var resp = await _httpClient.SendAsync(req);
            var content = await resp.Content.ReadAsStringAsync();
            return JsonSerializer.Deserialize<AuthResponse>(content) ?? new AuthResponse { Error = "Failed to deserialize response" };
        }
    }
}
