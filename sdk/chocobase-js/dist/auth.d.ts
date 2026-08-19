import { AuthResponse, Session, User } from './types.js';
export declare class AuthClient {
    private url;
    private headers;
    private currentSession;
    constructor(url: string, headers?: Record<string, string>);
    setSession(session: Session | null): void;
    getSession(): Session | null;
    getUser(): User | null;
    signUp(credentials: {
        username: string;
        password: string;
        role?: string;
    }): Promise<AuthResponse>;
    signInWithPassword(credentials: {
        username: string;
        password: string;
    }): Promise<AuthResponse>;
    refreshSession(): Promise<AuthResponse>;
    signOut(): {
        error: null;
    };
}
