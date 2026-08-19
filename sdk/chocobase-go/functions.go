package chocobase

// FunctionsClient handles Edge Function invocation.
type FunctionsClient struct {
	BaseURL string
	APIKey  string
}

func newFunctionsClient(baseURL, apiKey string) *FunctionsClient {
	return &FunctionsClient{
		BaseURL: baseURL + "/v1/functions",
		APIKey:  apiKey,
	}
}

type FunctionResponse struct {
	Data  interface{} `json:"data"`
	Error string      `json:"error,omitempty"`
}

func (f *FunctionsClient) Invoke(functionName string, body interface{}) (*FunctionResponse, error) {
	return &FunctionResponse{
		Data: map[string]string{
			"message": "Response from " + functionName,
		},
	}, nil
}
