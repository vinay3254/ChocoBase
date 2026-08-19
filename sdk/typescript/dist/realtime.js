"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.RealtimeClient = exports.RealtimeChannel = void 0;
class RealtimeChannel {
    url;
    apikey;
    token;
    channelTopic;
    callbacks = new Map();
    eventSource = null;
    constructor(url, apikey, token, topic) {
        this.url = url.replace(/\/$/, "");
        this.apikey = apikey;
        this.token = token;
        this.channelTopic = topic;
    }
    on(event, callback) {
        const list = this.callbacks.get(event) || [];
        list.push(callback);
        this.callbacks.set(event, list);
        return this;
    }
    async send(event, payload) {
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
        }
        catch (e) {
            return { error: e };
        }
    }
    subscribe() {
        const sseUrl = `${this.url}/v1/realtime/v1/sse/${this.channelTopic}${this.token ? `?token=${encodeURIComponent(this.token)}` : ""}`;
        if (typeof globalThis.EventSource !== "undefined") {
            const EventSourceClass = globalThis.EventSource;
            this.eventSource = new EventSourceClass(sseUrl);
            this.eventSource.addEventListener("broadcast", (event) => {
                try {
                    const parsed = JSON.parse(event.data);
                    const cbs = this.callbacks.get(parsed.event) || this.callbacks.get("broadcast") || [];
                    for (const cb of cbs) {
                        cb(parsed);
                    }
                }
                catch (_) { }
            });
            this.eventSource.addEventListener("change", (event) => {
                try {
                    const parsed = JSON.parse(event.data);
                    const cbs = this.callbacks.get("change") || [];
                    for (const cb of cbs) {
                        cb(parsed);
                    }
                }
                catch (_) { }
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
exports.RealtimeChannel = RealtimeChannel;
class RealtimeClient {
    url;
    apikey;
    token;
    constructor(url, apikey, token) {
        this.url = url;
        this.apikey = apikey;
        this.token = token;
    }
    channel(topic) {
        return new RealtimeChannel(this.url, this.apikey, this.token, topic);
    }
}
exports.RealtimeClient = RealtimeClient;
