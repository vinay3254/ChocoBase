export interface GraphQLResponse<T = any> {
    data: T | null;
    errors: Array<{
        message: string;
    }> | null;
}
export declare class GraphQLClient {
    private url;
    private headers;
    constructor(url: string, headers?: Record<string, string>);
    query<T = any>(query: string, variables?: Record<string, any>): Promise<GraphQLResponse<T>>;
}
