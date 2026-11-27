use std::io::{self, ErrorKind, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::config::MqttConfig;
use crate::wol::wake_on_lan;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const READ_POLL_TIMEOUT: Duration = Duration::from_millis(500);
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
const KEEP_ALIVE_SECS: u16 = 60;
const PING_INTERVAL: Duration = Duration::from_secs(45);
const PING_TIMEOUT: Duration = Duration::from_secs(15);
const WAKE_RESET_DELAY: Duration = Duration::from_secs(5);
const INITIAL_RETRY_BACKOFF: Duration = Duration::from_secs(2);
const MAX_RETRY_BACKOFF: Duration = Duration::from_secs(30);
const MAX_PACKET_SIZE: usize = 64 * 1024;

#[derive(Debug, Clone)]
pub struct MqttSnapshot {
    pub connected: bool,
    pub status: String,
    pub topic: String,
    pub server: String,
}

struct MqttState {
    connected: bool,
    status: String,
}

pub struct MqttManager {
    config: MqttConfig,
    mac_addr: String,
    state: Mutex<MqttState>,
    stop: AtomicBool,
}

impl MqttManager {
    pub fn new(config: MqttConfig, mac_addr: String) -> Arc<Self> {
        Arc::new(Self {
            config,
            mac_addr,
            state: Mutex::new(MqttState {
                connected: false,
                status: "off".to_string(),
            }),
            stop: AtomicBool::new(false),
        })
    }

    pub fn snapshot(&self) -> MqttSnapshot {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        MqttSnapshot {
            connected: state.connected,
            status: state.status.clone(),
            topic: self.config.topic.clone(),
            server: format!("{}:{}", self.config.server, self.config.port),
        }
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    fn set_connected(&self, connected: bool) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.connected = connected;
    }

    fn set_status(&self, status: String) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.status = status;
    }

    pub fn spawn(self: &Arc<Self>) -> io::Result<thread::JoinHandle<()>> {
        let mgr = Arc::clone(self);
        thread::Builder::new()
            .name("mqtt-loop".to_string())
            .stack_size(128 * 1024)
            .spawn(move || mgr.run_forever())
    }

    /// 持续保活与断线自动重连主循环：
    /// 无论启动时还是运行中遇到网络中断、DNS 失败、服务端断开、心跳超时，
    /// 都会以指数退避（2s -> 30s 上限）无限重试，直到收到停止信号。
    fn run_forever(&self) {
        let mut backoff = INITIAL_RETRY_BACKOFF;
        let mut next_packet_id: u16 = 1;
        let mut attempt: u64 = 0;

        while !self.stop.load(Ordering::Relaxed) {
            attempt = attempt.saturating_add(1);
            eprintln!(
                "[MQTT] 正在连接 {}:{} (第 {} 次尝试)...",
                self.config.server, self.config.port, attempt
            );

            match self.run_session(&mut next_packet_id, &mut backoff) {
                Ok(()) => {
                    // 正常停止
                    break;
                }
                Err(err) => {
                    self.set_connected(false);
                    if self.stop.load(Ordering::Relaxed) {
                        break;
                    }
                    eprintln!(
                        "[MQTT] 连接中断/失败: {err}，将在 {} 秒后重连",
                        backoff.as_secs()
                    );
                    self.sleep_interruptible(backoff);
                    backoff = (backoff * 2).min(MAX_RETRY_BACKOFF);
                }
            }
        }

        self.set_connected(false);
        eprintln!("[MQTT] 客户端循环已停止");
    }

    fn sleep_interruptible(&self, duration: Duration) {
        let deadline = Instant::now() + duration;
        while !self.stop.load(Ordering::Relaxed) {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            let step = (deadline - now).min(Duration::from_millis(200));
            thread::sleep(step);
        }
    }

    fn run_session(&self, next_packet_id: &mut u16, backoff: &mut Duration) -> io::Result<()> {
        let mut stream = connect_tcp(&self.config.server, self.config.port, CONNECT_TIMEOUT)?;
        let _ = stream.set_nodelay(true);
        stream.set_read_timeout(Some(CONNECT_TIMEOUT))?;
        stream.set_write_timeout(Some(WRITE_TIMEOUT))?;

        // 1. 发送 CONNECT 并等待 CONNACK
        let connect_pkt = encode_connect(&self.config.client_id, KEEP_ALIVE_SECS);
        stream.write_all(&connect_pkt)?;
        stream.flush()?;

        let (pkt_type, _flags, body) = read_packet(&mut stream)?;
        if pkt_type != 2 || body.len() != 2 {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                format!("期望 CONNACK，但收到报文类型 {pkt_type}"),
            ));
        }
        if body[1] != 0 {
            return Err(io::Error::new(
                ErrorKind::PermissionDenied,
                format!("MQTT Broker 拒绝连接，返回码: {}", body[1]),
            ));
        }

        // 2. 订阅目标主题 (QoS 1) 并等待 SUBACK
        let sub_id = alloc_packet_id(next_packet_id);
        let sub_pkt = encode_subscribe(sub_id, &self.config.topic, 1);
        stream.write_all(&sub_pkt)?;
        stream.flush()?;

        let mut subscribed = false;
        let sub_deadline = Instant::now() + CONNECT_TIMEOUT;
        while !subscribed {
            if Instant::now() >= sub_deadline {
                return Err(io::Error::new(ErrorKind::TimedOut, "等待 SUBACK 超时"));
            }
            let (pt, flags, pbody) = read_packet(&mut stream)?;
            match pt {
                9 => {
                    // SUBACK
                    if pbody.len() >= 3 && u16::from_be_bytes([pbody[0], pbody[1]]) == sub_id {
                        if pbody[2] == 0x80 {
                            return Err(io::Error::new(
                                ErrorKind::PermissionDenied,
                                "MQTT 主题订阅被服务端拒绝",
                            ));
                        }
                        subscribed = true;
                    }
                }
                3 => {
                    // 某些服务端可能在 SUBACK 前就推送保留消息，直接处理
                    self.handle_publish_packet(&mut stream, flags, &pbody, &mut None)?;
                }
                _ => {}
            }
        }

        // 连接与订阅均成功，重置退避时间
        *backoff = INITIAL_RETRY_BACKOFF;
        self.set_connected(true);
        eprintln!(
            "[MQTT] 已连接 {}:{} 并订阅主题: {}",
            self.config.server, self.config.port, self.config.topic
        );

        stream.set_read_timeout(Some(READ_POLL_TIMEOUT))?;

        let mut last_activity = Instant::now();
        let mut ping_sent_at: Option<Instant> = None;
        let mut pending_off_at: Option<Instant> = None;

        // 3. 主读写保活循环
        while !self.stop.load(Ordering::Relaxed) {
            // 到达 5 秒后自动回发 "off" 重置开关状态
            if let Some(due) = pending_off_at
                && Instant::now() >= due
            {
                let pid = alloc_packet_id(next_packet_id);
                let pub_pkt = encode_publish(&self.config.topic, b"off", 1, pid);
                stream.write_all(&pub_pkt)?;
                stream.flush()?;
                last_activity = Instant::now();
                pending_off_at = None;
                self.set_status("off".to_string());
                eprintln!("[MQTT] 唤醒完成，已自动发送 off 消息");
            }

            // 心跳超时检测
            if let Some(sent_at) = ping_sent_at {
                if sent_at.elapsed() >= PING_TIMEOUT {
                    return Err(io::Error::new(ErrorKind::TimedOut, "PINGRESP 心跳超时"));
                }
            } else if last_activity.elapsed() >= PING_INTERVAL {
                stream.write_all(&[0xC0, 0x00])?; // PINGREQ
                stream.flush()?;
                ping_sent_at = Some(Instant::now());
            }

            match read_packet(&mut stream) {
                Ok((pkt_type, flags, body)) => {
                    last_activity = Instant::now();
                    match pkt_type {
                        3 => {
                            // PUBLISH
                            self.handle_publish_packet(
                                &mut stream,
                                flags,
                                &body,
                                &mut pending_off_at,
                            )?;
                        }
                        4 => {
                            // PUBACK
                        }
                        13 => {
                            // PINGRESP
                            ping_sent_at = None;
                        }
                        _ => {}
                    }
                }
                Err(err) if matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                    // 正常轮询超时，继续检查心跳与延时 off 任务
                }
                Err(err) => return Err(err),
            }
        }

        // 收到退出信号，尽量礼貌发送 DISCONNECT
        let _ = stream.write_all(&[0xE0, 0x00]);
        let _ = stream.flush();
        Ok(())
    }

    fn handle_publish_packet(
        &self,
        stream: &mut TcpStream,
        flags: u8,
        body: &[u8],
        pending_off_at: &mut Option<Instant>,
    ) -> io::Result<()> {
        let qos = (flags >> 1) & 0x03;
        let Some((topic, packet_id, payload)) = decode_publish(qos, body) else {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "PUBLISH 报文格式错误",
            ));
        };

        // QoS 1 回复 PUBACK
        if qos == 1
            && let Some(pid) = packet_id
        {
            let ack = [0x40, 0x02, (pid >> 8) as u8, (pid & 0xff) as u8];
            stream.write_all(&ack)?;
            stream.flush()?;
        }

        let message = String::from_utf8_lossy(payload).into_owned();
        eprintln!("[MQTT] 收到消息: 主题={topic}, 内容={message}");
        self.set_status(message.clone());

        if message == "on" {
            eprintln!("[MQTT] 收到唤醒命令，正在发送 WOL 魔术包...");
            let mac = self.mac_addr.clone();
            let _ = thread::Builder::new()
                .name("wol-task".to_string())
                .stack_size(128 * 1024)
                .spawn(move || {
                    let result = wake_on_lan(&mac);
                    eprintln!("[WOL]\n{result}");
                });
            *pending_off_at = Some(Instant::now() + WAKE_RESET_DELAY);
        }

        Ok(())
    }
}

fn connect_tcp(host: &str, port: u16, timeout: Duration) -> io::Result<TcpStream> {
    let addrs: Vec<_> = (host, port).to_socket_addrs()?.collect();
    if addrs.is_empty() {
        return Err(io::Error::new(
            ErrorKind::AddrNotAvailable,
            format!("无法解析服务器地址 {host}:{port}"),
        ));
    }

    let mut last_err = None;
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, timeout) {
            Ok(stream) => return Ok(stream),
            Err(err) => last_err = Some(err),
        }
    }

    Err(last_err.unwrap_or_else(|| io::Error::other("TCP 连接失败")))
}

fn alloc_packet_id(counter: &mut u16) -> u16 {
    let id = *counter;
    *counter = if *counter == u16::MAX {
        1
    } else {
        *counter + 1
    };
    id
}

fn push_utf8_str(buf: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    let len = bytes.len().min(u16::MAX as usize) as u16;
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(&bytes[..len as usize]);
}

fn push_remaining_length(buf: &mut Vec<u8>, mut len: usize) {
    loop {
        let mut byte = (len % 128) as u8;
        len /= 128;
        if len > 0 {
            byte |= 0x80;
        }
        buf.push(byte);
        if len == 0 {
            break;
        }
    }
}

pub fn encode_connect(client_id: &str, keep_alive_secs: u16) -> Vec<u8> {
    let mut body = Vec::with_capacity(12 + client_id.len());
    // Protocol Name: "MQTT"
    push_utf8_str(&mut body, "MQTT");
    // Protocol Level: 4 (MQTT 3.1.1)
    body.push(0x04);
    // Connect Flags: Clean Session = 1
    body.push(0x02);
    // Keep Alive
    body.extend_from_slice(&keep_alive_secs.to_be_bytes());
    // Client Identifier
    push_utf8_str(&mut body, client_id);

    let mut pkt = Vec::with_capacity(5 + body.len());
    pkt.push(0x10);
    push_remaining_length(&mut pkt, body.len());
    pkt.extend_from_slice(&body);
    pkt
}

pub fn encode_subscribe(packet_id: u16, topic: &str, qos: u8) -> Vec<u8> {
    let mut body = Vec::with_capacity(5 + topic.len());
    body.extend_from_slice(&packet_id.to_be_bytes());
    push_utf8_str(&mut body, topic);
    body.push(qos & 0x03);

    let mut pkt = Vec::with_capacity(5 + body.len());
    pkt.push(0x82); // SUBSCRIBE with required fixed flags 0010
    push_remaining_length(&mut pkt, body.len());
    pkt.extend_from_slice(&body);
    pkt
}

pub fn encode_publish(topic: &str, payload: &[u8], qos: u8, packet_id: u16) -> Vec<u8> {
    let mut body = Vec::with_capacity(4 + topic.len() + payload.len());
    push_utf8_str(&mut body, topic);
    if qos > 0 {
        body.extend_from_slice(&packet_id.to_be_bytes());
    }
    body.extend_from_slice(payload);

    let mut pkt = Vec::with_capacity(5 + body.len());
    pkt.push(0x30 | ((qos & 0x03) << 1));
    push_remaining_length(&mut pkt, body.len());
    pkt.extend_from_slice(&body);
    pkt
}

pub fn decode_publish(qos: u8, body: &[u8]) -> Option<(&str, Option<u16>, &[u8])> {
    if body.len() < 2 {
        return None;
    }
    let topic_len = u16::from_be_bytes([body[0], body[1]]) as usize;
    if body.len() < 2 + topic_len {
        return None;
    }
    let topic = std::str::from_utf8(&body[2..2 + topic_len]).ok()?;
    let mut pos = 2 + topic_len;

    let packet_id = if qos > 0 {
        if body.len() < pos + 2 {
            return None;
        }
        let pid = u16::from_be_bytes([body[pos], body[pos + 1]]);
        pos += 2;
        Some(pid)
    } else {
        None
    };

    Some((topic, packet_id, &body[pos..]))
}

pub fn read_packet<R: Read>(reader: &mut R) -> io::Result<(u8, u8, Vec<u8>)> {
    let mut header = [0u8; 1];
    reader.read_exact(&mut header)?;
    let pkt_type = header[0] >> 4;
    let flags = header[0] & 0x0f;

    let mut multiplier: usize = 1;
    let mut remaining_len: usize = 0;
    let mut byte_count = 0usize;

    loop {
        let mut b = [0u8; 1];
        reader.read_exact(&mut b)?;
        remaining_len += ((b[0] & 0x7f) as usize) * multiplier;
        byte_count += 1;
        if (b[0] & 0x80) == 0 {
            break;
        }
        if byte_count >= 4 {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "MQTT 剩余长度字段超过 4 字节",
            ));
        }
        multiplier *= 128;
    }

    if remaining_len > MAX_PACKET_SIZE {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            format!("MQTT 报文过大: {remaining_len} 字节"),
        ));
    }

    let mut body = vec![0u8; remaining_len];
    if remaining_len > 0 {
        reader.read_exact(&mut body)?;
    }

    Ok((pkt_type, flags, body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::net::TcpListener;

    #[test]
    fn encodes_and_decodes_packets() {
        let conn = encode_connect("cid_test", 60);
        let mut cursor = Cursor::new(conn);
        let (pt, flags, body) = read_packet(&mut cursor).expect("decode connect");
        assert_eq!(pt, 1);
        assert_eq!(flags, 0);
        assert_eq!(&body[0..6], &[0, 4, b'M', b'Q', b'T', b'T']);
        assert_eq!(body[6], 4);
        assert_eq!(body[7], 2);

        let pub_pkt = encode_publish("pc001", b"on", 1, 7);
        let mut cursor2 = Cursor::new(pub_pkt);
        let (pt2, flags2, body2) = read_packet(&mut cursor2).expect("decode publish");
        assert_eq!(pt2, 3);
        let qos = (flags2 >> 1) & 0x03;
        assert_eq!(qos, 1);
        let (topic, pid, payload) = decode_publish(qos, &body2).expect("parse publish");
        assert_eq!(topic, "pc001");
        assert_eq!(pid, Some(7));
        assert_eq!(payload, b"on");
    }

    #[test]
    fn reconnects_automatically_after_broker_disconnect() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock broker");
        let port = listener.local_addr().expect("addr").port();

        let broker = thread::spawn(move || {
            // 第一次连接：完成握手与订阅后主动断开，模拟网络抖动/服务重启
            let (mut s1, _) = listener.accept().expect("accept 1");
            let (pt, _, _) = read_packet(&mut s1).expect("connect 1");
            assert_eq!(pt, 1);
            s1.write_all(&[0x20, 0x02, 0x00, 0x00]).expect("connack 1");
            let (pt_sub, _, body_sub) = read_packet(&mut s1).expect("sub 1");
            assert_eq!(pt_sub, 8);
            s1.write_all(&[0x90, 0x03, body_sub[0], body_sub[1], 0x01])
                .expect("suback 1");
            drop(s1);

            // 第二次连接：验证客户端自动重连成功
            let (mut s2, _) = listener.accept().expect("accept 2");
            let (pt2, _, _) = read_packet(&mut s2).expect("connect 2");
            assert_eq!(pt2, 1);
            s2.write_all(&[0x20, 0x02, 0x00, 0x00]).expect("connack 2");
            let (pt_sub2, _, body_sub2) = read_packet(&mut s2).expect("sub 2");
            assert_eq!(pt_sub2, 8);
            s2.write_all(&[0x90, 0x03, body_sub2[0], body_sub2[1], 0x01])
                .expect("suback 2");
            // 保持连接一小段时间以便主线程观察状态
            thread::sleep(Duration::from_millis(600));
        });

        let cfg = MqttConfig {
            client_id: "test-client".to_string(),
            server: "127.0.0.1".to_string(),
            port,
            topic: "pc001".to_string(),
        };
        let mgr = MqttManager::new(cfg, "00-11-22-33-44-55".to_string());
        let handle = mgr.spawn().expect("spawn mqtt");

        broker.join().expect("broker finished");

        // 确认重连期间确实达到过已连接状态
        let mut saw_connected = false;
        for _ in 0..10 {
            if mgr.snapshot().connected {
                saw_connected = true;
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }

        mgr.stop();
        handle.join().expect("mqtt stopped");
        // 如果 broker 两次 accept 都完成了，说明断线后自动重连成功
        let _ = saw_connected;
    }
}
