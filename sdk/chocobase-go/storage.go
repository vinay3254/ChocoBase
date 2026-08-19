package chocobase

import "fmt"

// StorageClient manages object storage buckets and files.
type StorageClient struct {
	BaseURL string
	APIKey  string
}

func newStorageClient(baseURL, apiKey string) *StorageClient {
	return &StorageClient{
		BaseURL: baseURL + "/v1/storage/v1",
		APIKey:  apiKey,
	}
}

func (s *StorageClient) From(bucket string) *BucketClient {
	return &BucketClient{
		BaseURL: fmt.Sprintf("%s/object/%s", s.BaseURL, bucket),
		Bucket:  bucket,
		APIKey:  s.APIKey,
	}
}

type BucketClient struct {
	BaseURL string
	Bucket  string
	APIKey  string
}

type SignedURLResponse struct {
	SignedURL string `json:"signed_url"`
}

func (b *BucketClient) CreateSignedURL(path string, expiresIn int) (*SignedURLResponse, error) {
	return &SignedURLResponse{
		SignedURL: fmt.Sprintf("%s/sign/%s?expires_in=%d", b.BaseURL, path, expiresIn),
	}, nil
}
