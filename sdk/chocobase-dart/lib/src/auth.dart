import 'dart:convert';
import 'package:http/http.dart' as http;

class User {
  final int id;
  final String username;
  final String role;

  User({required this.id, required this.username, required this.role});

  factory User.fromJson(Map<String, dynamic> json) {
    return User(
      id: json['id'] as int? ?? 0,
      username: json['username'] as String? ?? '',
      role: json['role'] as String? ?? 'user',
    );
  }
}

class AuthResponse {
  final String? accessToken;
  final String? refreshToken;
  final User? user;
  final String? error;

  AuthResponse({this.accessToken, this.refreshToken, this.user, this.error});
}

class AuthClient {
  final String url;
  final Map<String, String> headers;

  AuthClient(this.url, this.headers);

  Future<AuthResponse> signUp({required String username, required String password}) async {
    final endpoint = Uri.parse('$url/v1/auth/signup');
    final response = await http.post(
      endpoint,
      headers: headers,
      body: jsonEncode({'username': username, 'password': password}),
    );

    final data = jsonDecode(response.body) as Map<String, dynamic>;
    if (response.statusCode >= 200 && response.statusCode < 300) {
      return AuthResponse(
        accessToken: data['access_token'] as String?,
        refreshToken: data['refresh_token'] as String?,
        user: data['user'] != null ? User.fromJson(data['user'] as Map<String, dynamic>) : null,
      );
    } else {
      return AuthResponse(error: data['error'] as String? ?? 'Sign up failed');
    }
  }

  Future<AuthResponse> signInWithPassword({required String username, required String password}) async {
    final endpoint = Uri.parse('$url/v1/auth/token');
    final response = await http.post(
      endpoint,
      headers: headers,
      body: jsonEncode({'username': username, 'password': password}),
    );

    final data = jsonDecode(response.body) as Map<String, dynamic>;
    if (response.statusCode >= 200 && response.statusCode < 300) {
      return AuthResponse(
        accessToken: data['access_token'] as String?,
        refreshToken: data['refresh_token'] as String?,
        user: data['user'] != null ? User.fromJson(data['user'] as Map<String, dynamic>) : null,
      );
    } else {
      return AuthResponse(error: data['error'] as String? ?? 'Sign in failed');
    }
  }

  Future<AuthResponse> refreshSession(String refreshToken) async {
    final endpoint = Uri.parse('$url/v1/auth/refresh');
    final response = await http.post(
      endpoint,
      headers: headers,
      body: jsonEncode({'refresh_token': refreshToken}),
    );

    final data = jsonDecode(response.body) as Map<String, dynamic>;
    if (response.statusCode >= 200 && response.statusCode < 300) {
      return AuthResponse(
        accessToken: data['access_token'] as String?,
        refreshToken: data['refresh_token'] as String?,
      );
    } else {
      return AuthResponse(error: data['error'] as String? ?? 'Refresh session failed');
    }
  }
}
