export type RealtimeCallback = (payload: any) => void;
export declare class RealtimeChannel {
    private channelName;
    private url;
    private token?;
    private listeners;
    private ws?;
    constructor(url: string, channelName: string, token?: string);
    on(event: string, callback: RealtimeCallback): this;
    subscribe(): this;
    send(payload: any): void;
    unsubscribe(): void;
}
export declare class RealtimeClient {
    private url;
    private token?;
    constructor(url: string, token?: string);
    channel(name: string): RealtimeChannel;
}
