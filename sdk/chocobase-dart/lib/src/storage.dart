import 'dart:convert';
import 'dart:typed_data';
import 'package:http/http.dart' as http;

class StorageResponse {
  final dynamic data;
  final String? error;

  StorageResponse({this.data, this.error});
}

class StorageFileApi {
  final String baseUrl;
  final String bucket;
  final Map<String, String> headers;

  StorageFileApi(this.baseUrl, this.bucket, this.headers);

  Future<StorageResponse> upload(String path, Uint8List fileBytes, {String contentType = 'application/octet-stream'}) async {
    final uri = Uri.parse('$baseUrl/v1/storage/v1/object/$bucket/$path');
    final uploadHeaders = Map<String, String>.from(headers);
    uploadHeaders['Content-Type'] = contentType;

    final response = await http.post(uri, headers: uploadHeaders, body: fileBytes);
    if (response.statusCode >= 200 && response.statusCode < 300) {
      return StorageResponse(data: jsonDecode(response.body));
    } else {
      return StorageResponse(error: 'Upload failed: ${response.statusCode}');
    }
  }

  Future<String?> createSignedUrl(String path, {int expiresIn = 3600}) async {
    final uri = Uri.parse('$baseUrl/v1/storage/v1/object/sign/$bucket/$path');
    final response = await http.post(
      uri,
      headers: headers,
      body: jsonEncode({'expires_in': expiresIn}),
    );

    if (response.statusCode >= 200 && response.statusCode < 300) {
      final json = jsonDecode(response.body) as Map<String, dynamic>;
      final signedUrl = json['signed_url'] as String?;
      return signedUrl != null ? '$baseUrl$signedUrl' : null;
    }
    return null;
  }
}

class StorageClient {
  final String baseUrl;
  final Map<String, String> headers;

  StorageClient(this.baseUrl, this.headers);

  StorageFileApi from(String bucket) {
    return StorageFileApi(baseUrl, bucket, headers);
  }
}
