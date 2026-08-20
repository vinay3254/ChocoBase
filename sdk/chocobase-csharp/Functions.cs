using System;
using System.Collections.Generic;
using System.Net.Http;
using System.Text;
using System.Text.Json;
using System.Threading.Tasks;

namespace ChocoBase
{
    public class FunctionsClient
    {
        private readonly string _baseUrl;
        private readonly HttpClient _httpClient;
        private readonly Dictionary<string, string> _headers;

        public FunctionsClient(string baseUrl, HttpClient httpClient, Dictionary<string, string> headers)
        {
            _baseUrl = baseUrl;
            _httpClient = httpClient;
            _headers = headers;
        }

        public async Task<string> InvokeAsync(string functionName, object? body = null)
        {
            var url = $"{_baseUrl}/v1/functions/v1/{functionName}";
            var payload = body != null ? JsonSerializer.Serialize(body) : "{}";
            using var req = new HttpRequestMessage(HttpMethod.Post, url)
            {
                Content = new StringContent(payload, Encoding.UTF8, "application/json")
            };
            foreach (var h in _headers) req.Headers.TryAddWithoutValidation(h.Key, h.Value);

            var resp = await _httpClient.SendAsync(req);
            return await resp.Content.ReadAsStringAsync();
        }
    }
}
