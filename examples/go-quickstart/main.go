package main

import (
	"fmt"
	"os"

	"github.com/vinay3254/ChocoBase/sdk/chocobase-go"
)

func main() {
	url := os.Getenv("CHOCOBASE_URL")
	if url == "" {
		url = "http://localhost:8080"
	}
	apiKey := os.Getenv("CHOCOBASE_ANON_KEY")
	if apiKey == "" {
		apiKey = "anon_key_dev"
	}

	fmt.Printf("🚀 Initializing ChocoBase Go Client at: %s\n", url)
	client := chocobase.NewClient(url, apiKey)

	// 1. User Authentication
	fmt.Println("\n🔐 Registering user...")
	authResp, err := client.Auth.SignUp("gopher@golang.org", "gopher-secret-password")
	if err != nil {
		fmt.Printf("Auth error: %v\n", err)
	} else {
		fmt.Printf("User registered successfully: %+v\n", authResp.User)
	}

	// 2. Database Queries with PostgREST query builder
	fmt.Println("\n📊 Executing PostgREST table query...")
	query := client.From("tasks").Select("id, title, status").Eq("completed", false).Limit(5)
	fmt.Printf("Query Params: %+v\n", query.Params)
	res, _ := query.Execute()
	fmt.Printf("Query Result: %+v\n", res)

	// 3. Storage signed URL creation
	fmt.Println("\n📦 Generating Object Storage signed URL...")
	signedURL, _ := client.Storage.From("documents").CreateSignedURL("spec.pdf", 3600)
	fmt.Printf("Signed URL: %s\n", signedURL.SignedURL)

	// 4. Serverless Edge Function Invocation
	fmt.Println("\n⚡ Invoking Edge Function...")
	fnResp, _ := client.Functions.Invoke("transform-data", map[string]string{"lang": "Go"})
	fmt.Printf("Edge function response: %+v\n", fnResp)

	// 5. Realtime Channel Subscription
	fmt.Println("\n📡 Setting up Realtime channel listener...")
	channel := client.Realtime.Channel("public:tasks")
	channel.On("INSERT", func(payload interface{}) {
		fmt.Printf("Realtime insert event: %+v\n", payload)
	})
	channel.Subscribe()
	fmt.Printf("Subscribed to channel: %s\n", channel.Topic)

	fmt.Println("\n✅ Go quickstart demonstration complete!")
}
