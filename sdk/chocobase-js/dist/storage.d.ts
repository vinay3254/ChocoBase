import { StorageBucket, StorageFile } from './types.js';
export declare class StorageFileApi {
    private url;
    private bucketId;
    private headers;
    constructor(url: string, bucketId: string, headers?: Record<string, string>);
    upload(path: string, fileBody: string | ArrayBuffer | Blob): Promise<{
        data: StorageFile | null;
        error: any;
    }>;
    download(path: string): Promise<{
        data: Blob | null;
        error: any;
    }>;
    remove(paths: string[]): Promise<{
        data: {
            message: string;
        }[] | null;
        error: any;
    }>;
    getPublicUrl(path: string): {
        data: {
            publicUrl: string;
        };
    };
}
export declare class StorageClient {
    private url;
    private headers;
    constructor(url: string, headers?: Record<string, string>);
    from(bucketId: string): StorageFileApi;
    listBuckets(): Promise<{
        data: StorageBucket[] | null;
        error: any;
    }>;
    createBucket(id: string, options?: {
        public?: boolean;
    }): Promise<{
        data: {
            name: string;
        } | null;
        error: any;
    }>;
    deleteBucket(id: string): Promise<{
        data: {
            message: string;
        } | null;
        error: any;
    }>;
}
