import { PostgrestResponse } from './types.js';
export declare class PostgrestQueryBuilder<T = any> {
    private url;
    private table;
    private headers;
    private params;
    private method;
    private body?;
    constructor(url: string, table: string, headers?: Record<string, string>);
    select(columns?: string): this;
    insert(values: Record<string, any> | Record<string, any>[]): this;
    update(values: Record<string, any>): this;
    delete(): this;
    eq(column: string, value: any): this;
    neq(column: string, value: any): this;
    gt(column: string, value: any): this;
    gte(column: string, value: any): this;
    lt(column: string, value: any): this;
    lte(column: string, value: any): this;
    like(column: string, pattern: string): this;
    ilike(column: string, pattern: string): this;
    is(column: string, value: 'null' | 'not.null' | null): this;
    in(column: string, values: any[]): this;
    order(column: string, options?: {
        ascending?: boolean;
    }): this;
    limit(count: number): this;
    range(from: number, to: number): this;
    execute(): Promise<PostgrestResponse<T>>;
    then<TResult1 = PostgrestResponse<T>, TResult2 = never>(onfulfilled?: ((value: PostgrestResponse<T>) => TResult1 | PromiseLike<TResult1>) | null, onrejected?: ((reason: any) => TResult2 | PromiseLike<TResult2>) | null): Promise<TResult1 | TResult2>;
}
