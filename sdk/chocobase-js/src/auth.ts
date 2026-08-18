import { AuthResponse, Session, User } from './types.js';

export class AuthClient {
  private url: string;
  private headers: Record<string, string>;
  private currentSession: Session | null = null;

  constructor(url: string, headers: Record<string, string> = {}) {
    this.url = url.replace(/\/+$/, '');
    this.headers = { ...headers };
  }

  setSession(session: Session | null) {
    this.currentSession = session;
  }

  getSession(): Session | null {
    return this.currentSession;
  }

  getUser(): User | null {
    return this.currentSession?.user || null;
  }

  async signUp(credentials: { username: string; password: string; role?: string }): Promise<AuthResponse> {
    const endpoint = `${this.url}/v1/auth/signup`;
    try {
      const res = await fetch(endpoint, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          ...this.headers,
        },
        body: JSON.stringify(credentials),
      });

      const data = await res.json().catch(() => null);
      if (!res.ok) {
        return {
          data: null,
          error: { message: data?.error || res.statusText },
        };
      }

      const session: Session = {
        access_token: data.access_token,
        refresh_token: data.refresh_token,
        token_type: data.token_type,
        user: data.user,
      };
      this.currentSession = session;

      return {
        data: { user: session.user, session },
        error: null,
      };
    } catch (err: any) {
      return {
        data: null,
        error: { message: err.message || 'Signup request failed' },
      };
    }
  }

  async signInWithPassword(credentials: { username: string; password: string }): Promise<AuthResponse> {
    const endpoint = `${this.url}/v1/auth/token`;
    try {
      const res = await fetch(endpoint, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          ...this.headers,
        },
        body: JSON.stringify(credentials),
      });

      const data = await res.json().catch(() => null);
      if (!res.ok) {
        return {
          data: null,
          error: { message: data?.error || 'Invalid credentials' },
        };
      }

      const session: Session = {
        access_token: data.access_token,
        refresh_token: data.refresh_token,
        token_type: data.token_type,
        user: data.user,
      };
      this.currentSession = session;

      return {
        data: { user: session.user, session },
        error: null,
      };
    } catch (err: any) {
      return {
        data: null,
        error: { message: err.message || 'Login request failed' },
      };
    }
  }

  async refreshSession(): Promise<AuthResponse> {
    if (!this.currentSession?.refresh_token) {
      return {
        data: null,
        error: { message: 'No refresh token available' },
      };
    }

    const endpoint = `${this.url}/v1/auth/refresh`;
    try {
      const res = await fetch(endpoint, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          ...this.headers,
        },
        body: JSON.stringify({ refresh_token: this.currentSession.refresh_token }),
      });

      const data = await res.json().catch(() => null);
      if (!res.ok) {
        return {
          data: null,
          error: { message: data?.error || 'Session refresh failed' },
        };
      }

      const session: Session = {
        access_token: data.access_token,
        refresh_token: data.refresh_token,
        token_type: data.token_type,
        user: data.user,
      };
      this.currentSession = session;

      return {
        data: { user: session.user, session },
        error: null,
      };
    } catch (err: any) {
      return {
        data: null,
        error: { message: err.message || 'Refresh request failed' },
      };
    }
  }

  signOut(): { error: null } {
    this.currentSession = null;
    return { error: null };
  }
}
