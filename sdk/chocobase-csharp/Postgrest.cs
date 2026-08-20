using System;
using System.Collections.Generic;
using System.Net.Http;
using System.Text;
using System.Text.Json;
using System.Threading.Tasks;

namespace ChocoBase
{
    public class QueryBuilder
    {
        private readonly string _baseUrl;
        private readonly string _table;
        private readonly HttpClient _httpClient;
        private readonly Dictionary<string, string> _headers;
        private readonly Dictionary<string, string> _params = new Dictionary<string, string>();

        public QueryBuilder(string baseUrl, string table, HttpClient httpClient, Dictionary<string, string> headers)
        {
            _baseUrl = baseUrl;
            _table = table;
            _httpClient = httpClient;
            _headers = headers;
        }

        public QueryBuilder Select(string columns = "*")
        {
            _params["select"] = columns;
            return this;
        }

        public QueryBuilder Eq(string column, object value)
        {
            _params[column] = $"eq.{value}";
            return this;
        }

        public QueryBuilder Limit(int count)
        {
            _params["limit"] = count.ToString();
            return this;
        }

        public async Task<List<Dictionary<string, object>>> ExecuteAsync()
        {
            var query = new StringBuilder();
            bool first = true;
            foreach (var kv in _params)
            {
                query.Append(first ? "?" : "&");
                query.Append(Uri.EscapeDataString(kv.Key));
                query.Append("=");
                query.Append(Uri.EscapeDataString(kv.Value));
                first = false;
            }

            var url = $"{_baseUrl}/rest/v1/{_table}{query}";
            using var req = new HttpRequestMessage(HttpMethod.Get, url);
            foreach (var h in _headers) req.Headers.TryAddWithoutValidation(h.Key, h.Value);

            var resp = await _httpClient.SendAsync(req);
            var json = await resp.Content.ReadAsStringAsync();

            try
            {
                return JsonSerializer.Deserialize<List<Dictionary<string, object>>>(json) ?? new List<Dictionary<string, object>>();
            }
            catch
            {
                return new List<Dictionary<string, object>>();
            }
        }
    }
}
