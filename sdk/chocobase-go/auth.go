package chocobase

// AuthClient manages authentication and user credentials.
type AuthClient struct {
	BaseURL string
	APIKey  string
}

func newAuthClient(baseURL, apiKey string) *AuthClient {
	return &AuthClient{
		BaseURL: baseURL + "/v1/auth",
		APIKey:  apiKey,
	}
}

// User represents authenticated user details.
type User struct {
	ID    string `json:"id"`
	Email string `json:"email"`
}

// Session represents active JWT tokens.
type Session struct {
	AccessToken  string `json:"access_token"`
	RefreshToken string `json:"refresh_token"`
}

// AuthResponse holds user and session data.
type AuthResponse struct {
	User    *User    `json:"user"`
	Session *Session `json:"session"`
	Error   string   `json:"error,omitempty"`
}

func (a *AuthClient) SignUp(email, password string) (*AuthResponse, error) {
	return &AuthResponse{
		User: &User{
			ID:    "usr_go_generated",
			Email: email,
		},
		Session: &Session{
			AccessToken:  "mock_jwt_token",
			RefreshToken: "rt_mock_refresh_token",
		},
	}, nil
}

func (a *AuthClient) SignInWithPassword(email, password string) (*AuthResponse, error) {
	return &AuthResponse{
		User: &User{
			ID:    "usr_go_generated",
			Email: email,
		},
		Session: &Session{
			AccessToken:  "mock_jwt_token",
			RefreshToken: "rt_mock_refresh_token",
		},
	}, nil
}
