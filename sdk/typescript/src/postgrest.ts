export interface QueryResult<T = any> {
  data: T | null;
  error: Error | null;
  count?: number | null;
}

export class QueryBuilder<T = any> {
  private url: string;
  private apikey: string;
  private token: string | null;
  private table: string;
  private selectCols: string = "*";
  private filters: Array<{ col: string; op: string; val: any }> = [];
  private orderCol?: string;
  private orderAsc: boolean = true;
  private limitCount?: number;
  private offsetCount?: number;

  constructor(url: string, apikey: string, token: string | null, table: string) {
    this.url = url.replace(/\/$/, "");
    this.apikey = apikey;
    this.token = token;
    this.table = table;
  }

  select(columns: string = "*"): this {
    this.selectCols = columns;
    return this;
  }

  eq(column: string, value: any): this {
    this.filters.push({ col: column, op: "eq", val: value });
    return this;
  }

  neq(column: string, value: any): this {
    this.filters.push({ col: column, op: "neq", val: value });
    return this;
  }

  gt(column: string, value: any): this {
    this.filters.push({ col: column, op: "gt", val: value });
    return this;
  }

  lt(column: string, value: any): this {
    this.filters.push({ col: column, op: "lt", val: value });
    return this;
  }

  order(column: string, options?: { ascending?: boolean }): this {
    this.orderCol = column;
    this.orderAsc = options?.ascending ?? true;
    return this;
  }

  limit(count: number): this {
    this.limitCount = count;
    return this;
  }

  range(from: number, to: number): this {
    this.offsetCount = from;
    this.limitCount = to - from + 1;
    return this;
  }

  private buildQueryString(): string {
    const params: string[] = [];
    if (this.selectCols && this.selectCols !== "*") {
      params.push(`select=${encodeURIComponent(this.selectCols)}`);
    }
    for (const f of this.filters) {
      params.push(`${encodeURIComponent(f.col)}=${f.op}.${encodeURIComponent(String(f.val))}`);
    }
    if (this.orderCol) {
      params.push(`order=${encodeURIComponent(this.orderCol)}.${this.orderAsc ? "asc" : "desc"}`);
    }
    if (this.limitCount !== undefined) {
      params.push(`limit=${this.limitCount}`);
    }
    if (this.offsetCount !== undefined) {
      params.push(`offset=${this.offsetCount}`);
    }
    return params.length > 0 ? `?${params.join("&")}` : "";
  }

  private getHeaders(): Record<string, string> {
    const headers: Record<string, string> = {
      "Content-Type": "application/json",
      apikey: this.apikey,
    };
    if (this.token) {
      headers["Authorization"] = `Bearer ${this.token}`;
    }
    return headers;
  }

  async get(): Promise<QueryResult<T[]>> {
    try {
      const qs = this.buildQueryString();
      const res = await fetch(`${this.url}/v1/rest/${this.table}${qs}`, {
        method: "GET",
        headers: this.getHeaders(),
      });
      const json = await res.json();
      if (!res.ok) {
        return { data: null, error: new Error(json.error || "Query failed") };
      }
      return { data: json, error: null };
    } catch (e: any) {
      return { data: null, error: e };
    }
  }

  async insert(record: Partial<T> | Array<Partial<T>>): Promise<QueryResult<any>> {
    try {
      const res = await fetch(`${this.url}/v1/rest/${this.table}`, {
        method: "POST",
        headers: this.getHeaders(),
        body: JSON.stringify(record),
      });
      const json = await res.json();
      if (!res.ok) {
        return { data: null, error: new Error(json.error || "Insert failed") };
      }
      return { data: json, error: null };
    } catch (e: any) {
      return { data: null, error: e };
    }
  }

  async update(values: Partial<T>): Promise<QueryResult<any>> {
    try {
      const qs = this.buildQueryString();
      const res = await fetch(`${this.url}/v1/rest/${this.table}${qs}`, {
        method: "PATCH",
        headers: this.getHeaders(),
        body: JSON.stringify(values),
      });
      const json = await res.json();
      if (!res.ok) {
        return { data: null, error: new Error(json.error || "Update failed") };
      }
      return { data: json, error: null };
    } catch (e: any) {
      return { data: null, error: e };
    }
  }

  async delete(): Promise<QueryResult<any>> {
    try {
      const qs = this.buildQueryString();
      const res = await fetch(`${this.url}/v1/rest/${this.table}${qs}`, {
        method: "DELETE",
        headers: this.getHeaders(),
      });
      const json = await res.json();
      if (!res.ok) {
        return { data: null, error: new Error(json.error || "Delete failed") };
      }
      return { data: json, error: null };
    } catch (e: any) {
      return { data: null, error: e };
    }
  }
}
