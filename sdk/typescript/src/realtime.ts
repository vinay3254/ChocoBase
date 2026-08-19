export type RealtimeCallback = (payload: any) => void;

export class RealtimeChannel {
  private url: string;
  private apikey: string;
  private token: string | null;
  private channelTopic: string;
  private callbacks: Map<string, RealtimeCallback[]> = new Map();
  private eventSource: any = null;

  constructor(url: string, apikey: string, token: string | null, topic: string) {
    this.url = url.replace(/\/$/, "");
    this.apikey = apikey;
    this.token = token;
    this.channelTopic = topic;
  }

  on(event: string, callback: RealtimeCallback): this {
    const list = this.callbacks.get(event) || [];
    list.push(callback);
    this.callbacks.set(event, list);
    return this;
  }

  async send(event: string, payload: any): Promise<{ error: Error | null }> {
    try {
      const res = await fetch(`${this.url}/v1/realtime/v1/broadcast/${this.channelTopic}`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          apikey: this.apikey,
          ...(this.token ? { Authorization: `Bearer ${this.token}` } : {}),
        },
        body: JSON.stringify({ event, payload }),
      });
      if (!res.ok) {
        const json = await res.json().catch(() => ({}));
        return { error: new Error(json.error || "Broadcast failed") };
      }
      return { error: null };
    } catch (e: any) {
      return { error: e };
    }
  }

  subscribe(): this {
    const sseUrl = `${this.url}/v1/realtime/v1/sse/${this.channelTopic}${
      this.token ? `?token=${encodeURIComponent(this.token)}` : ""
    }`;

    if (typeof (globalThis as any).EventSource !== "undefined") {
      const EventSourceClass = (globalThis as any).EventSource;
      this.eventSource = new EventSourceClass(sseUrl);

      this.eventSource.addEventListener("broadcast", (event: any) => {
        try {
          const parsed = JSON.parse(event.data);
          const cbs = this.callbacks.get(parsed.event) || this.callbacks.get("broadcast") || [];
          for (const cb of cbs) {
            cb(parsed);
          }
        } catch (_) {}
      });

      this.eventSource.addEventListener("change", (event: any) => {
        try {
          const parsed = JSON.parse(event.data);
          const cbs = this.callbacks.get("change") || [];
          for (const cb of cbs) {
            cb(parsed);
          }
        } catch (_) {}
      });
    }

    return this;
  }

  unsubscribe() {
    if (this.eventSource) {
      this.eventSource.close();
      this.eventSource = null;
    }
  }
}

export class RealtimeClient {
  private url: string;
  private apikey: string;
  private token: string | null;

  constructor(url: string, apikey: string, token: string | null) {
    this.url = url;
    this.apikey = apikey;
    this.token = token;
  }

  channel(topic: string): RealtimeChannel {
    return new RealtimeChannel(this.url, this.apikey, this.token, topic);
  }
}
