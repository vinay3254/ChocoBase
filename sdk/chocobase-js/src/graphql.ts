export interface GraphQLResponse<T = any> {
  data: T | null;
  errors: Array<{ message: string }> | null;
}

export class GraphQLClient {
  private url: string;
  private headers: Record<string, string>;

  constructor(url: string, headers: Record<string, string> = {}) {
    this.url = url.replace(/\/$/, "");
    this.headers = headers;
  }

  async query<T = any>(
    query: string,
    variables?: Record<string, any>
  ): Promise<GraphQLResponse<T>> {
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
    } catch (e: any) {
      return {
        data: null,
        errors: [{ message: e.message || String(e) }],
      };
    }
  }
}
