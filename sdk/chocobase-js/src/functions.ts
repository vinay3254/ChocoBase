export interface FunctionInvokeOptions {
  body?: any;
  headers?: Record<string, string>;
}

export class FunctionsClient {
  private url: string;
  private headers: Record<string, string>;

  constructor(url: string, headers: Record<string, string> = {}) {
    this.url = url;
    this.headers = headers;
  }

  async invoke<T = any>(functionName: string, options: FunctionInvokeOptions = {}): Promise<{ data: T | null; error: any }> {
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
      return { data: json as T, error: null };
    } catch (err: any) {
      return { data: null, error: { message: err.message || 'Function invocation failed' } };
    }
  }
}
