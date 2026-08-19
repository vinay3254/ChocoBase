export type RealtimeCallback = (payload: any) => void;

export class RealtimeChannel {
  private channelName: string;
  private url: string;
  private token?: string;
  private listeners: Map<string, RealtimeCallback[]> = new Map();
  private ws?: any;

  constructor(url: string, channelName: string, token?: string) {
    this.url = url;
    this.channelName = channelName;
    this.token = token;
  }

  on(event: string, callback: RealtimeCallback): this {
    const list = this.listeners.get(event) || [];
    list.push(callback);
    this.listeners.set(event, list);
    return this;
  }

  subscribe(): this {
    const wsUrl = this.url.replace(/^http/, 'ws') + `/v1/realtime?channel=${encodeURIComponent(this.channelName)}` + (this.token ? `&token=${encodeURIComponent(this.token)}` : '');
    
    if (typeof WebSocket !== 'undefined') {
      try {
        this.ws = new WebSocket(wsUrl);
        this.ws.onmessage = (event: any) => {
          try {
            const data = JSON.parse(event.data);
            const eventType = data.type || data.event || '*';
            const callbacks = this.listeners.get(eventType) || [];
            callbacks.forEach(cb => cb(data));
            const wildcard = this.listeners.get('*') || [];
            wildcard.forEach(cb => cb(data));
          } catch (e) {
            // non-json message
          }
        };
      } catch (err) {
        console.warn('WebSocket connection failed:', err);
      }
    }
    return this;
  }

  send(payload: any): void {
    if (this.ws && this.ws.readyState === 1) {
      this.ws.send(JSON.stringify(payload));
    }
  }

  unsubscribe(): void {
    if (this.ws) {
      this.ws.close();
      this.ws = undefined;
    }
    this.listeners.clear();
  }
}

export class RealtimeClient {
  private url: string;
  private token?: string;

  constructor(url: string, token?: string) {
    this.url = url;
    this.token = token;
  }

  channel(name: string): RealtimeChannel {
    return new RealtimeChannel(this.url, name, this.token);
  }
}
