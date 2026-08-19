export class GraphQLClient {
    url;
    headers;
    constructor(url, headers = {}) {
        this.url = url.replace(/\/$/, "");
        this.headers = headers;
    }
    async query(query, variables) {
        try {
            const res = await fetch(`${this.url}/v1/graphql`, {
                method: "POST",
                headers: {
                    "Content-Type": "application/json",
                    ...this.headers,
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
