using System;
using System.Collections.Generic;
using System.Net.Http;
using System.Text;
using System.Text.Json;
using System.Threading.Tasks;

namespace ChocoBase
{
    public class StorageBucket
    {
        private readonly string _baseUrl;
        private readonly string _bucket;
        private readonly HttpClient _httpClient;
        private readonly Dictionary<string, string> _headers;

        public StorageBucket(string baseUrl, string bucket, HttpClient httpClient, Dictionary<string, string> headers)
        {
            _baseUrl = baseUrl;
            _bucket = bucket;
            _httpClient = httpClient;
            _headers = headers;
        }

        public async Task<string?> CreateSignedUrlAsync(string path, int expiresInSeconds = 3600)
        {
            var url = $"{_baseUrl}/v1/storage/v1/object/sign/{_bucket}/{path}";
            var payload = JsonSerializer.Serialize(new { expires_in = expiresInSeconds });
            using var req = new HttpRequestMessage(HttpMethod.Post, url)
            {
                Content = new StringContent(payload, Encoding.UTF8, "application/json")
            };
            foreach (var h in _headers) req.Headers.TryAddWithoutValidation(h.Key, h.Value);

            var resp = await _httpClient.SendAsync(req);
            if (resp.IsSuccessStatusCode)
            {
                var content = await resp.Content.ReadAsStringAsync();
                using var doc = JsonDocument.Parse(content);
                if (doc.RootElement.TryGetProperty("signed_url", out var su))
                {
                    return $"{_baseUrl}{su.GetString()}";
                }
            }
            return null;
        }
    }

    public class StorageClient
    {
        private readonly string _baseUrl;
        private readonly HttpClient _httpClient;
        private readonly Dictionary<string, string> _headers;

        public StorageClient(string baseUrl, HttpClient httpClient, Dictionary<string, string> headers)
        {
            _baseUrl = baseUrl;
            _httpClient = httpClient;
            _headers = headers;
        }

        public StorageBucket From(string bucket)
        {
            return new StorageBucket(_baseUrl, bucket, _httpClient, _headers);
        }
    }
}
