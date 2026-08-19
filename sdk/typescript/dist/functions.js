"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.FunctionsClient = void 0;
class FunctionsClient {
    url;
    apikey;
    token;
    constructor(url, apikey, token) {
        this.url = url.replace(/\/$/, "");
        this.apikey = apikey;
        this.token = token;
    }
    async invoke(functionName, options) {
        try {
            const res = await fetch(`${this.url}/v1/functions/v1/${functionName}`, {
                method: "POST",
                headers: {
                    "Content-Type": "application/json",
                    apikey: this.apikey,
                    ...(this.token ? { Authorization: `Bearer ${this.token}` } : {}),
                    ...(options?.headers || {}),
                },
                body: options?.body ? JSON.stringify(options.body) : "{}",
            });
            const json = await res.json();
            if (!res.ok) {
                return { data: null, error: new Error(json.error || "Function execution failed") };
            }
            return { data: json, error: null };
        }
        catch (e) {
            return { data: null, error: e };
        }
    }
}
exports.FunctionsClient = FunctionsClient;
