import { AuthClient } from "./auth.js";
import { FunctionsClient } from "./functions.js";
import { GraphQLClient } from "./graphql.js";
import { QueryBuilder } from "./postgrest.js";
import { RealtimeClient } from "./realtime.js";
import { StorageClient } from "./storage.js";

export * from "./auth.js";
export * from "./functions.js";
export * from "./graphql.js";
export * from "./postgrest.js";
export * from "./realtime.js";
export * from "./storage.js";

export interface ChocoBaseClientOptions {
  auth?: {
    autoRefreshToken?: boolean;
    persistSession?: boolean;
  };
}

export class ChocoBaseClient {
  public auth: AuthClient;
  public storage: StorageClient;
  public functions: FunctionsClient;
  public graphql: GraphQLClient;
  public realtime: RealtimeClient;

  private url: string;
  private apikey: string;

  constructor(url: string, apikey: string, options?: ChocoBaseClientOptions) {
    this.url = url.replace(/\/$/, "");
    this.apikey = apikey;

    this.auth = new AuthClient(this.url, this.apikey);
    this.storage = new StorageClient(this.url, this.apikey, null);
    this.functions = new FunctionsClient(this.url, this.apikey, null);
    this.graphql = new GraphQLClient(this.url, this.apikey, null);
    this.realtime = new RealtimeClient(this.url, this.apikey, null);
  }

  from<T = any>(table: string): QueryBuilder<T> {
    const token = this.auth.session?.access_token || null;
    return new QueryBuilder<T>(this.url, this.apikey, token, table);
  }

  channel(topic: string) {
    const token = this.auth.session?.access_token || null;
    const realtime = new RealtimeClient(this.url, this.apikey, token);
    return realtime.channel(topic);
  }
}

export function createClient(
  url: string,
  apikey: string,
  options?: ChocoBaseClientOptions
): ChocoBaseClient {
  return new ChocoBaseClient(url, apikey, options);
}
