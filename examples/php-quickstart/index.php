<?php

require_once __DIR__ . '/../../sdk/chocobase-php/src/ChocoClient.php';
require_once __DIR__ . '/../../sdk/chocobase-php/src/Auth.php';
require_once __DIR__ . '/../../sdk/chocobase-php/src/Postgrest.php';
require_once __DIR__ . '/../../sdk/chocobase-php/src/Storage.php';
require_once __DIR__ . '/../../sdk/chocobase-php/src/Functions.php';

use ChocoBase\ChocoClient;

echo "🍫 ChocoBase PHP & Laravel Quickstart\n";

$client = ChocoClient::createClient('http://localhost:8080', 'anon_dev_token');

// 1. Auth: Sign up
$auth = $client->auth->signUp('php_dev', 'secure_password_123');
echo "Auth user: " . ($auth['user']['username'] ?? 'anon') . "\n";

// 2. PostgREST: Query table
$posts = $client->from('posts')->select('id, title, content')->limit(5)->execute();
echo "Fetched " . count($posts) . " posts.\n";

// 3. Storage: Signed URL
$signedUrl = $client->storage->from('documents')->createSignedUrl('contract.pdf', 3600);
echo "Signed download URL: {$signedUrl}\n";

// 4. Edge Functions: Invoke
$res = $client->functions->invoke('send-notification', ['type' => 'email', 'to' => 'user@example.com']);
echo "Function result: " . json_encode($res) . "\n";

echo "✅ PHP Quickstart completed successfully!\n";
