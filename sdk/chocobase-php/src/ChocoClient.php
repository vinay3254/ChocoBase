<?php

namespace ChocoBase;

class ChocoClient
{
    private string $url;
    private string $apiKey;
    private array $headers;

    public Auth $auth;
    public Storage $storage;
    public Functions $functions;

    public function __construct(string $url, string $apiKey, array $customHeaders = [])
    {
        $this->url = rtrim($url, '/');
        $this->apiKey = $apiKey;
        $this->headers = array_merge([
            'apikey' => $apiKey,
            'Authorization' => "Bearer {$apiKey}",
            'Content-Type' => 'application/json'
        ], $customHeaders);

        $this->auth = new Auth($this->url, $this->headers);
        $this->storage = new Storage($this->url, $this->headers);
        $this->functions = new Functions($this->url, $this->headers);
    }

    public function from(string $table): Postgrest
    {
        return new Postgrest($this->url, $table, $this->headers);
    }

    public static function createClient(string $url, string $apiKey, array $customHeaders = []): self
    {
        return new self($url, $apiKey, $customHeaders);
    }
}
