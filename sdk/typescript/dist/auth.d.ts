export interface User {
    id: number;
    username: string;
    role: string;
}
export interface Session {
    access_token: string;
    refresh_token: string;
    expires_in: number;
    user: User;
}
export interface AuthResponse {
    data: {
        user: User | null;
        session: Session | null;
    } | null;
    error: Error | null;
}
export declare class AuthClient {
    private url;
    private apikey;
    private currentSession;
    constructor(url: string, apikey: string);
    get session(): Session | null;
    setSession(session: Session | null): void;
    signUp(credentials: {
        username: string;
        password: string;
    }): Promise<AuthResponse>;
    signInWithPassword(credentials: {
        username: string;
        password: string;
    }): Promise<AuthResponse>;
    refreshSession(): Promise<AuthResponse>;
    signOut(): Promise<{
        error: Error | null;
    }>;
}
