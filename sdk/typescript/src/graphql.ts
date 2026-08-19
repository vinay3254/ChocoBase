export interface GraphQLResponse<T = any> {
  data: T | null;
  errors: Array<{ message: string }> | null;
}

export class GraphQLClient {
  private url: string;
  private apikey: string;
  private token: string | null;

  constructor(url: string, apikey: string, token: string | null) {
    this.url = url.replace(/\/$/, "");
    this.apikey = apikey;
    this.token = token;
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
    } catch (e: any) {
      return {
        data: null,
        errors: [{ message: e.message || String(e) }],
      };
    }
  }
}
