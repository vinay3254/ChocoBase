/// Official Dart and Flutter client for ChocoBase.
library chocobase;

export 'src/auth.dart';
export 'src/postgrest.dart';
export 'src/storage.dart';
export 'src/realtime.dart';
export 'src/functions.dart';

import 'src/auth.dart';
import 'src/postgrest.dart';
import 'src/storage.dart';
import 'src/realtime.dart';
import 'src/functions.dart';

/// Main client for connecting to a ChocoBase project.
class ChocoClient {
  final String url;
  final String apiKey;
  final Map<String, String> headers;

  late final AuthClient auth;
  late final StorageClient storage;
  late final FunctionsClient functions;
  late final RealtimeClient realtime;

  ChocoClient(this.url, this.apiKey, {Map<String, String>? customHeaders})
      : headers = {
          'apikey': apiKey,
          'Authorization': 'Bearer $apiKey',
          'Content-Type': 'application/json',
          ...?customHeaders,
        } {
    auth = AuthClient(url, headers);
    storage = StorageClient(url, headers);
    functions = FunctionsClient(url, headers);
    realtime = RealtimeClient(url, apiKey);
  }

  /// Create a query builder targeting a specific database table.
  QueryBuilder from(String table) {
    return QueryBuilder(url, table, headers);
  }
}

/// Helper function to create a new [ChocoClient] instance.
ChocoClient createClient(String url, String apiKey, {Map<String, String>? customHeaders}) {
  return ChocoClient(url, apiKey, customHeaders: customHeaders);
}
