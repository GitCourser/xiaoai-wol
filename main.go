package main

import (
	"fmt"
	"log"
	"net/http"
	"os"
	"sync"
)

var (
	debugXiaoai       bool
	lastXiaoaiRequest string
	mu                sync.Mutex
)

type Request struct {
	Request struct {
		Type     int8 `json:"type"`
		Intent   struct {
			Query string `json:"query"`
		} `json:"intent"`
		NoResponse bool `json:"no_response"`
	} `json:"request"`
}

type Response struct {
	Version  string `json:"version"`
	IsEnd    bool   `json:"is_session_end"`
	Response struct {
		NotUnderstand bool `json:"not_understand"`
		OpenMic       bool `json:"open_mic"`
		ToSpeak       struct {
			Type int8   `json:"type"`
			Text string `json:"text"`
		} `json:"to_speak"`
		ToDisplay struct {
			Type int8   `json:"type"`
			Text string `json:"text"`
		} `json:"to_display"`
	} `json:"response"`
}

func main() {
	// 加载配置文件
	config, err := LoadConfig("xiaoai.json")
	if err != nil {
		fmt.Printf("配置加载失败: %v\n", err)
		os.Exit(1)
	}

	// 初始化MQTT管理器
	var mqttManager *MQTTManager
	if config.MQTT.Server != "" && config.MQTT.Topic != "" {
		mqttManager = NewMQTTManager(config.MQTT, config.Mac)

		// 尝试连接MQTT服务器（非阻塞）
		go func() {
			if err := mqttManager.Connect(); err != nil {
				log.Printf("MQTT连接失败: %v", err)
			}
		}()

		log.Printf("MQTT管理器已初始化，服务器: %s:%d, 主题: %s", 
			config.MQTT.Server, config.MQTT.Port, config.MQTT.Topic)
	} else {
		log.Printf("MQTT配置不完整，跳过MQTT功能初始化")
	}

	// 设置HTTP路由处理函数
	setupRoutes(*config, mqttManager)

	// 启动HTTP服务器
	fmt.Printf("小爱接口服务启动，端口: %d\n", config.Port)
	if mqttManager != nil {
		fmt.Printf("MQTT功能已启用，主题: %s\n", config.MQTT.Topic)
	} else {
		fmt.Println("MQTT功能未启用")
	}

	if err := http.ListenAndServe(fmt.Sprintf(":%d", config.Port), nil); err != nil {
		fmt.Printf("HTTP服务器启动失败: %v\n", err)

		// 清理MQTT连接
		if mqttManager != nil {
			mqttManager.Disconnect()
		}

		os.Exit(1)
	}
}

// setupRoutes 设置HTTP路由处理函数
func setupRoutes(config Config, mqttManager *MQTTManager) {
	http.HandleFunc("/", handleRoot)
	http.HandleFunc("/top", handleTop)
	http.HandleFunc("/restart", handleRestart)

	http.HandleFunc("/toggle-debug", func(w http.ResponseWriter, r *http.Request) {
		handleToggleDebug(w, r, &debugXiaoai, &lastXiaoaiRequest, &mu)
	})

	http.HandleFunc("/get-last-request", func(w http.ResponseWriter, r *http.Request) {
		handleGetLastRequest(w, r, &debugXiaoai, &lastXiaoaiRequest, &mu)
	})

	http.HandleFunc("/wol", func(w http.ResponseWriter, r *http.Request) {
		handleWol(w, r, config)
	})

	http.HandleFunc("/xiaoai", func(w http.ResponseWriter, r *http.Request) {
		handleXiaoai(w, r, config, &debugXiaoai, &lastXiaoaiRequest, &mu)
	})

	http.HandleFunc("/mqtt-status", func(w http.ResponseWriter, r *http.Request) {
		handleMQTTStatus(w, r, mqttManager)
	})
}