export class FunctionsClient {
    url;
    headers;
    constructor(url, headers = {}) {
        this.url = url;
        this.headers = headers;
    }
    async invoke(functionName, options = {}) {
        const endpoint = `${this.url}/v1/functions/${functionName}`;
        try {
            const res = await fetch(endpoint, {
                method: 'POST',
                headers: {
                    'Content-Type': 'application/json',
                    ...this.headers,
                    ...options.headers,
                },
                body: options.body !== undefined ? JSON.stringify(options.body) : undefined,
            });
            const json = await res.json().catch(() => null);
            if (!res.ok) {
                return { data: null, error: { message: json?.error || res.statusText } };
            }
            return { data: json, error: null };
        }
        catch (err) {
            return { data: null, error: { message: err.message || 'Function invocation failed' } };
        }
    }
}
