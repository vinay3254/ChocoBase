<?php

namespace ChocoBase;

class Auth
{
    private string $baseUrl;
    private array $headers;

    public function __construct(string $baseUrl, array $headers)
    {
        $this->baseUrl = $baseUrl;
        $this->headers = $headers;
    }

    public function signUp(string $username, string $password): array
    {
        return $this->post('/v1/auth/signup', [
            'username' => $username,
            'password' => $password
        ]);
    }

    public function signIn(string $username, string $password): array
    {
        return $this->post('/v1/auth/token', [
            'username' => $username,
            'password' => $password
        ]);
    }

    private function post(string $path, array $data): array
    {
        $ch = curl_init("{$this->baseUrl}{$path}");
        $reqHeaders = [];
        foreach ($this->headers as $k => $v) {
            $reqHeaders[] = "{$k}: {$v}";
        }

        curl_setopt_array($ch, [
            CURLOPT_POST => true,
            CURLOPT_RETURNTRANSFER => true,
            CURLOPT_HTTPHEADER => $reqHeaders,
            CURLOPT_POSTFIELDS => json_encode($data)
        ]);

        $res = curl_exec($ch);
        curl_close($ch);

        return json_decode($res ?: '{}', true) ?: [];
    }
}
