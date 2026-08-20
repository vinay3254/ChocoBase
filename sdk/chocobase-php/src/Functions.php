<?php

namespace ChocoBase;

class Functions
{
    private string $baseUrl;
    private array $headers;

    public function __construct(string $baseUrl, array $headers)
    {
        $this->baseUrl = $baseUrl;
        $this->headers = $headers;
    }

    public function invoke(string $functionName, array $body = []): array
    {
        $url = "{$this->baseUrl}/v1/functions/v1/{$functionName}";
        $ch = curl_init($url);
        $reqHeaders = [];
        foreach ($this->headers as $k => $v) {
            $reqHeaders[] = "{$k}: {$v}";
        }

        curl_setopt_array($ch, [
            CURLOPT_POST => true,
            CURLOPT_RETURNTRANSFER => true,
            CURLOPT_HTTPHEADER => $reqHeaders,
            CURLOPT_POSTFIELDS => json_encode($body)
        ]);

        $res = curl_exec($ch);
        curl_close($ch);

        return json_decode($res ?: '[]', true) ?: ['raw' => $res];
    }
}
