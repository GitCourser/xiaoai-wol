mod config;
mod http;
mod mqtt;
mod sysinfo;
mod wol;

use std::net::{Ipv6Addr, SocketAddr, TcpListener};
use std::process;
use std::sync::Arc;

use config::Config;
use mqtt::MqttManager;

const DEFAULT_CONFIG_FILE: &str = "xiaoai.json";

fn main() {
    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| DEFAULT_CONFIG_FILE.to_string());

    let config = match Config::load(&config_path) {
        Ok(cfg) => cfg,
        Err(err) => {
            eprintln!("配置加载失败: {err}");
            process::exit(1);
        }
    };

    let mqtt_manager: Option<Arc<MqttManager>> =
        if !config.mqtt.server.is_empty() && !config.mqtt.topic.is_empty() {
            let manager = MqttManager::new(config.mqtt.clone(), config.mac.clone());
            if let Err(err) = manager.spawn() {
                eprintln!("MQTT 后台线程启动失败: {err}");
            } else {
                eprintln!(
                    "MQTT管理器已初始化，服务器: {}:{}, 主题: {}",
                    config.mqtt.server, config.mqtt.port, config.mqtt.topic
                );
            }
            Some(manager)
        } else {
            eprintln!("MQTT配置不完整，跳过MQTT功能初始化");
            None
        };

    let listener = match bind_listener(config.port) {
        Ok(l) => l,
        Err(err) => {
            eprintln!("HTTP服务器启动失败: {err}");
            if let Some(mgr) = &mqtt_manager {
                mgr.stop();
            }
            process::exit(1);
        }
    };

    println!("设备管理服务启动，端口: {}", config.port);
    if mqtt_manager.is_some() {
        println!("MQTT功能已启用，主题: {}", config.mqtt.topic);
    } else {
        println!("MQTT功能未启用");
    }

    if let Err(err) = http::serve(listener, &config, mqtt_manager.as_ref()) {
        eprintln!("HTTP服务器运行异常: {err}");
        if let Some(mgr) = &mqtt_manager {
            mgr.stop();
        }
        process::exit(1);
    }
}

/// 优先尝试双栈绑定（[::]:port），若系统禁用 IPv6 则回退到 0.0.0.0:port
fn bind_listener(port: u16) -> std::io::Result<TcpListener> {
    let v6_addr = SocketAddr::from((Ipv6Addr::UNSPECIFIED, port));
    TcpListener::bind(v6_addr).or_else(|_| TcpListener::bind(("0.0.0.0", port)))
}
