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
export declare class ChocoBaseClient {
    auth: AuthClient;
    storage: StorageClient;
    functions: FunctionsClient;
    graphql: GraphQLClient;
    realtime: RealtimeClient;
    private url;
    private apikey;
    constructor(url: string, apikey: string, options?: ChocoBaseClientOptions);
    from<T = any>(table: string): QueryBuilder<T>;
    channel(topic: string): import("./realtime.js").RealtimeChannel;
}
export declare function createClient(url: string, apikey: string, options?: ChocoBaseClientOptions): ChocoBaseClient;
