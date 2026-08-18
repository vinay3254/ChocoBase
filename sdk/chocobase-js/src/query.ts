import { PostgrestResponse } from './types.js';

export class PostgrestQueryBuilder<T = any> {
  private url: string;
  private table: string;
  private headers: Record<string, string>;
  private params: URLSearchParams;
  private method: string = 'GET';
  private body?: any;

  constructor(url: string, table: string, headers: Record<string, string> = {}) {
    this.url = url.replace(/\/+$/, '');
    this.table = table;
    this.headers = { ...headers };
    this.params = new URLSearchParams();
  }

  select(columns: string = '*'): this {
    this.method = 'GET';
    this.params.set('select', columns);
    return this;
  }

  insert(values: Record<string, any> | Record<string, any>[]): this {
    this.method = 'POST';
    this.body = values;
    return this;
  }

  update(values: Record<string, any>): this {
    this.method = 'PATCH';
    this.body = values;
    return this;
  }

  delete(): this {
    this.method = 'DELETE';
    return this;
  }

  eq(column: string, value: any): this {
    this.params.set(column, `eq.${value}`);
    return this;
  }

  neq(column: string, value: any): this {
    this.params.set(column, `neq.${value}`);
    return this;
  }

  gt(column: string, value: any): this {
    this.params.set(column, `gt.${value}`);
    return this;
  }

  gte(column: string, value: any): this {
    this.params.set(column, `gte.${value}`);
    return this;
  }

  lt(column: string, value: any): this {
    this.params.set(column, `lt.${value}`);
    return this;
  }

  lte(column: string, value: any): this {
    this.params.set(column, `lte.${value}`);
    return this;
  }

  like(column: string, pattern: string): this {
    this.params.set(column, `like.${pattern}`);
    return this;
  }

  ilike(column: string, pattern: string): this {
    this.params.set(column, `ilike.${pattern}`);
    return this;
  }

  is(column: string, value: 'null' | 'not.null' | null): this {
    if (value === null || value === 'null') {
      this.params.set(column, 'is.null');
    } else {
      this.params.set(column, 'is.not.null');
    }
    return this;
  }

  in(column: string, values: any[]): this {
    this.params.set(column, `in.(${values.join(',')})`);
    return this;
  }

  order(column: string, options: { ascending?: boolean } = { ascending: true }): this {
    const dir = options.ascending ? 'asc' : 'desc';
    this.params.set('order', `${column}.${dir}`);
    return this;
  }

  limit(count: number): this {
    this.params.set('limit', count.toString());
    return this;
  }

  range(from: number, to: number): this {
    this.params.set('offset', from.toString());
    this.params.set('limit', (to - from + 1).toString());
    return this;
  }

  async execute(): Promise<PostgrestResponse<T>> {
    const qs = this.params.toString();
    const endpoint = `${this.url}/v1/rest/${this.table}${qs ? `?${qs}` : ''}`;

    const init: RequestInit = {
      method: this.method,
      headers: {
        'Content-Type': 'application/json',
        ...this.headers,
      },
    };

    if (this.body && this.method !== 'GET') {
      init.body = JSON.stringify(this.body);
    }

    try {
      const res = await fetch(endpoint, init);
      const data = await res.json().catch(() => null);

      if (!res.ok) {
        return {
          data: null,
          error: {
            message: data?.error || res.statusText,
            code: data?.code,
          },
          status: res.status,
          statusText: res.statusText,
        };
      }

      return {
        data: data as T,
        error: null,
        status: res.status,
        statusText: res.statusText,
      };
    } catch (err: any) {
      return {
        data: null,
        error: {
          message: err.message || 'Network request failed',
        },
        status: 0,
        statusText: 'Network Error',
      };
    }
  }

  // Promise-like then for direct await support e.g. await client.from('t').select()
  then<TResult1 = PostgrestResponse<T>, TResult2 = never>(
    onfulfilled?: ((value: PostgrestResponse<T>) => TResult1 | PromiseLike<TResult1>) | null,
    onrejected?: ((reason: any) => TResult2 | PromiseLike<TResult2>) | null
  ): Promise<TResult1 | TResult2> {
    return this.execute().then(onfulfilled, onrejected);
  }
}
