export interface ClientOptions {
    auth?: {
        autoRefreshToken?: boolean;
        persistSession?: boolean;
        storageKey?: string;
    };
    headers?: Record<string, string>;
}
export interface User {
    id: number;
    username: string;
    role: string;
}
export interface Session {
    access_token: string;
    refresh_token: string;
    token_type: string;
    expires_in?: number;
    user: User;
}
export interface AuthResponse {
    data: {
        user: User | null;
        session: Session | null;
    } | null;
    error: {
        message: string;
        code?: string;
    } | null;
}
export interface PostgrestResponse<T = any> {
    data: T | null;
    error: {
        message: string;
        code?: string;
        details?: string;
    } | null;
    count?: number | null;
    status: number;
    statusText: string;
}
export interface StorageBucket {
    id: string;
    name: string;
    public: boolean;
    created_at: number;
}
export interface StorageFile {
    Key: string;
    Id: string;
    size: number;
}
