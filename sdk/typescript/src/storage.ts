export interface StorageResponse<T = any> {
  data: T | null;
  error: Error | null;
}

export class StorageFileApi {
  private url: string;
  private apikey: string;
  private token: string | null;
  private bucketId: string;

  constructor(url: string, apikey: string, token: string | null, bucketId: string) {
    this.url = url.replace(/\/$/, "");
    this.apikey = apikey;
    this.token = token;
    this.bucketId = bucketId;
  }

  private getHeaders(): Record<string, string> {
    const headers: Record<string, string> = { apikey: this.apikey };
    if (this.token) {
      headers["Authorization"] = `Bearer ${this.token}`;
    }
    return headers;
  }

  async upload(path: string, fileBody: string | Uint8Array): Promise<StorageResponse<{ Key: string; etag: string }>> {
    try {
      const res = await fetch(`${this.url}/v1/storage/v1/object/${this.bucketId}/${path}`, {
        method: "POST",
        headers: {
          ...this.getHeaders(),
          "Content-Type": "application/octet-stream",
        },
        body: fileBody as any,
      });
      const json = await res.json();
      if (!res.ok) {
        return { data: null, error: new Error(json.error || "Upload failed") };
      }
      return { data: json, error: null };
    } catch (e: any) {
      return { data: null, error: e };
    }
  }

  async download(path: string, options?: { range?: string }): Promise<StorageResponse<Blob>> {
    try {
      const headers: Record<string, string> = { ...this.getHeaders() };
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
    } catch (e: any) {
      return { data: null, error: e };
    }
  }

  async list(): Promise<StorageResponse<Array<{ name: string; id: string; metadata: any }>>> {
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
    } catch (e: any) {
      return { data: null, error: e };
    }
  }

  async remove(paths: string[]): Promise<StorageResponse<any>> {
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
    } catch (e: any) {
      return { data: null, error: e };
    }
  }

  async createSignedUrl(path: string, expiresIn: number): Promise<StorageResponse<{ signedUrl: string }>> {
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
    } catch (e: any) {
      return { data: null, error: e };
    }
  }
}

export class StorageClient {
  private url: string;
  private apikey: string;
  private token: string | null;

  constructor(url: string, apikey: string, token: string | null) {
    this.url = url;
    this.apikey = apikey;
    this.token = token;
  }

  from(bucketId: string): StorageFileApi {
    return new StorageFileApi(this.url, this.apikey, this.token, bucketId);
  }

  async createBucket(id: string, options?: { public?: boolean }): Promise<StorageResponse<any>> {
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
    } catch (e: any) {
      return { data: null, error: e };
    }
  }
}
