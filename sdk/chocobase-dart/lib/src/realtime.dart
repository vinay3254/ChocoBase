import 'dart:convert';
import 'package:web_socket_channel/web_socket_channel.dart';

typedef RealtimeCallback = void Function(Map<String, dynamic> payload);

class RealtimeChannel {
  final String topic;
  final WebSocketChannel socket;
  final Map<String, List<RealtimeCallback>> _listeners = {};

  RealtimeChannel(this.topic, this.socket);

  RealtimeChannel on(String event, RealtimeCallback callback) {
    _listeners.putIfAbsent(event, () => []).add(callback);
    return this;
  }

  void subscribe() {
    final joinPayload = {
      'topic': topic,
      'event': 'phx_join',
      'payload': {},
      'ref': '1',
    };
    socket.sink.add(jsonEncode(joinPayload));

    socket.stream.listen((message) {
      if (message is String) {
        try {
          final data = jsonDecode(message) as Map<String, dynamic>;
          final event = data['event'] as String?;
          final payload = data['payload'] as Map<String, dynamic>? ?? {};

          if (event != null && _listeners.containsKey(event)) {
            for (final cb in _listeners[event]!) {
              cb(payload);
            }
          }
        } catch (_) {}
      }
    });
  }

  void send(String event, Map<String, dynamic> payload) {
    final msg = {
      'topic': topic,
      'event': event,
      'payload': payload,
      'ref': '2',
    };
    socket.sink.add(jsonEncode(msg));
  }
}

class RealtimeClient {
  final String baseUrl;
  final String apiKey;

  RealtimeClient(this.baseUrl, this.apiKey);

  RealtimeChannel channel(String topic) {
    final wsUrl = baseUrl.replaceFirst('http://', 'ws://').replaceFirst('https://', 'wss://');
    final uri = Uri.parse('$wsUrl/v1/realtime/v1/websocket?apikey=$apiKey');
    final socket = WebSocketChannel.connect(uri);
    return RealtimeChannel(topic, socket);
  }
}
