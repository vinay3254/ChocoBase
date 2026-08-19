export interface FunctionResponse<T = any> {
  data: T | null;
  error: Error | null;
}

export class FunctionsClient {
  private url: string;
  private apikey: string;
  private token: string | null;

  constructor(url: string, apikey: string, token: string | null) {
    this.url = url.replace(/\/$/, "");
    this.apikey = apikey;
    this.token = token;
  }

  async invoke<T = any>(
    functionName: string,
    options?: { body?: any; headers?: Record<string, string> }
  ): Promise<FunctionResponse<T>> {
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
    } catch (e: any) {
      return { data: null, error: e };
    }
  }
}
