<?php

namespace ChocoBase;

class Postgrest
{
    private string $baseUrl;
    private string $table;
    private array $headers;
    private array $params = [];

    public function __construct(string $baseUrl, string $table, array $headers)
    {
        $this->baseUrl = $baseUrl;
        $this->table = $table;
        $this->headers = $headers;
    }

    public function select(string $columns = '*'): self
    {
        $this->params['select'] = $columns;
        return $this;
    }

    public function eq(string $column, $value): self
    {
        $this->params[$column] = "eq.{$value}";
        return $this;
    }

    public function limit(int $count): self
    {
        $this->params['limit'] = (string)$count;
        return $this;
    }

    public function execute(): array
    {
        $query = http_build_query($this->params);
        $url = "{$this->baseUrl}/rest/v1/{$this->table}" . ($query ? "?{$query}" : "");

        $ch = curl_init($url);
        $reqHeaders = [];
        foreach ($this->headers as $k => $v) {
            $reqHeaders[] = "{$k}: {$v}";
        }

        curl_setopt_array($ch, [
            CURLOPT_RETURNTRANSFER => true,
            CURLOPT_HTTPHEADER => $reqHeaders
        ]);

        $res = curl_exec($ch);
        curl_close($ch);

        $decoded = json_decode($res ?: '[]', true);
        if (isset($decoded['rows'])) {
            return $decoded['rows'];
        }
        return is_array($decoded) ? $decoded : [];
    }
}
