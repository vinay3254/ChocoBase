export class AuthClient {
    url;
    headers;
    currentSession = null;
    constructor(url, headers = {}) {
        this.url = url.replace(/\/+$/, '');
        this.headers = { ...headers };
    }
    setSession(session) {
        this.currentSession = session;
    }
    getSession() {
        return this.currentSession;
    }
    getUser() {
        return this.currentSession?.user || null;
    }
    async signUp(credentials) {
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
            const session = {
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
        }
        catch (err) {
            return {
                data: null,
                error: { message: err.message || 'Signup request failed' },
            };
        }
    }
    async signInWithPassword(credentials) {
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
            const session = {
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
        }
        catch (err) {
            return {
                data: null,
                error: { message: err.message || 'Login request failed' },
            };
        }
    }
    async refreshSession() {
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
            const session = {
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
        }
        catch (err) {
            return {
                data: null,
                error: { message: err.message || 'Refresh request failed' },
            };
        }
    }
    signOut() {
        this.currentSession = null;
        return { error: null };
    }
}
