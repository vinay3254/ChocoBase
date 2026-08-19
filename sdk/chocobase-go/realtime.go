package chocobase

import "strings"

// RealtimeClient manages Realtime WebSocket subscriptions.
type RealtimeClient struct {
	BaseURL string
	APIKey  string
}

func newRealtimeClient(baseURL, apiKey string) *RealtimeClient {
	wsURL := baseURL
	if strings.HasPrefix(wsURL, "http://") {
		wsURL = strings.Replace(wsURL, "http://", "ws://", 1)
	} else if strings.HasPrefix(wsURL, "https://") {
		wsURL = strings.Replace(wsURL, "https://", "wss://", 1)
	}
	return &RealtimeClient{
		BaseURL: wsURL + "/v1/realtime",
		APIKey:  apiKey,
	}
}

type RealtimeChannel struct {
	Topic string
}

func (r *RealtimeClient) Channel(topic string) *RealtimeChannel {
	return &RealtimeChannel{Topic: topic}
}

func (c *RealtimeChannel) On(event string, handler func(payload interface{})) *RealtimeChannel {
	return c
}

func (c *RealtimeChannel) Subscribe() *RealtimeChannel {
	return c
}
