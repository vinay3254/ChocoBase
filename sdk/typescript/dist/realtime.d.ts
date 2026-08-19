export type RealtimeCallback = (payload: any) => void;
export declare class RealtimeChannel {
    private url;
    private apikey;
    private token;
    private channelTopic;
    private callbacks;
    private eventSource;
    constructor(url: string, apikey: string, token: string | null, topic: string);
    on(event: string, callback: RealtimeCallback): this;
    send(event: string, payload: any): Promise<{
        error: Error | null;
    }>;
    subscribe(): this;
    unsubscribe(): void;
}
export declare class RealtimeClient {
    private url;
    private apikey;
    private token;
    constructor(url: string, apikey: string, token: string | null);
    channel(topic: string): RealtimeChannel;
}
