"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.StorageClient = exports.StorageFileApi = void 0;
class StorageFileApi {
    url;
    apikey;
    token;
    bucketId;
    constructor(url, apikey, token, bucketId) {
        this.url = url.replace(/\/$/, "");
        this.apikey = apikey;
        this.token = token;
        this.bucketId = bucketId;
    }
    getHeaders() {
        const headers = { apikey: this.apikey };
        if (this.token) {
            headers["Authorization"] = `Bearer ${this.token}`;
        }
        return headers;
    }
    async upload(path, fileBody) {
        try {
            const res = await fetch(`${this.url}/v1/storage/v1/object/${this.bucketId}/${path}`, {
                method: "POST",
                headers: {
                    ...this.getHeaders(),
                    "Content-Type": "application/octet-stream",
                },
                body: fileBody,
            });
            const json = await res.json();
            if (!res.ok) {
                return { data: null, error: new Error(json.error || "Upload failed") };
            }
            return { data: json, error: null };
        }
        catch (e) {
            return { data: null, error: e };
        }
    }
    async download(path, options) {
        try {
            const headers = { ...this.getHeaders() };
            if (options?.range) {
                headers["Range"] = options.range;
            }
            const res = await fetch(`${this.url}/v1/storage/v1/object/public/${this.bucketId}/${path}`, {
                method: "GET",
                headers,
            });
            if (!res.ok) {
                const json = await res.json().catch(() => ({}));
                return { data: null, error: new Error(json.error || "Download failed") };
            }
            const blob = await res.blob();
            return { data: blob, error: null };
        }
        catch (e) {
            return { data: null, error: e };
        }
    }
    async list() {
        try {
            const res = await fetch(`${this.url}/v1/storage/v1/object/list/${this.bucketId}`, {
                method: "GET",
                headers: this.getHeaders(),
            });
            const json = await res.json();
            if (!res.ok) {
                return { data: null, error: new Error(json.error || "List objects failed") };
            }
            return { data: json, error: null };
        }
        catch (e) {
            return { data: null, error: e };
        }
    }
    async remove(paths) {
        try {
            const results = [];
            for (const p of paths) {
                const res = await fetch(`${this.url}/v1/storage/v1/object/${this.bucketId}/${p}`, {
                    method: "DELETE",
                    headers: this.getHeaders(),
                });
                results.push(await res.json());
            }
            return { data: results, error: null };
        }
        catch (e) {
            return { data: null, error: e };
        }
    }
    async createSignedUrl(path, expiresIn) {
        try {
            const res = await fetch(`${this.url}/v1/storage/v1/object/sign/${this.bucketId}/${path}`, {
                method: "POST",
                headers: { ...this.getHeaders(), "Content-Type": "application/json" },
                body: JSON.stringify({ expiresIn }),
            });
            const json = await res.json();
            if (!res.ok) {
                return { data: null, error: new Error(json.error || "Failed to create signed URL") };
            }
            return { data: { signedUrl: `${this.url}${json.signedURL}` }, error: null };
        }
        catch (e) {
            return { data: null, error: e };
        }
    }
}
exports.StorageFileApi = StorageFileApi;
class StorageClient {
    url;
    apikey;
    token;
    constructor(url, apikey, token) {
        this.url = url;
        this.apikey = apikey;
        this.token = token;
    }
    from(bucketId) {
        return new StorageFileApi(this.url, this.apikey, this.token, bucketId);
    }
    async createBucket(id, options) {
        try {
            const res = await fetch(`${this.url}/v1/storage/v1/bucket`, {
                method: "POST",
                headers: {
                    apikey: this.apikey,
                    "Content-Type": "application/json",
                    ...(this.token ? { Authorization: `Bearer ${this.token}` } : {}),
                },
                body: JSON.stringify({ id, name: id, public: options?.public ?? false }),
            });
            const json = await res.json();
            if (!res.ok) {
                return { data: null, error: new Error(json.error || "Create bucket failed") };
            }
            return { data: json, error: null };
        }
        catch (e) {
            return { data: null, error: e };
        }
    }
}
exports.StorageClient = StorageClient;
