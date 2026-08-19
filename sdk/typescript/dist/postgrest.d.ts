export interface QueryResult<T = any> {
    data: T | null;
    error: Error | null;
    count?: number | null;
}
export declare class QueryBuilder<T = any> {
    private url;
    private apikey;
    private token;
    private table;
    private selectCols;
    private filters;
    private orderCol?;
    private orderAsc;
    private limitCount?;
    private offsetCount?;
    constructor(url: string, apikey: string, token: string | null, table: string);
    select(columns?: string): this;
    eq(column: string, value: any): this;
    neq(column: string, value: any): this;
    gt(column: string, value: any): this;
    lt(column: string, value: any): this;
    order(column: string, options?: {
        ascending?: boolean;
    }): this;
    limit(count: number): this;
    range(from: number, to: number): this;
    private buildQueryString;
    private getHeaders;
    get(): Promise<QueryResult<T[]>>;
    insert(record: Partial<T> | Array<Partial<T>>): Promise<QueryResult<any>>;
    update(values: Partial<T>): Promise<QueryResult<any>>;
    delete(): Promise<QueryResult<any>>;
}
