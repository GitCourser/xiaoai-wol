package main

import (
	"encoding/json"
	"fmt"
	"os"
)

// Config 主配置结构体，扩展了原有配置以支持MQTT
type Config struct {
	XiaoaiKey string     `json:"xiaoai_key"`
	Mac       string     `json:"mac"`
	Port      int        `json:"port"`
	MQTT      MQTTConfig `json:"mqtt"`
}

// MQTTConfig MQTT配置结构体
type MQTTConfig struct {
	ClientID string `json:"client_id"`
	Server   string `json:"server"`
	Port     int    `json:"port"`
	Topic    string `json:"topic"`
}

// LoadConfig 从指定文件加载配置
func LoadConfig(filename string) (*Config, error) {
	file, err := os.Open(filename)
	if err != nil {
		return nil, fmt.Errorf("无法打开配置文件 %s: %v", filename, err)
	}
	defer file.Close()

	var config Config
	if err := json.NewDecoder(file).Decode(&config); err != nil {
		return nil, fmt.Errorf("配置文件解析错误: %v", err)
	}

	// 设置配置的默认值
	setDefaults(&config)

	// 验证配置
	if err := validateConfig(&config); err != nil {
		return nil, fmt.Errorf("配置验证失败: %v", err)
	}

	return &config, nil
}

// setDefaults 为所有配置设置默认值
func setDefaults(config *Config) {
	// 设置基本配置的默认值
	if config.XiaoaiKey == "" {
		config.XiaoaiKey = "xiaoaikey"
	}
	if config.Mac == "" {
		config.Mac = "00-00-00-FF-FF-FF"
	}
	if config.Port == 0 {
		config.Port = 3030
	}

	// 设置MQTT配置的默认值
	if config.MQTT.ClientID == "" {
		config.MQTT.ClientID = "bemfa_private"
	}
	if config.MQTT.Server == "" {
		config.MQTT.Server = "bemfa.com"
	}
	if config.MQTT.Port == 0 {
		config.MQTT.Port = 9501
	}
	if config.MQTT.Topic == "" {
		config.MQTT.Topic = "title"
	}
}

// validateConfig 验证配置的有效性
func validateConfig(config *Config) error {
	// 验证基本配置
	if config.XiaoaiKey == "" {
		return fmt.Errorf("xiaoai_key 不能为空")
	}
	if config.Mac == "" {
		return fmt.Errorf("mac 地址不能为空")
	}
	if config.Port <= 0 || config.Port > 65535 {
		return fmt.Errorf("port 必须在 1-65535 范围内")
	}

	// 验证MQTT配置
	if config.MQTT.ClientID == "" {
		return fmt.Errorf("MQTT client_id 不能为空")
	}
	if config.MQTT.Server == "" {
		return fmt.Errorf("MQTT server 不能为空")
	}
	if config.MQTT.Port <= 0 || config.MQTT.Port > 65535 {
		return fmt.Errorf("MQTT port 必须在 1-65535 范围内")
	}
	if config.MQTT.Topic == "" {
		return fmt.Errorf("MQTT topic 不能为空")
	}

	return nil
}