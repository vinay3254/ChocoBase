using System;
using System.Collections.Generic;
using System.Net.Http;

namespace ChocoBase
{
    /// <summary>
    /// Official C# and .NET client for ChocoBase.
    /// Compatible with Unity, .NET MAUI, ASP.NET Core, and Blazor.
    /// </summary>
    public class ChocoClient
    {
        public string Url { get; }
        public string ApiKey { get; }
        public HttpClient HttpClient { get; }
        public Dictionary<string, string> Headers { get; }

        public AuthClient Auth { get; }
        public StorageClient Storage { get; }
        public FunctionsClient Functions { get; }

        public ChocoClient(string url, string apiKey, HttpClient? customHttpClient = null, Dictionary<string, string>? customHeaders = null)
        {
            Url = url.TrimEnd('/');
            ApiKey = apiKey;
            HttpClient = customHttpClient ?? new HttpClient();

            Headers = new Dictionary<string, string>
            {
                { "apikey", apiKey },
                { "Authorization", $"Bearer {apiKey}" }
            };

            if (customHeaders != null)
            {
                foreach (var kv in customHeaders)
                {
                    Headers[kv.Key] = kv.Value;
                }
            }

            Auth = new AuthClient(Url, HttpClient, Headers);
            Storage = new StorageClient(Url, HttpClient, Headers);
            Functions = new FunctionsClient(Url, HttpClient, Headers);
        }

        public QueryBuilder From(string table)
        {
            return new QueryBuilder(Url, table, HttpClient, Headers);
        }

        public static ChocoClient CreateClient(string url, string apiKey, Dictionary<string, string>? customHeaders = null)
        {
            return new ChocoClient(url, apiKey, null, customHeaders);
        }
    }
}
