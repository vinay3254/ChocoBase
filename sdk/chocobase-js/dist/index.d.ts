import { AuthClient } from './auth.js';
import { FunctionsClient } from './functions.js';
import { GraphQLClient } from './graphql.js';
import { PostgrestQueryBuilder } from './query.js';
import { RealtimeClient } from './realtime.js';
import { StorageClient } from './storage.js';
import { ClientOptions } from './types.js';
export declare class ChocoClient {
    auth: AuthClient;
    storage: StorageClient;
    realtime: RealtimeClient;
    functions: FunctionsClient;
    graphql: GraphQLClient;
    private url;
    private key?;
    private headers;
    constructor(url: string, key?: string, options?: ClientOptions);
    from<T = any>(table: string): PostgrestQueryBuilder<T>;
    rpc(funcName: string, params?: Record<string, any>): Promise<{
        data: any;
        error: any;
    }>;
}
export declare function createClient(url: string, key?: string, options?: ClientOptions): ChocoClient;
export * from './types.js';
export * from './auth.js';
export * from './query.js';
export * from './storage.js';
export * from './realtime.js';
export * from './functions.js';
export * from './graphql.js';
