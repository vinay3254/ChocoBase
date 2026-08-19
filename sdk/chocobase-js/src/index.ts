import { AuthClient } from './auth.js';
import { FunctionsClient } from './functions.js';
import { GraphQLClient } from './graphql.js';
import { PostgrestQueryBuilder } from './query.js';
import { RealtimeClient } from './realtime.js';
import { StorageClient } from './storage.js';
import { ClientOptions } from './types.js';

export class ChocoClient {
  public auth: AuthClient;
  public storage: StorageClient;
  public realtime: RealtimeClient;
  public functions: FunctionsClient;
  public graphql: GraphQLClient;
  private url: string;
  private key?: string;
  private headers: Record<string, string>;

  constructor(url: string, key?: string, options: ClientOptions = {}) {
    this.url = url.replace(/\/+$/, '');
    this.key = key;
    this.headers = {
      ...(key ? { Authorization: `Bearer ${key}` } : {}),
      ...options.headers,
    };

    this.auth = new AuthClient(this.url, this.headers);
    this.storage = new StorageClient(this.url, this.headers);
    this.realtime = new RealtimeClient(this.url, this.key);
    this.functions = new FunctionsClient(this.url, this.headers);
    this.graphql = new GraphQLClient(this.url, this.headers);
  }

  from<T = any>(table: string): PostgrestQueryBuilder<T> {
    const activeToken = this.auth.getSession()?.access_token || this.key;
    const reqHeaders = {
      ...this.headers,
      ...(activeToken ? { Authorization: `Bearer ${activeToken}` } : {}),
    };
    return new PostgrestQueryBuilder<T>(this.url, table, reqHeaders);
  }

  async rpc(funcName: string, params: Record<string, any> = {}): Promise<{ data: any; error: any }> {
    const endpoint = `${this.url}/v1/rpc/${funcName}`;
    const activeToken = this.auth.getSession()?.access_token || this.key;
    try {
      const res = await fetch(endpoint, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          ...this.headers,
          ...(activeToken ? { Authorization: `Bearer ${activeToken}` } : {}),
        },
        body: JSON.stringify(params),
      });

      const data = await res.json().catch(() => null);
      if (!res.ok) {
        return { data: null, error: { message: data?.error || res.statusText } };
      }
      return { data, error: null };
    } catch (err: any) {
      return { data: null, error: { message: err.message || 'RPC invocation failed' } };
    }
  }
}

export function createClient(url: string, key?: string, options?: ClientOptions): ChocoClient {
  return new ChocoClient(url, key, options);
}

export * from './types.js';
export * from './auth.js';
export * from './query.js';
export * from './storage.js';
export * from './realtime.js';
export * from './functions.js';
export * from './graphql.js';
