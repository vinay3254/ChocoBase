//! RFC 6455 WebSocket Engine for Supabase Realtime Channels in ChocoBase.
//! Provides persistent full-duplex WebSocket connections for live Broadcast, Presence, and Database Changefeeds.

use std::collections::HashMap;
use std::io;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::auth::ExecutionContext;
use crate::engine::SharedDatabase;
use crate::http::realtime_channels::{BroadcastMessage, RealtimeChannelManager};

pub fn sha1_digest(data: &[u8]) -> [u8; 20] {
    let mut h0: u32 = 0x67452301;
    let mut h1: u32 = 0xEFCDAB89;
    let mut h2: u32 = 0x98BADCFE;
    let mut h3: u32 = 0x10325476;
    let mut h4: u32 = 0xC3D2E1F0;

    let bit_len = (data.len() as u64) * 8;
    let mut msg = data.to_vec();
    msg.push(0x80);
    while (msg.len() % 64) != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.chunks(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let (mut a, mut b, mut c, mut d, mut e) = (h0, h1, h2, h3, h4);
        for (i, &w_val) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(w_val);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }

        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }

    let mut out = [0u8; 20];
    out[0..4].copy_from_slice(&h0.to_be_bytes());
    out[4..8].copy_from_slice(&h1.to_be_bytes());
    out[8..12].copy_from_slice(&h2.to_be_bytes());
    out[12..16].copy_from_slice(&h3.to_be_bytes());
    out[16..20].copy_from_slice(&h4.to_be_bytes());
    out
}

pub fn base64_encode(input: &[u8]) -> String {
    const CHARSET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);

        let n = ((b0 as u32) << 16) | ((b1 as u32) << 8) | (b2 as u32);
        result.push(CHARSET[((n >> 18) & 63) as usize] as char);
        result.push(CHARSET[((n >> 12) & 63) as usize] as char);

        if chunk.len() > 1 {
            result.push(CHARSET[((n >> 6) & 63) as usize] as char);
        } else {
            result.push('=');
        }

        if chunk.len() > 2 {
            result.push(CHARSET[(n & 63) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

pub fn generate_websocket_accept(key: &str) -> String {
    let combined = format!("{}258EAFA5-E914-47DA-95CA-C5AB0DC85B11", key.trim());
    let digest = sha1_digest(combined.as_bytes());
    base64_encode(&digest)
}

pub fn encode_text_frame(text: &str) -> Vec<u8> {
    let payload = text.as_bytes();
    let len = payload.len();
    let mut frame = Vec::new();
    frame.push(0x81); // FIN + Text

    if len < 126 {
        frame.push(len as u8);
    } else if len <= 65535 {
        frame.push(126);
        frame.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        frame.push(127);
        frame.extend_from_slice(&(len as u64).to_be_bytes());
    }
    frame.extend_from_slice(payload);
    frame
}

pub fn decode_frame(buf: &[u8]) -> Option<(u8, Vec<u8>, usize)> {
    if buf.len() < 2 {
        return None;
    }
    let opcode = buf[0] & 0x0F;
    let masked = (buf[1] & 0x80) != 0;
    let mut payload_len = (buf[1] & 0x7F) as usize;
    let mut cursor = 2;

    if payload_len == 126 {
        if buf.len() < cursor + 2 {
            return None;
        }
        payload_len = u16::from_be_bytes([buf[cursor], buf[cursor + 1]]) as usize;
        cursor += 2;
    } else if payload_len == 127 {
        if buf.len() < cursor + 8 {
            return None;
        }
        payload_len = u64::from_be_bytes([
            buf[cursor],
            buf[cursor + 1],
            buf[cursor + 2],
            buf[cursor + 3],
            buf[cursor + 4],
            buf[cursor + 5],
            buf[cursor + 6],
            buf[cursor + 7],
        ]) as usize;
        cursor += 8;
    }

    let mask = if masked {
        if buf.len() < cursor + 4 {
            return None;
        }
        let m = [
            buf[cursor],
            buf[cursor + 1],
            buf[cursor + 2],
            buf[cursor + 3],
        ];
        cursor += 4;
        Some(m)
    } else {
        None
    };

    if buf.len() < cursor + payload_len {
        return None;
    }

    let mut payload = buf[cursor..cursor + payload_len].to_vec();
    if let Some(m) = mask {
        for i in 0..payload.len() {
            payload[i] ^= m[i % 4];
        }
    }

    Some((opcode, payload, cursor + payload_len))
}

pub async fn handle_websocket_session(
    mut socket: TcpStream,
    ws_key: &str,
    realtime_mgr: Arc<RealtimeChannelManager>,
    db: Arc<SharedDatabase>,
    ctx: ExecutionContext,
) -> io::Result<()> {
    let accept = generate_websocket_accept(ws_key);
    let handshake_resp = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
    );
    socket.write_all(handshake_resp.as_bytes()).await?;
    socket.flush().await?;

    let mut raw_buf = Vec::new();
    let mut temp_chunk = [0u8; 4096];

    let mut subscribed_topics: HashMap<String, tokio::sync::broadcast::Receiver<BroadcastMessage>> =
        HashMap::new();
    let mut db_change_rx = db.subscribe();

    loop {
        tokio::select! {
            read_res = socket.read(&mut temp_chunk) => {
                let n = match read_res {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(_) => break,
                };
                raw_buf.extend_from_slice(&temp_chunk[..n]);

                while let Some((opcode, payload, consumed)) = decode_frame(&raw_buf) {
                    raw_buf.drain(..consumed);

                    match opcode {
                        0x8 => {
                            // Close frame
                            let close_resp = [0x88, 0x00];
                            let _ = socket.write_all(&close_resp).await;
                            return Ok(());
                        }
                        0x9 => {
                            // Ping -> Pong (0xA)
                            let mut pong = vec![0x8A, payload.len() as u8];
                            pong.extend_from_slice(&payload);
                            let _ = socket.write_all(&pong).await;
                        }
                        0x1 => {
                            // Text frame (JSON message)
                            if let Ok(text) = std::str::from_utf8(&payload) {
                                if let Ok(v) = serde_json::from_str::<serde_json::Value>(text) {
                                    let topic = v.get("topic").and_then(|s| s.as_str()).unwrap_or("");
                                    let event = v.get("event").and_then(|s| s.as_str()).unwrap_or("");
                                    let req_ref = v.get("ref").and_then(|s| s.as_str()).unwrap_or("");

                                    if event == "phx_join" {
                                        let rx = realtime_mgr.subscribe(topic);
                                        subscribed_topics.insert(topic.to_string(), rx);
                                        let reply = serde_json::json!({
                                            "topic": topic,
                                            "event": "phx_reply",
                                            "payload": { "status": "ok", "response": {} },
                                            "ref": req_ref
                                        });
                                        let frame = encode_text_frame(&reply.to_string());
                                        let _ = socket.write_all(&frame).await;
                                    } else if event == "broadcast" {
                                        let msg_payload = v.get("payload").cloned().unwrap_or(serde_json::Value::Null);
                                        let msg = BroadcastMessage {
                                            channel: topic.to_string(),
                                            event: "broadcast".to_string(),
                                            payload: msg_payload,
                                            sender_id: ctx.user_id,
                                            sender_role: ctx.role.clone().unwrap_or_else(|| "anon".into()),
                                            timestamp: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs(),
                                        };
                                        realtime_mgr.publish(msg);
                                    } else if event == "heartbeat" {
                                        let reply = serde_json::json!({
                                            "topic": "phoenix",
                                            "event": "phx_reply",
                                            "payload": { "status": "ok" },
                                            "ref": req_ref
                                        });
                                        let frame = encode_text_frame(&reply.to_string());
                                        let _ = socket.write_all(&frame).await;
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            db_change = db_change_rx.recv() => {
                if let Ok(evt) = db_change {
                    let msg_json = serde_json::json!({
                        "topic": format!("realtime:{}", evt.table),
                        "event": "postgres_changes",
                        "payload": {
                            "table": evt.table,
                            "action": format!("{:?}", evt.action).to_uppercase(),
                            "new": evt.new_row,
                            "old": evt.old_row,
                            "commit_timestamp": evt.timestamp_ms
                        }
                    });
                    let frame = encode_text_frame(&msg_json.to_string());
                    if socket.write_all(&frame).await.is_err() {
                        break;
                    }
                    let _ = socket.flush().await;
                }
            }
        }
    }

    Ok(())
}
