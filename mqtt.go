package main

import (
	"fmt"
	"log"
	"sync"
	"time"

	mqtt "github.com/eclipse/paho.mqtt.golang"
)

// MQTTManager MQTT客户端管理器
type MQTTManager struct {
	client         mqtt.Client
	config         MQTTConfig
	status         string // "on" 或 "off"
	mutex          sync.RWMutex
	connected      bool
	macAddr        string // 用于唤醒功能的MAC地址
	reconnectChan  chan bool
	stopReconnect  chan bool
	maxRetries     int
	retryInterval  time.Duration
	isReconnecting bool
}

// NewMQTTManager 创建新的MQTT管理器实例
func NewMQTTManager(config MQTTConfig, macAddr string) *MQTTManager {
	return &MQTTManager{
		config:         config,
		status:         "off",
		connected:      false,
		macAddr:        macAddr,
		reconnectChan:  make(chan bool, 1),
		stopReconnect:  make(chan bool, 1),
		maxRetries:     3,
		retryInterval:  5 * time.Second,
		isReconnecting: false,
	}
}

// Connect 连接到MQTT服务器，带有重试机制
func (m *MQTTManager) Connect() error {
	return m.connectWithRetry()
}

// connectWithRetry 带重试机制的连接方法
func (m *MQTTManager) connectWithRetry() error {
	var lastErr error
	
	for attempt := 1; attempt <= m.maxRetries; attempt++ {
		log.Printf("MQTT连接尝试 %d/%d", attempt, m.maxRetries)
		
		if err := m.doConnect(); err != nil {
			lastErr = err
			log.Printf("MQTT连接失败 (尝试 %d/%d): %v", attempt, m.maxRetries, err)
			
			if attempt < m.maxRetries {
				log.Printf("等待 %v 后重试...", m.retryInterval)
				time.Sleep(m.retryInterval)
			}
			continue
		}
		
		log.Printf("MQTT连接成功 (尝试 %d/%d)", attempt, m.maxRetries)
		
		// 启动重连监控
		go m.startReconnectMonitor()
		return nil
	}
	
	return fmt.Errorf("MQTT连接失败，已重试 %d 次，最后错误: %v", m.maxRetries, lastErr)
}

// doConnect 执行实际的连接操作
func (m *MQTTManager) doConnect() error {
	// 配置MQTT客户端选项
	opts := mqtt.NewClientOptions()
	opts.AddBroker(fmt.Sprintf("tcp://%s:%d", m.config.Server, m.config.Port))
	opts.SetClientID(m.config.ClientID)
	opts.SetCleanSession(true)
	opts.SetAutoReconnect(false) // 禁用内置自动重连，使用自定义重连逻辑
	opts.SetKeepAlive(60 * time.Second)
	opts.SetPingTimeout(10 * time.Second)
	opts.SetConnectTimeout(10 * time.Second)

	// 设置连接丢失回调
	opts.SetConnectionLostHandler(func(client mqtt.Client, err error) {
		log.Printf("MQTT连接丢失: %v", err)
		m.mutex.Lock()
		m.connected = false
		m.mutex.Unlock()
		
		// 触发重连
		m.triggerReconnect()
	})

	// 设置连接成功回调
	opts.SetOnConnectHandler(func(client mqtt.Client) {
		log.Printf("MQTT连接成功，服务器: %s:%d", m.config.Server, m.config.Port)
		m.mutex.Lock()
		m.connected = true
		m.isReconnecting = false
		m.mutex.Unlock()
		
		// 连接成功后自动订阅主题，带重试
		if err := m.subscribeWithRetry(); err != nil {
			log.Printf("自动订阅失败: %v", err)
		}
	})

	// 创建客户端
	m.client = mqtt.NewClient(opts)

	// 尝试连接
	if token := m.client.Connect(); token.Wait() && token.Error() != nil {
		return fmt.Errorf("MQTT连接失败: %v", token.Error())
	}

	return nil
}

// Subscribe 订阅MQTT主题
func (m *MQTTManager) Subscribe() error {
	if m.client == nil || !m.client.IsConnected() {
		return fmt.Errorf("MQTT客户端未连接")
	}

	// 订阅主题，使用QoS级别1，并设置消息处理回调
	token := m.client.Subscribe(m.config.Topic, 1, m.messageHandler)
	if token.Wait() && token.Error() != nil {
		return fmt.Errorf("订阅主题失败: %v", token.Error())
	}

	log.Printf("成功订阅MQTT主题: %s (QoS: 1)", m.config.Topic)
	return nil
}

// subscribeWithRetry 带重试机制的订阅方法
func (m *MQTTManager) subscribeWithRetry() error {
	var lastErr error
	
	for attempt := 1; attempt <= m.maxRetries; attempt++ {
		if err := m.Subscribe(); err != nil {
			lastErr = err
			log.Printf("订阅失败 (尝试 %d/%d): %v", attempt, m.maxRetries, err)
			
			if attempt < m.maxRetries {
				time.Sleep(m.retryInterval)
			}
			continue
		}
		
		log.Printf("订阅成功 (尝试 %d/%d)", attempt, m.maxRetries)
		return nil
	}
	
	return fmt.Errorf("订阅失败，已重试 %d 次，最后错误: %v", m.maxRetries, lastErr)
}

// Publish 发布消息到MQTT主题，带重试机制
func (m *MQTTManager) Publish(message string) error {
	return m.publishWithRetry(message)
}

// publishWithRetry 带重试机制的发布方法
func (m *MQTTManager) publishWithRetry(message string) error {
	var lastErr error
	
	for attempt := 1; attempt <= m.maxRetries; attempt++ {
		if err := m.doPublish(message); err != nil {
			lastErr = err
			log.Printf("发布消息失败 (尝试 %d/%d): %v", attempt, m.maxRetries, err)
			
			if attempt < m.maxRetries {
				time.Sleep(m.retryInterval)
			}
			continue
		}
		
		log.Printf("发布消息成功 (尝试 %d/%d)", attempt, m.maxRetries)
		return nil
	}
	
	return fmt.Errorf("发布消息失败，已重试 %d 次，最后错误: %v", m.maxRetries, lastErr)
}

// doPublish 执行实际的发布操作
func (m *MQTTManager) doPublish(message string) error {
	if m.client == nil || !m.client.IsConnected() {
		return fmt.Errorf("MQTT客户端未连接")
	}

	// 发布消息，使用QoS级别1
	token := m.client.Publish(m.config.Topic, 1, false, message)
	if token.Wait() && token.Error() != nil {
		return fmt.Errorf("发布消息失败: %v", token.Error())
	}

	log.Printf("成功发布MQTT消息: 主题=%s, 消息=%s", m.config.Topic, message)
	
	// 更新本地状态
	m.SetStatus(message)
	
	return nil
}

// GetStatus 获取当前状态（线程安全）
func (m *MQTTManager) GetStatus() string {
	m.mutex.RLock()
	defer m.mutex.RUnlock()
	return m.status
}

// SetStatus 设置当前状态（线程安全）
func (m *MQTTManager) SetStatus(status string) {
	m.mutex.Lock()
	defer m.mutex.Unlock()
	m.status = status
}

// IsConnected 检查MQTT连接状态（线程安全）
func (m *MQTTManager) IsConnected() bool {
	m.mutex.RLock()
	defer m.mutex.RUnlock()
	return m.connected && m.client != nil && m.client.IsConnected()
}

// GetConnectionInfo 获取连接信息
func (m *MQTTManager) GetConnectionInfo() map[string]interface{} {
	m.mutex.RLock()
	defer m.mutex.RUnlock()
	
	return map[string]interface{}{
		"connected": m.connected && m.client != nil && m.client.IsConnected(),
		"status":    m.status,
		"topic":     m.config.Topic,
		"server":    fmt.Sprintf("%s:%d", m.config.Server, m.config.Port),
		"client_id": m.config.ClientID,
	}
}

// messageHandler MQTT消息处理回调函数
func (m *MQTTManager) messageHandler(client mqtt.Client, msg mqtt.Message) {
	message := string(msg.Payload())
	log.Printf("收到MQTT消息: 主题=%s, 消息=%s", msg.Topic(), message)
	
	// 更新本地状态
	m.SetStatus(message)
	
	// 处理"on"消息 - 执行唤醒功能
	if message == "on" {
		log.Printf("收到唤醒命令，正在执行唤醒操作...")
		
		// 异步执行唤醒操作，避免阻塞MQTT消息处理
		go func() {
			// 调用唤醒函数
			wakeResult := wakeOnLan(m.macAddr)
			log.Printf("\n%s", wakeResult)
			
			// 唤醒后延迟5秒后自动发送"off"消息
			time.Sleep(5 * time.Second)
			if err := m.Publish("off"); err != nil {
				log.Printf("发送off消息失败: %v", err)
			} else {
				log.Printf("唤醒完成，已自动发送off消息")
			}
		}()
	}
}

// triggerReconnect 触发重连
func (m *MQTTManager) triggerReconnect() {
	m.mutex.Lock()
	if m.isReconnecting {
		m.mutex.Unlock()
		return // 已经在重连中，避免重复触发
	}
	m.isReconnecting = true
	m.mutex.Unlock()
	
	// 非阻塞发送重连信号
	select {
	case m.reconnectChan <- true:
		log.Printf("触发MQTT重连")
	default:
		log.Printf("重连信号已在队列中，跳过")
	}
}

// startReconnectMonitor 启动重连监控
func (m *MQTTManager) startReconnectMonitor() {
	log.Printf("启动MQTT重连监控")
	
	for {
		select {
		case <-m.reconnectChan:
			log.Printf("开始MQTT重连...")
			m.handleReconnect()
			
		case <-m.stopReconnect:
			log.Printf("停止MQTT重连监控")
			return
		}
	}
}

// handleReconnect 处理重连逻辑
func (m *MQTTManager) handleReconnect() {
	// 使用指数退避策略进行重连
	backoffInterval := m.retryInterval
	maxBackoff := 60 * time.Second
	
	for attempt := 1; attempt <= m.maxRetries; attempt++ {
		log.Printf("MQTT重连尝试 %d/%d", attempt, m.maxRetries)
		
		// 先断开现有连接
		if m.client != nil && m.client.IsConnected() {
			m.client.Disconnect(250)
		}
		
		// 尝试重新连接
		if err := m.doConnect(); err != nil {
			log.Printf("MQTT重连失败 (尝试 %d/%d): %v", attempt, m.maxRetries, err)
			
			if attempt < m.maxRetries {
				log.Printf("等待 %v 后重试重连...", backoffInterval)
				time.Sleep(backoffInterval)
				
				// 指数退避，但不超过最大值
				backoffInterval *= 2
				if backoffInterval > maxBackoff {
					backoffInterval = maxBackoff
				}
			}
			continue
		}
		
		log.Printf("MQTT重连成功 (尝试 %d/%d)", attempt, m.maxRetries)
		return
	}
	
	log.Printf("MQTT重连失败，已重试 %d 次", m.maxRetries)
	m.mutex.Lock()
	m.isReconnecting = false
	m.mutex.Unlock()
	
	// 重连失败后，等待一段时间再次尝试
	go func() {
		time.Sleep(30 * time.Second)
		log.Printf("重连冷却期结束，准备再次尝试重连")
		m.triggerReconnect()
	}()
}

// Disconnect 断开MQTT连接
func (m *MQTTManager) Disconnect() {
	// 停止重连监控
	select {
	case m.stopReconnect <- true:
	default:
	}
	
	if m.client != nil && m.client.IsConnected() {
		// 取消订阅
		if token := m.client.Unsubscribe(m.config.Topic); token.Wait() && token.Error() != nil {
			log.Printf("取消订阅失败: %v", token.Error())
		}
		
		// 断开连接
		m.client.Disconnect(250)
		log.Printf("MQTT连接已断开")
	}
	
	m.mutex.Lock()
	m.connected = false
	m.isReconnecting = false
	m.mutex.Unlock()
}