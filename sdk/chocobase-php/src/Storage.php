<?php

namespace ChocoBase;

class StorageBucket
{
    private string $baseUrl;
    private string $bucket;
    private array $headers;

    public function __construct(string $baseUrl, string $bucket, array $headers)
    {
        $this->baseUrl = $baseUrl;
        $this->bucket = $bucket;
        $this->headers = $headers;
    }

    public function createSignedUrl(string $path, int $expiresIn = 3600): ?string
    {
        $url = "{$this->baseUrl}/v1/storage/v1/object/sign/{$this->bucket}/{$path}";
        $ch = curl_init($url);
        $reqHeaders = [];
        foreach ($this->headers as $k => $v) {
            $reqHeaders[] = "{$k}: {$v}";
        }

        curl_setopt_array($ch, [
            CURLOPT_POST => true,
            CURLOPT_RETURNTRANSFER => true,
            CURLOPT_HTTPHEADER => $reqHeaders,
            CURLOPT_POSTFIELDS => json_encode(['expires_in' => $expiresIn])
        ]);

        $res = curl_exec($ch);
        curl_close($ch);

        $decoded = json_decode($res ?: '{}', true);
        if (isset($decoded['signed_url'])) {
            return "{$this->baseUrl}{$decoded['signed_url']}";
        }
        return null;
    }
}

class Storage
{
    private string $baseUrl;
    private array $headers;

    public function __construct(string $baseUrl, array $headers)
    {
        $this->baseUrl = $baseUrl;
        $this->headers = $headers;
    }

    public function from(string $bucket): StorageBucket
    {
        return new StorageBucket($this->baseUrl, $bucket, $this->headers);
    }
}
