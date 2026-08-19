export class StorageFileApi {
    url;
    bucketId;
    headers;
    constructor(url, bucketId, headers = {}) {
        this.url = url.replace(/\/+$/, '');
        this.bucketId = bucketId;
        this.headers = { ...headers };
    }
    async upload(path, fileBody) {
        const cleanPath = path.replace(/^\/+/, '');
        const endpoint = `${this.url}/v1/storage/v1/object/${this.bucketId}/${cleanPath}`;
        try {
            const res = await fetch(endpoint, {
                method: 'POST',
                headers: {
                    'Content-Type': 'application/octet-stream',
                    ...this.headers,
                },
                body: fileBody,
            });
            const data = await res.json().catch(() => null);
            if (!res.ok) {
                return { data: null, error: { message: data?.error || res.statusText } };
            }
            return { data: data, error: null };
        }
        catch (err) {
            return { data: null, error: { message: err.message || 'Upload failed' } };
        }
    }
    async download(path) {
        const cleanPath = path.replace(/^\/+/, '');
        const endpoint = `${this.url}/v1/storage/v1/object/${this.bucketId}/${cleanPath}`;
        try {
            const res = await fetch(endpoint, {
                method: 'GET',
                headers: { ...this.headers },
            });
            if (!res.ok) {
                const json = await res.json().catch(() => null);
                return { data: null, error: { message: json?.error || res.statusText } };
            }
            const blob = await res.blob();
            return { data: blob, error: null };
        }
        catch (err) {
            return { data: null, error: { message: err.message || 'Download failed' } };
        }
    }
    async remove(paths) {
        try {
            for (const path of paths) {
                const cleanPath = path.replace(/^\/+/, '');
                const endpoint = `${this.url}/v1/storage/v1/object/${this.bucketId}/${cleanPath}`;
                await fetch(endpoint, {
                    method: 'DELETE',
                    headers: { ...this.headers },
                });
            }
            return { data: paths.map(() => ({ message: 'deleted' })), error: null };
        }
        catch (err) {
            return { data: null, error: { message: err.message || 'Delete failed' } };
        }
    }
    getPublicUrl(path) {
        const cleanPath = path.replace(/^\/+/, '');
        return {
            data: {
                publicUrl: `${this.url}/v1/storage/v1/object/public/${this.bucketId}/${cleanPath}`,
            },
        };
    }
}
export class StorageClient {
    url;
    headers;
    constructor(url, headers = {}) {
        this.url = url.replace(/\/+$/, '');
        this.headers = { ...headers };
    }
    from(bucketId) {
        return new StorageFileApi(this.url, bucketId, this.headers);
    }
    async listBuckets() {
        const endpoint = `${this.url}/v1/storage/v1/bucket`;
        try {
            const res = await fetch(endpoint, { headers: { ...this.headers } });
            const data = await res.json().catch(() => null);
            if (!res.ok) {
                return { data: null, error: { message: data?.error || res.statusText } };
            }
            return { data: data, error: null };
        }
        catch (err) {
            return { data: null, error: { message: err.message || 'Failed to list buckets' } };
        }
    }
    async createBucket(id, options = {}) {
        const endpoint = `${this.url}/v1/storage/v1/bucket`;
        try {
            const res = await fetch(endpoint, {
                method: 'POST',
                headers: {
                    'Content-Type': 'application/json',
                    ...this.headers,
                },
                body: JSON.stringify({ id, name: id, public: options.public ?? false }),
            });
            const data = await res.json().catch(() => null);
            if (!res.ok) {
                return { data: null, error: { message: data?.error || res.statusText } };
            }
            return { data, error: null };
        }
        catch (err) {
            return { data: null, error: { message: err.message || 'Failed to create bucket' } };
        }
    }
    async deleteBucket(id) {
        const endpoint = `${this.url}/v1/storage/v1/bucket/${id}`;
        try {
            const res = await fetch(endpoint, {
                method: 'DELETE',
                headers: { ...this.headers },
            });
            const data = await res.json().catch(() => null);
            if (!res.ok) {
                return { data: null, error: { message: data?.error || res.statusText } };
            }
            return { data, error: null };
        }
        catch (err) {
            return { data: null, error: { message: err.message || 'Failed to delete bucket' } };
        }
    }
}
