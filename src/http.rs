use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv6Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use crate::config::{Config, json_escape};
use crate::mqtt::MqttManager;
use crate::sysinfo::{get_system_info, trigger_reboot_async};
use crate::wol::wake_on_lan;

const HTTP_READ_TIMEOUT: Duration = Duration::from_secs(5);
const HTTP_WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_REQ_BYTES: usize = 8192;

const INDEX_HTML: &str = r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="UTF-8">
    <title>设备管理</title>
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <meta name="color-scheme" content="light dark">
    <style>
        :root {
            color-scheme: light dark;
            --bg: #f8f9fa;
            --card-bg: #fff;
            --card-border: transparent;
            --shadow: 0 2px 4px rgba(0, 0, 0, 0.1);
            --text: #333;
            --text-muted: #666;
            --accent: #007bff;
            --btn-bg: #f0f0f0;
            --btn-hover: #e0e0e0;
            --btn-border: #ccc;
            --btn-text: #333;
            --result-bg: #e9ecef;
            --panel-border: transparent;
            --status-bg: #f8f9fa;
            --status-connected: #28a745;
            --status-disconnected: #dc3545;
            --status-disabled: #6c757d;
        }
        @media (prefers-color-scheme: dark) {
            :root {
                --bg: #12151a;
                --card-bg: #1b2028;
                --card-border: #2a313c;
                --shadow: 0 2px 8px rgba(0, 0, 0, 0.45);
                --text: #e6edf3;
                --text-muted: #9aa4b2;
                --accent: #4da3ff;
                --btn-bg: #252c37;
                --btn-hover: #303947;
                --btn-border: #3a4454;
                --btn-text: #e6edf3;
                --result-bg: #14181f;
                --panel-border: #2a313c;
                --status-bg: #222832;
                --status-connected: #3fb950;
                --status-disconnected: #f85149;
                --status-disabled: #8b949e;
            }
        }
        body { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", "Microsoft YaHei", "Helvetica Neue", Helvetica, Arial, sans-serif; margin: 20px; background-color: var(--bg); color: var(--text); }
        h2 { color: var(--accent); margin-top: 0; }
        .container { max-width: 800px; margin: auto; padding: 20px; background-color: var(--card-bg); border: 1px solid var(--card-border); border-radius: 8px; box-shadow: var(--shadow); }
        .toolbar { display: flex; flex-wrap: wrap; align-items: center; gap: 10px; }
        button { padding: 10px 15px; font-size: 14px; cursor: pointer; color: var(--btn-text); border: 1px solid var(--btn-border); border-radius: 4px; background-color: var(--btn-bg); transition: background-color 0.2s, border-color 0.2s; }
        button:hover { background-color: var(--btn-hover); }
        .console-link { margin-left: auto; font-size: 14px; color: var(--accent); text-decoration: none; }
        .console-link:hover { text-decoration: underline; }
        #result { margin-top: 20px; padding: 15px; background: var(--result-bg); border: 1px solid var(--panel-border); white-space: pre-wrap; word-wrap: break-word; border-radius: 4px; min-height: 50px; }
        .mqtt-status { margin-top: 20px; padding: 15px; background-color: var(--status-bg); border-radius: 8px; border-left: 4px solid var(--accent); }
        .status-indicator { display: inline-block; width: 12px; height: 12px; border-radius: 50%; margin-right: 8px; }
        .status-connected { background-color: var(--status-connected); }
        .status-disconnected { background-color: var(--status-disconnected); }
        .status-disabled { background-color: var(--status-disabled); }
        .mqtt-details { margin-top: 10px; font-size: 14px; color: var(--text-muted); line-height: 1.6; }
    </style>
</head>
<body>
<div class="container">
    <h2>设备管理面板</h2>
    <div class="toolbar">
        <button onclick="wakeup()">唤醒电脑</button>
        <button onclick="restart()">重启设备</button>
        <button onclick="checkResource()">系统信息</button>
        <a class="console-link" href="https://cloud.bemfa.com/tcp/devicemqtt.html" target="_blank" rel="noopener">巴法云MQTT</a>
    </div>

    <div class="mqtt-status" id="mqttStatus">
        <div style="display: flex; align-items: center;">
            <span class="status-indicator status-disabled" id="mqttIndicator"></span>
            <strong>MQTT状态：</strong>
            <span id="mqttStatusText">检查中...</span>
        </div>
        <div class="mqtt-details" id="mqttDetails" style="display: none;">
            <div>服务器: <span id="mqttServer">-</span></div>
            <div>主　题: <span id="mqttTopic">-</span></div>
            <div>状　态: <span id="mqttCurrentStatus">-</span></div>
        </div>
    </div>

    <div id="result"></div>
</div>
<script>
    function wakeup() {
        fetch('/wol')
            .then(r => r.text())
            .then(data => document.getElementById('result').textContent = data)
            .catch(e => document.getElementById('result').textContent = '错误: ' + e);
    }

    function restart() {
        if (confirm('确定要重启吗？')) {
            fetch('/restart')
                .then(r => r.text())
                .then(data => document.getElementById('result').textContent = data || '重启命令已发送')
                .catch(e => document.getElementById('result').textContent = '错误: ' + e);
        }
    }

    function checkResource() {
        fetch('/top')
            .then(r => r.text())
            .then(data => document.getElementById('result').textContent = data)
            .catch(e => document.getElementById('result').textContent = '错误: ' + e);
    }

    function updateMQTTStatus() {
        fetch('/mqtt-status')
            .then(r => r.json())
            .then(data => {
                const indicator = document.getElementById('mqttIndicator');
                const statusText = document.getElementById('mqttStatusText');
                const details = document.getElementById('mqttDetails');
                const server = document.getElementById('mqttServer');
                const topic = document.getElementById('mqttTopic');
                const currentStatus = document.getElementById('mqttCurrentStatus');

                if (!data.enabled) {
                    indicator.className = 'status-indicator status-disabled';
                    statusText.textContent = '未启用';
                    details.style.display = 'none';
                } else if (data.connected) {
                    indicator.className = 'status-indicator status-connected';
                    statusText.textContent = '已连接';
                    details.style.display = 'block';
                    server.textContent = data.server || '-';
                    topic.textContent = data.topic || '-';
                    currentStatus.textContent = data.status || 'off';
                } else {
                    indicator.className = 'status-indicator status-disconnected';
                    statusText.textContent = '连接断开（自动重连中）';
                    details.style.display = 'block';
                    server.textContent = data.server || '-';
                    topic.textContent = data.topic || '-';
                    currentStatus.textContent = data.status || 'off';
                }
            })
            .catch(e => {
                console.error('获取MQTT状态失败:', e);
                const indicator = document.getElementById('mqttIndicator');
                const statusText = document.getElementById('mqttStatusText');
                indicator.className = 'status-indicator status-disabled';
                statusText.textContent = '状态获取失败';
            });
    }

    window.onload = function() {
        updateMQTTStatus();
        setInterval(updateMQTTStatus, 5000);
    };
</script>
</body>
</html>"#;

pub fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_loopback() || v4.is_link_local(),
        IpAddr::V6(v6) => is_private_ipv6(v6),
    }
}

fn is_private_ipv6(v6: Ipv6Addr) -> bool {
    if v6.is_loopback() {
        return true;
    }
    if let Some(v4) = v6.to_ipv4_mapped() {
        return v4.is_private() || v4.is_loopback() || v4.is_link_local();
    }
    let seg0 = v6.segments()[0];
    // Unique local (fc00::/7) or link-local (fe80::/10)
    (seg0 & 0xfe00) == 0xfc00 || (seg0 & 0xffc0) == 0xfe80
}

pub fn serve(
    listener: TcpListener,
    config: &Config,
    mqtt: Option<&Arc<MqttManager>>,
) -> io::Result<()> {
    for incoming in listener.incoming() {
        let Ok(mut stream) = incoming else {
            continue;
        };
        let Ok(peer) = stream.peer_addr() else {
            continue;
        };
        let _ = handle_connection(&mut stream, peer, config, mqtt);
    }
    Ok(())
}

pub fn handle_connection(
    stream: &mut TcpStream,
    peer: SocketAddr,
    config: &Config,
    mqtt: Option<&Arc<MqttManager>>,
) -> io::Result<()> {
    let _ = stream.set_read_timeout(Some(HTTP_READ_TIMEOUT));
    let _ = stream.set_write_timeout(Some(HTTP_WRITE_TIMEOUT));

    if !is_private_ip(peer.ip()) {
        return write_response(stream, "404 Not Found", "text/plain; charset=utf-8", "");
    }

    let Some(path) = read_request_path(stream)? else {
        return Ok(());
    };

    match path.as_str() {
        "/" => write_response(stream, "200 OK", "text/html; charset=utf-8", INDEX_HTML),
        "/wol" => {
            let body = wake_on_lan(&config.mac);
            write_response(stream, "200 OK", "text/plain; charset=utf-8", &body)
        }
        "/top" => {
            let body = get_system_info();
            write_response(stream, "200 OK", "text/plain; charset=utf-8", &body)
        }
        "/restart" => {
            trigger_reboot_async();
            write_response(
                stream,
                "200 OK",
                "text/plain; charset=utf-8",
                "重启命令已发送，系统将在1秒后重启",
            )
        }
        "/mqtt-status" => {
            let body = build_mqtt_status_json(mqtt);
            write_response(stream, "200 OK", "application/json; charset=utf-8", &body)
        }
        _ => write_response(
            stream,
            "404 Not Found",
            "text/plain; charset=utf-8",
            "Not Found",
        ),
    }
}

fn read_request_path(stream: &mut TcpStream) -> io::Result<Option<String>> {
    let mut buf = [0u8; 1024];
    let mut data = Vec::with_capacity(512);

    while data.len() < MAX_REQ_BYTES {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                data.extend_from_slice(&buf[..n]);
                if data.windows(4).any(|w| w == b"\r\n\r\n")
                    || data.windows(2).any(|w| w == b"\n\n")
                {
                    break;
                }
            }
            Err(err)
                if matches!(
                    err.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                break;
            }
            Err(err) => return Err(err),
        }
    }

    if data.is_empty() {
        return Ok(None);
    }

    let req = String::from_utf8_lossy(&data);
    let Some(first_line) = req.lines().next() else {
        return Ok(None);
    };
    let mut parts = first_line.split_whitespace();
    let _method = parts.next();
    let Some(target) = parts.next() else {
        return Ok(None);
    };
    let path = target.split('?').next().unwrap_or("/");
    Ok(Some(path.to_string()))
}

fn build_mqtt_status_json(mqtt: Option<&Arc<MqttManager>>) -> String {
    match mqtt {
        None => {
            r#"{"enabled":false,"connected":false,"status":"off","topic":"","message":"MQTT功能未启用"}"#
                .to_string()
        }
        Some(mgr) => {
            let snap = mgr.snapshot();
            format!(
                r#"{{"enabled":true,"connected":{},"status":{},"topic":{},"server":{}}}"#,
                snap.connected,
                json_escape(&snap.status),
                json_escape(&snap.topic),
                json_escape(&snap.server),
            )
        }
    }
}

fn write_response(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &str,
) -> io::Result<()> {
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes())?;
    if !body.is_empty() {
        stream.write_all(body.as_bytes())?;
    }
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn checks_private_ips() {
        assert!(is_private_ip(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))));
        assert!(is_private_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10))));
        assert!(is_private_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2))));
        assert!(is_private_ip(IpAddr::V4(Ipv4Addr::new(172, 16, 0, 5))));
        assert!(!is_private_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
        assert!(is_private_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)));
        assert!(is_private_ip(IpAddr::V6(
            "::ffff:192.168.1.2".parse().unwrap()
        )));
        assert!(!is_private_ip(IpAddr::V6(
            "2001:4860:4860::8888".parse().unwrap()
        )));
    }

    #[test]
    fn mqtt_status_json_is_valid() {
        let disabled = build_mqtt_status_json(None);
        assert!(disabled.contains(r#""enabled":false"#));
    }
}
