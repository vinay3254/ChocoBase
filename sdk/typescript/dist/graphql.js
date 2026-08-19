"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.GraphQLClient = void 0;
class GraphQLClient {
    url;
    apikey;
    token;
    constructor(url, apikey, token) {
        this.url = url.replace(/\/$/, "");
        this.apikey = apikey;
        this.token = token;
    }
    async query(query, variables) {
        try {
            const res = await fetch(`${this.url}/v1/graphql`, {
                method: "POST",
                headers: {
                    "Content-Type": "application/json",
                    apikey: this.apikey,
                    ...(this.token ? { Authorization: `Bearer ${this.token}` } : {}),
                },
                body: JSON.stringify({ query, variables }),
            });
            const json = await res.json();
            return {
                data: json.data || null,
                errors: json.errors || null,
            };
        }
        catch (e) {
            return {
                data: null,
                errors: [{ message: e.message || String(e) }],
            };
        }
    }
}
exports.GraphQLClient = GraphQLClient;
