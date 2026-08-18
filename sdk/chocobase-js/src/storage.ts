import { StorageBucket, StorageFile } from './types.js';

export class StorageFileApi {
  private url: string;
  private bucketId: string;
  private headers: Record<string, string>;

  constructor(url: string, bucketId: string, headers: Record<string, string> = {}) {
    this.url = url.replace(/\/+$/, '');
    this.bucketId = bucketId;
    this.headers = { ...headers };
  }

  async upload(path: string, fileBody: string | ArrayBuffer | Blob): Promise<{ data: StorageFile | null; error: any }> {
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

      return { data: data as StorageFile, error: null };
    } catch (err: any) {
      return { data: null, error: { message: err.message || 'Upload failed' } };
    }
  }

  async download(path: string): Promise<{ data: Blob | null; error: any }> {
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
    } catch (err: any) {
      return { data: null, error: { message: err.message || 'Download failed' } };
    }
  }

  async remove(paths: string[]): Promise<{ data: { message: string }[] | null; error: any }> {
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
    } catch (err: any) {
      return { data: null, error: { message: err.message || 'Delete failed' } };
    }
  }

  getPublicUrl(path: string): { data: { publicUrl: string } } {
    const cleanPath = path.replace(/^\/+/, '');
    return {
      data: {
        publicUrl: `${this.url}/v1/storage/v1/object/public/${this.bucketId}/${cleanPath}`,
      },
    };
  }
}

export class StorageClient {
  private url: string;
  private headers: Record<string, string>;

  constructor(url: string, headers: Record<string, string> = {}) {
    this.url = url.replace(/\/+$/, '');
    this.headers = { ...headers };
  }

  from(bucketId: string): StorageFileApi {
    return new StorageFileApi(this.url, bucketId, this.headers);
  }

  async listBuckets(): Promise<{ data: StorageBucket[] | null; error: any }> {
    const endpoint = `${this.url}/v1/storage/v1/bucket`;
    try {
      const res = await fetch(endpoint, { headers: { ...this.headers } });
      const data = await res.json().catch(() => null);
      if (!res.ok) {
        return { data: null, error: { message: data?.error || res.statusText } };
      }
      return { data: data as StorageBucket[], error: null };
    } catch (err: any) {
      return { data: null, error: { message: err.message || 'Failed to list buckets' } };
    }
  }

  async createBucket(id: string, options: { public?: boolean } = {}): Promise<{ data: { name: string } | null; error: any }> {
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
    } catch (err: any) {
      return { data: null, error: { message: err.message || 'Failed to create bucket' } };
    }
  }

  async deleteBucket(id: string): Promise<{ data: { message: string } | null; error: any }> {
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
    } catch (err: any) {
      return { data: null, error: { message: err.message || 'Failed to delete bucket' } };
    }
  }
}
