from typing import Callable, Dict, Any, List

class RealtimeChannel:
    def __init__(self, topic: str):
        self.topic = topic
        self.listeners: List[Dict[str, Any]] = []

    def on(self, event: str, callback: Callable[[Any], None]) -> "RealtimeChannel":
        self.listeners.append({"event": event, "callback": callback})
        return self

    def subscribe(self) -> "RealtimeChannel":
        return self

class RealtimeClient:
    """Client for WebSocket CDC realtime channels."""

    def __init__(self, url: str, key: str):
        ws_url = url.replace("http://", "ws://").replace("https://", "wss://")
        self.url = f"{ws_url}/v1/realtime"
        self.key = key
        self.channels: Dict[str, RealtimeChannel] = {}

    def channel(self, topic: str) -> RealtimeChannel:
        if topic not in self.channels:
            self.channels[topic] = RealtimeChannel(topic)
        return self.channels[topic]
