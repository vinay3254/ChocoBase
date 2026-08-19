export interface FunctionInvokeOptions {
    body?: any;
    headers?: Record<string, string>;
}
export declare class FunctionsClient {
    private url;
    private headers;
    constructor(url: string, headers?: Record<string, string>);
    invoke<T = any>(functionName: string, options?: FunctionInvokeOptions): Promise<{
        data: T | null;
        error: any;
    }>;
}
