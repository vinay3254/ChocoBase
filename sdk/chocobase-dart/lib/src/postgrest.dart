import 'dart:convert';
import 'package:http/http.dart' as http;

class PostgrestResponse {
  final List<dynamic>? data;
  final int? count;
  final String? error;

  PostgrestResponse({this.data, this.count, this.error});
}

class QueryBuilder {
  final String baseUrl;
  final String table;
  final Map<String, String> headers;
  final Map<String, String> params = {};

  QueryBuilder(this.baseUrl, this.table, this.headers);

  QueryBuilder select([String columns = '*']) {
    params['select'] = columns;
    return this;
  }

  QueryBuilder eq(String column, dynamic value) {
    params[column] = 'eq.$value';
    return this;
  }

  QueryBuilder neq(String column, dynamic value) {
    params[column] = 'neq.$value';
    return this;
  }

  QueryBuilder gt(String column, dynamic value) {
    params[column] = 'gt.$value';
    return this;
  }

  QueryBuilder lt(String column, dynamic value) {
    params[column] = 'lt.$value';
    return this;
  }

  QueryBuilder limit(int count) {
    params['limit'] = count.toString();
    return this;
  }

  QueryBuilder order(String column, {bool ascending = true}) {
    params['order'] = '$column.${ascending ? "asc" : "desc"}';
    return this;
  }

  Future<PostgrestResponse> execute() async {
    final uri = Uri.parse('$baseUrl/rest/v1/$table').replace(queryParameters: params);
    final response = await http.get(uri, headers: headers);

    if (response.statusCode >= 200 && response.statusCode < 300) {
      final decoded = jsonDecode(response.body);
      if (decoded is List) {
        return PostgrestResponse(data: decoded);
      } else if (decoded is Map<String, dynamic> && decoded.containsKey('rows')) {
        return PostgrestResponse(data: decoded['rows'] as List?);
      } else {
        return PostgrestResponse(data: [decoded]);
      }
    } else {
      return PostgrestResponse(error: 'Query failed with status: ${response.statusCode}');
    }
  }

  Future<PostgrestResponse> insert(dynamic record) async {
    final uri = Uri.parse('$baseUrl/rest/v1/$table');
    final response = await http.post(
      uri,
      headers: headers,
      body: jsonEncode(record),
    );

    if (response.statusCode >= 200 && response.statusCode < 300) {
      return PostgrestResponse(data: [jsonDecode(response.body)]);
    } else {
      return PostgrestResponse(error: 'Insert failed with status: ${response.statusCode}');
    }
  }

  Future<PostgrestResponse> delete() async {
    final uri = Uri.parse('$baseUrl/rest/v1/$table').replace(queryParameters: params);
    final response = await http.delete(uri, headers: headers);

    if (response.statusCode >= 200 && response.statusCode < 300) {
      return PostgrestResponse(data: []);
    } else {
      return PostgrestResponse(error: 'Delete failed with status: ${response.statusCode}');
    }
  }
}
