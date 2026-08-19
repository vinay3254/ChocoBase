export interface GraphQLResponse<T = any> {
    data: T | null;
    errors: Array<{
        message: string;
    }> | null;
}
export declare class GraphQLClient {
    private url;
    private apikey;
    private token;
    constructor(url: string, apikey: string, token: string | null);
    query<T = any>(query: string, variables?: Record<string, any>): Promise<GraphQLResponse<T>>;
}
