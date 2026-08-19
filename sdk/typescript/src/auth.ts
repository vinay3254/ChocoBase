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
  data: { user: User | null; session: Session | null } | null;
  error: Error | null;
}

export class AuthClient {
  private url: string;
  private apikey: string;
  private currentSession: Session | null = null;

  constructor(url: string, apikey: string) {
    this.url = url.replace(/\/$/, "");
    this.apikey = apikey;
  }

  get session(): Session | null {
    return this.currentSession;
  }

  setSession(session: Session | null) {
    this.currentSession = session;
  }

  async signUp(credentials: { username: string; password: string }): Promise<AuthResponse> {
    try {
      const res = await fetch(`${this.url}/v1/auth/signup`, {
        method: "POST",
        headers: { "Content-Type": "application/json", apikey: this.apikey },
        body: JSON.stringify(credentials),
      });
      const json = await res.json();
      if (!res.ok) {
        return { data: null, error: new Error(json.error || "Sign up failed") };
      }
      return { data: { user: json.user, session: null }, error: null };
    } catch (e: any) {
      return { data: null, error: e };
    }
  }

  async signInWithPassword(credentials: { username: string; password: string }): Promise<AuthResponse> {
    try {
      const res = await fetch(`${this.url}/v1/auth/token`, {
        method: "POST",
        headers: { "Content-Type": "application/json", apikey: this.apikey },
        body: JSON.stringify(credentials),
      });
      const json = await res.json();
      if (!res.ok) {
        return { data: null, error: new Error(json.error || "Authentication failed") };
      }
      this.currentSession = {
        access_token: json.token,
        refresh_token: json.refresh_token,
        expires_in: json.expires_in,
        user: { id: json.user_id, username: json.username, role: json.role },
      };
      return { data: { user: this.currentSession.user, session: this.currentSession }, error: null };
    } catch (e: any) {
      return { data: null, error: e };
    }
  }

  async refreshSession(): Promise<AuthResponse> {
    if (!this.currentSession?.refresh_token) {
      return { data: null, error: new Error("No refresh token present") };
    }
    try {
      const res = await fetch(`${this.url}/v1/auth/refresh`, {
        method: "POST",
        headers: { "Content-Type": "application/json", apikey: this.apikey },
        body: JSON.stringify({ refresh_token: this.currentSession.refresh_token }),
      });
      const json = await res.json();
      if (!res.ok) {
        return { data: null, error: new Error(json.error || "Refresh failed") };
      }
      this.currentSession = {
        access_token: json.token,
        refresh_token: json.refresh_token,
        expires_in: json.expires_in,
        user: { id: json.user_id, username: json.username, role: json.role },
      };
      return { data: { user: this.currentSession.user, session: this.currentSession }, error: null };
    } catch (e: any) {
      return { data: null, error: e };
    }
  }

  async signOut(): Promise<{ error: Error | null }> {
    try {
      if (this.currentSession?.refresh_token) {
        await fetch(`${this.url}/v1/auth/logout`, {
          method: "POST",
          headers: { "Content-Type": "application/json", apikey: this.apikey },
          body: JSON.stringify({ refresh_token: this.currentSession.refresh_token }),
        });
      }
      this.currentSession = null;
      return { error: null };
    } catch (e: any) {
      return { error: e };
    }
  }
}
