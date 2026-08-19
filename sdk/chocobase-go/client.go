package chocobase

import "strings"

// Client is the main client for interacting with ChocoBase.
type Client struct {
	BaseURL   string
	APIKey    string
	Auth      *AuthClient
	Postgrest *PostgrestClient
	Storage   *StorageClient
	Functions *FunctionsClient
	Realtime  *RealtimeClient
}

// NewClient initializes a new ChocoBase client with URL and API key.
func NewClient(url, apiKey string) *Client {
	cleanURL := strings.TrimRight(url, "/")
	return &Client{
		BaseURL:   cleanURL,
		APIKey:    apiKey,
		Auth:      newAuthClient(cleanURL, apiKey),
		Postgrest: newPostgrestClient(cleanURL, apiKey),
		Storage:   newStorageClient(cleanURL, apiKey),
		Functions: newFunctionsClient(cleanURL, apiKey),
		Realtime:  newRealtimeClient(cleanURL, apiKey),
	}
}

// From returns a QueryBuilder for the specified table.
func (c *Client) From(table string) *QueryBuilder {
	return c.Postgrest.From(table)
}

// Table is an alias for From.
func (c *Client) Table(table string) *QueryBuilder {
	return c.From(table)
}
