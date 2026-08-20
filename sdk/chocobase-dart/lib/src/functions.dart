import 'dart:convert';
import 'package:http/http.dart' as http;

class FunctionResponse {
  final dynamic data;
  final String? error;

  FunctionResponse({this.data, this.error});
}

class FunctionsClient {
  final String baseUrl;
  final Map<String, String> headers;

  FunctionsClient(this.baseUrl, this.headers);

  Future<FunctionResponse> invoke(String functionName, {dynamic body}) async {
    final uri = Uri.parse('$baseUrl/v1/functions/v1/$functionName');
    final response = await http.post(
      uri,
      headers: headers,
      body: body != null ? jsonEncode(body) : null,
    );

    if (response.statusCode >= 200 && response.statusCode < 300) {
      try {
        final decoded = jsonDecode(response.body);
        return FunctionResponse(data: decoded);
      } catch (_) {
        return FunctionResponse(data: response.body);
      }
    } else {
      return FunctionResponse(error: 'Function invocation failed: ${response.statusCode}');
    }
  }
}
