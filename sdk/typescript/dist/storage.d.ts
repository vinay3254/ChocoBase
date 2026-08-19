export interface StorageResponse<T = any> {
    data: T | null;
    error: Error | null;
}
export declare class StorageFileApi {
    private url;
    private apikey;
    private token;
    private bucketId;
    constructor(url: string, apikey: string, token: string | null, bucketId: string);
    private getHeaders;
    upload(path: string, fileBody: string | Uint8Array): Promise<StorageResponse<{
        Key: string;
        etag: string;
    }>>;
    download(path: string, options?: {
        range?: string;
    }): Promise<StorageResponse<Blob>>;
    list(): Promise<StorageResponse<Array<{
        name: string;
        id: string;
        metadata: any;
    }>>>;
    remove(paths: string[]): Promise<StorageResponse<any>>;
    createSignedUrl(path: string, expiresIn: number): Promise<StorageResponse<{
        signedUrl: string;
    }>>;
}
export declare class StorageClient {
    private url;
    private apikey;
    private token;
    constructor(url: string, apikey: string, token: string | null);
    from(bucketId: string): StorageFileApi;
    createBucket(id: string, options?: {
        public?: boolean;
    }): Promise<StorageResponse<any>>;
}
