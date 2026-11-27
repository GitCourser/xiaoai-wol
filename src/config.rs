use std::fs;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub mac: String,
    pub port: u16,
    pub mqtt: MqttConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MqttConfig {
    pub client_id: String,
    pub server: String,
    pub port: u16,
    pub topic: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mac: "00-00-00-FF-FF-FF".to_string(),
            port: 3030,
            mqtt: MqttConfig::default(),
        }
    }
}

impl Default for MqttConfig {
    fn default() -> Self {
        Self {
            client_id: "bemfa_private".to_string(),
            server: "bemfa.com".to_string(),
            port: 9501,
            topic: "title".to_string(),
        }
    }
}

impl Config {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let path_ref = path.as_ref();
        let content = fs::read_to_string(path_ref)
            .map_err(|e| format!("无法打开配置文件 {}: {e}", path_ref.display()))?;
        Self::parse_json(&content)
    }

    pub fn parse_json(input: &str) -> Result<Self, String> {
        let mut parser = JsonParser::new(input);
        let root = parser
            .parse_value()
            .map_err(|e| format!("配置文件解析错误: {e}"))?;
        parser.skip_ws();
        if !parser.is_eof() {
            return Err("配置文件解析错误: 存在多余内容".to_string());
        }

        let JsonValue::Object(entries) = root else {
            return Err("配置文件解析错误: 根节点必须是 JSON 对象".to_string());
        };

        let mut mac: Option<String> = None;
        let mut port: Option<i64> = None;
        let mut mqtt_client_id: Option<String> = None;
        let mut mqtt_server: Option<String> = None;
        let mut mqtt_port: Option<i64> = None;
        let mut mqtt_topic: Option<String> = None;

        for (key, value) in entries {
            match key.as_str() {
                "mac" => mac = Some(expect_string(&key, value)?),
                "port" => port = Some(expect_int(&key, value)?),
                "mqtt" => {
                    let JsonValue::Object(mqtt_entries) = value else {
                        return Err("配置文件解析错误: mqtt 必须是 JSON 对象".to_string());
                    };
                    for (mk, mv) in mqtt_entries {
                        match mk.as_str() {
                            "client_id" => mqtt_client_id = Some(expect_string(&mk, mv)?),
                            "server" => mqtt_server = Some(expect_string(&mk, mv)?),
                            "port" => mqtt_port = Some(expect_int(&mk, mv)?),
                            "topic" => mqtt_topic = Some(expect_string(&mk, mv)?),
                            _ => {}
                        }
                    }
                }
                // 忽略未知字段以保持向后兼容
                _ => {}
            }
        }

        let defaults = Config::default();
        let mac = match mac {
            Some(s) if !s.is_empty() => s,
            _ => defaults.mac,
        };
        let port = match port {
            None | Some(0) => defaults.port,
            Some(p) if (1..=65535).contains(&p) => p as u16,
            Some(_) => return Err("配置验证失败: port 必须在 1-65535 范围内".to_string()),
        };

        let mqtt_defaults = MqttConfig::default();
        let mqtt_client_id = match mqtt_client_id {
            Some(s) if !s.is_empty() => s,
            _ => mqtt_defaults.client_id,
        };
        let mqtt_server = match mqtt_server {
            Some(s) if !s.is_empty() => s,
            _ => mqtt_defaults.server,
        };
        let mqtt_port = match mqtt_port {
            None | Some(0) => mqtt_defaults.port,
            Some(p) if (1..=65535).contains(&p) => p as u16,
            Some(_) => return Err("配置验证失败: MQTT port 必须在 1-65535 范围内".to_string()),
        };
        let mqtt_topic = match mqtt_topic {
            Some(s) if !s.is_empty() => s,
            _ => mqtt_defaults.topic,
        };

        let config = Config {
            mac,
            port,
            mqtt: MqttConfig {
                client_id: mqtt_client_id,
                server: mqtt_server,
                port: mqtt_port,
                topic: mqtt_topic,
            },
        };

        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), String> {
        if self.mac.is_empty() {
            return Err("配置验证失败: mac 地址不能为空".to_string());
        }
        if self.port == 0 {
            return Err("配置验证失败: port 必须在 1-65535 范围内".to_string());
        }
        if self.mqtt.client_id.is_empty() {
            return Err("配置验证失败: MQTT client_id 不能为空".to_string());
        }
        if self.mqtt.server.is_empty() {
            return Err("配置验证失败: MQTT server 不能为空".to_string());
        }
        if self.mqtt.port == 0 {
            return Err("配置验证失败: MQTT port 必须在 1-65535 范围内".to_string());
        }
        if self.mqtt.topic.is_empty() {
            return Err("配置验证失败: MQTT topic 不能为空".to_string());
        }
        Ok(())
    }
}

fn expect_string(field: &str, value: JsonValue) -> Result<String, String> {
    match value {
        JsonValue::String(s) => Ok(s),
        JsonValue::Null => Ok(String::new()),
        _ => Err(format!("配置文件解析错误: 字段 {field} 应为字符串")),
    }
}

fn expect_int(field: &str, value: JsonValue) -> Result<i64, String> {
    match value {
        JsonValue::Int(n) => Ok(n),
        JsonValue::Null => Ok(0),
        _ => Err(format!("配置文件解析错误: 字段 {field} 应为整数")),
    }
}

/// 极简 JSON 字符串转义，供 HTTP 响应序列化使用
pub fn json_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 2);
    out.push('"');
    for ch in input.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                use std::fmt::Write;
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[derive(Debug)]
enum JsonValue {
    Null,
    Bool,
    Int(i64),
    Float,
    String(String),
    Array,
    Object(Vec<(String, JsonValue)>),
}

struct JsonParser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> JsonParser<'a> {
    fn new(input: &'a str) -> Self {
        let bytes = input.as_bytes();
        // 兼容 UTF-8 BOM
        let pos = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
            3
        } else {
            0
        };
        Self { bytes, pos }
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if matches!(b, b' ' | b'\t' | b'\r' | b'\n') {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn parse_value(&mut self) -> Result<JsonValue, String> {
        self.skip_ws();
        let Some(b) = self.peek() else {
            return Err("意外的输入结束".to_string());
        };
        match b {
            b'{' => self.parse_object(),
            b'[' => self.parse_array(),
            b'"' => self.parse_string().map(JsonValue::String),
            b't' => self.expect_lit(b"true", JsonValue::Bool),
            b'f' => self.expect_lit(b"false", JsonValue::Bool),
            b'n' => self.expect_lit(b"null", JsonValue::Null),
            b'-' | b'0'..=b'9' => self.parse_number(),
            _ => Err(format!("非法字符 '{}' (位置 {})", b as char, self.pos)),
        }
    }

    fn expect_lit(&mut self, lit: &[u8], val: JsonValue) -> Result<JsonValue, String> {
        if self.bytes[self.pos..].starts_with(lit) {
            self.pos += lit.len();
            Ok(val)
        } else {
            Err(format!("位置 {} 处字面量无效", self.pos))
        }
    }

    fn parse_object(&mut self) -> Result<JsonValue, String> {
        self.pos += 1; // '{'
        self.skip_ws();
        let mut entries = Vec::new();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(JsonValue::Object(entries));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some(b'"') {
                return Err(format!("位置 {} 处期望对象键字符串", self.pos));
            }
            let key = self.parse_string()?;
            self.skip_ws();
            if self.peek() != Some(b':') {
                return Err(format!("位置 {} 处期望 ':'", self.pos));
            }
            self.pos += 1;
            let value = self.parse_value()?;
            entries.push((key, value));
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(format!("位置 {} 处期望 ',' 或 '}}'", self.pos)),
            }
        }
        Ok(JsonValue::Object(entries))
    }

    fn parse_array(&mut self) -> Result<JsonValue, String> {
        self.pos += 1; // '['
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(JsonValue::Array);
        }
        loop {
            let _ = self.parse_value()?;
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(format!("位置 {} 处期望 ',' 或 ']'", self.pos)),
            }
        }
        Ok(JsonValue::Array)
    }

    fn parse_string(&mut self) -> Result<String, String> {
        self.pos += 1; // '"'
        let mut out = String::new();
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            self.pos += 1;
            match b {
                b'"' => return Ok(out),
                b'\\' => {
                    if self.pos >= self.bytes.len() {
                        return Err("未闭合的转义序列".to_string());
                    }
                    let esc = self.bytes[self.pos];
                    self.pos += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000C}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let code = self.parse_hex4()?;
                            let ch = char::from_u32(code as u32)
                                .ok_or_else(|| "无效的 Unicode 转义".to_string())?;
                            out.push(ch);
                        }
                        _ => return Err(format!("不支持的转义字符 \\{}", esc as char)),
                    }
                }
                b if b < 0x20 => return Err("字符串中存在未转义的控制字符".to_string()),
                _ => {
                    // 解码完整 UTF-8 字符
                    let start = self.pos - 1;
                    let s = std::str::from_utf8(&self.bytes[start..])
                        .map_err(|_| "无效的 UTF-8 编码".to_string())?;
                    let ch = s.chars().next().ok_or_else(|| "意外结束".to_string())?;
                    out.push(ch);
                    self.pos = start + ch.len_utf8();
                }
            }
        }
        Err("未闭合的字符串".to_string())
    }

    fn parse_hex4(&mut self) -> Result<u16, String> {
        if self.pos + 4 > self.bytes.len() {
            return Err("不完整的 \\u 转义".to_string());
        }
        let slice = std::str::from_utf8(&self.bytes[self.pos..self.pos + 4])
            .map_err(|_| "无效的 \\u 转义".to_string())?;
        self.pos += 4;
        u16::from_str_radix(slice, 16).map_err(|_| "无效的十六进制数字".to_string())
    }

    fn parse_number(&mut self) -> Result<JsonValue, String> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        let digit_start = self.pos;
        while let Some(b) = self.peek() {
            if b.is_ascii_digit() {
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.pos == digit_start {
            return Err(format!("位置 {start} 处数字格式无效"));
        }
        let mut is_float = false;
        if self.peek() == Some(b'.') {
            is_float = true;
            self.pos += 1;
            let frac_start = self.pos;
            while let Some(b) = self.peek() {
                if b.is_ascii_digit() {
                    self.pos += 1;
                } else {
                    break;
                }
            }
            if self.pos == frac_start {
                return Err(format!("位置 {start} 处小数格式无效"));
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            is_float = true;
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            let exp_start = self.pos;
            while let Some(b) = self.peek() {
                if b.is_ascii_digit() {
                    self.pos += 1;
                } else {
                    break;
                }
            }
            if self.pos == exp_start {
                return Err(format!("位置 {start} 处指数格式无效"));
            }
        }
        if is_float {
            Ok(JsonValue::Float)
        } else {
            let s = std::str::from_utf8(&self.bytes[start..self.pos])
                .map_err(|_| "数字编码错误".to_string())?;
            let n = s.parse::<i64>().map_err(|_| format!("整数超出范围: {s}"))?;
            Ok(JsonValue::Int(n))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_standard_config_and_ignores_extra_fields() {
        let json = r#"{
            "port": 8080,
            "mac": "AA:BB:CC:DD:EE:FF",
            "legacy_field": "ignored",
            "mqtt": {
                "client_id": "cid123",
                "topic": "pc001",
                "server": "bemfa.com",
                "port": 9501
            }
        }"#;
        let cfg = Config::parse_json(json).expect("should parse");
        assert_eq!(cfg.port, 8080);
        assert_eq!(cfg.mac, "AA:BB:CC:DD:EE:FF");
        assert_eq!(cfg.mqtt.client_id, "cid123");
        assert_eq!(cfg.mqtt.topic, "pc001");
        assert_eq!(cfg.mqtt.server, "bemfa.com");
        assert_eq!(cfg.mqtt.port, 9501);
    }

    #[test]
    fn applies_defaults_for_missing_fields() {
        let cfg = Config::parse_json("{}").expect("empty object uses defaults");
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn rejects_invalid_port() {
        let err = Config::parse_json(r#"{"port": 70000}"#).unwrap_err();
        assert!(err.contains("port"));
        let err2 = Config::parse_json(r#"{"mqtt": {"port": -1}}"#).unwrap_err();
        assert!(err2.contains("MQTT port"));
    }

    #[test]
    fn escapes_json_strings() {
        assert_eq!(json_escape("a\"b\\c\nd"), "\"a\\\"b\\\\c\\nd\"");
    }
}
