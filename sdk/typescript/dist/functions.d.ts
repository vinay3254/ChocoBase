export interface FunctionResponse<T = any> {
    data: T | null;
    error: Error | null;
}
export declare class FunctionsClient {
    private url;
    private apikey;
    private token;
    constructor(url: string, apikey: string, token: string | null);
    invoke<T = any>(functionName: string, options?: {
        body?: any;
        headers?: Record<string, string>;
    }): Promise<FunctionResponse<T>>;
}
