// ChocoBase Dart & Flutter Quickstart Example
import 'package:chocobase/chocobase.dart';

void main() async {
  print('🍫 ChocoBase Dart / Flutter Quickstart');

  final client = createClient('http://localhost:8080', 'anon_dev_token');

  // 1. Auth: Sign up
  final authResp = await client.auth.signUp(
    username: 'flutter_dev',
    password: 'secure_password_123',
  );
  if (authResp.error != null) {
    print('Auth notice: ${authResp.error}');
  } else {
    print('Authenticated as: ${authResp.user?.username}');
  }

  // 2. Database: Query PostgREST
  final tasks = await client
      .from('tasks')
      .select('id, title, completed')
      .eq('completed', false)
      .limit(5)
      .execute();

  print('Pending tasks: ${tasks.data}');

  // 3. Storage: Generate Signed URL
  final signedUrl = await client.storage.from('documents').createSignedUrl('guide.pdf', expiresIn: 3600);
  print('Signed download URL: $signedUrl');

  // 4. Edge Functions: Invoke
  final fnResp = await client.functions.invoke('hello', body: {'framework': 'Flutter'});
  print('Function result: ${fnResp.data}');

  print('✅ Dart Quickstart completed successfully!');
}
