package main

import (
	"bufio"
	"bytes"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"log"
	"net"
	"net/http"
	"os"
	"os/exec"
	"runtime"
	"strconv"
	"strings"
	"sync"
	"time"
)

// 检查是否为内网IP
func isPrivateIP(ip string) bool {
	// 移除端口号
	if idx := strings.LastIndex(ip, ":"); idx != -1 {
		ip = ip[:idx]
	}

	// 解析IP地址
	parsedIP := net.ParseIP(ip)
	if parsedIP == nil {
		return false
	}

	// 检查是否为私有IP地址
	return parsedIP.IsPrivate() || parsedIP.IsLoopback()
}

func handleRoot(w http.ResponseWriter, r *http.Request) {
	if !isPrivateIP(r.RemoteAddr) {
		w.WriteHeader(http.StatusNotFound)
		return
	}

	html := `<!DOCTYPE html>
<html>
<head>
    <meta charset="UTF-8">
    <title>设备管理</title>
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <style>
        body { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", "Microsoft YaHei", "Helvetica Neue", Helvetica, Arial, sans-serif; margin: 20px; background-color: #f8f9fa; color: #333; }
        h2 { color: #007bff; }
        .container { max-width: 800px; margin: auto; padding: 20px; background-color: #fff; border-radius: 8px; box-shadow: 0 2px 4px rgba(0,0,0,0.1); }
        button { padding: 10px 15px; margin: 5px; font-size: 14px; cursor: pointer; border: 1px solid #ccc; border-radius: 4px; background-color: #f0f0f0; transition: background-color 0.2s; }
        button:hover { background-color: #e0e0e0; }
        #result { margin-top: 20px; padding: 15px; background: #e9ecef; white-space: pre-wrap; word-wrap: break-word; border-radius: 4px; min-height: 50px; }
        .switch-container { display: flex; align-items: center; margin: 20px 5px; padding: 10px; background-color: #f8f9fa; border-radius: 8px;}
        .switch { position: relative; display: inline-block; width: 50px; height: 28px; }
        .switch input { opacity: 0; width: 0; height: 0; }
        .slider { position: absolute; cursor: pointer; top: 0; left: 0; right: 0; bottom: 0; background-color: #ccc; transition: .4s; border-radius: 28px; }
        .slider:before { position: absolute; content: ""; height: 20px; width: 20px; left: 4px; bottom: 4px; background-color: white; transition: .4s; border-radius: 50%; }
        input:checked + .slider { background-color: #28a745; }
        input:checked + .slider:before { transform: translateX(22px); }
        .mqtt-status { margin: 20px 5px; padding: 15px; background-color: #f8f9fa; border-radius: 8px; border-left: 4px solid #007bff; }
        .status-indicator { display: inline-block; width: 12px; height: 12px; border-radius: 50%; margin-right: 8px; }
        .status-connected { background-color: #28a745; }
        .status-disconnected { background-color: #dc3545; }
        .status-disabled { background-color: #6c757d; }
        .mqtt-details { margin-top: 10px; font-size: 14px; color: #666; }
    </style>
</head>
<body>
<div class="container">
    <h2>设备管理面板</h2>
    <button onclick="wakeup()">唤醒电脑</button>
    <button onclick="restart()">重启设备</button>
    <button onclick="checkResource()">系统信息</button>

    <div class="switch-container">
        <label class="switch">
            <input type="checkbox" id="debugSwitch" onchange="toggleDebug()">
            <span class="slider"></span>
        </label>
        <span style="margin-left: 10px;">小爱API调试</span>
        <a href="https://developers.xiaoai.mi.com/skill/create/index" target="_blank" style="margin-left: auto;">小爱技能平台</a>
        <a href="https://cloud.bemfa.com/tcp/devicemqtt.html" target="_blank" style="margin-left: auto;">巴法云MQTT</a>
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
        let debugInterval = null;

        function clearResult() {
            const checkbox = document.getElementById('debugSwitch');
            if (!checkbox.checked) {
                document.getElementById('result').textContent = '';
            }
        }

        function wakeup() {
            clearResult();
            fetch('/wol')
                .then(r => r.text())
                .then(data => document.getElementById('result').textContent = data)
                .catch(e => document.getElementById('result').textContent = '错误: ' + e);
        }

        function restart() {
            if (confirm('确定要重启吗？')) {
                clearResult();
                fetch('/restart')
                    .then(() => document.getElementById('result').textContent = '重启命令已发送')
                    .catch(e => document.getElementById('result').textContent = '错误: ' + e);
            }
        }

        function checkResource() {
            clearResult();
            fetch('/top')
                .then(r => r.text())
                .then(data => document.getElementById('result').textContent = data)
                .catch(e => document.getElementById('result').textContent = '错误: ' + e);
        }

        function toggleDebug() {
            fetch('/toggle-debug')
                .then(r => r.json())
                .then(data => {
                    const checkbox = document.getElementById('debugSwitch');
                    checkbox.checked = data.debug;
                    if (data.debug) {
                        startDebugPolling();
                    } else {
                        stopDebugPolling();
                    }
                });
        }

        function startDebugPolling() {
            if (debugInterval) return;
            document.getElementById('result').textContent = '调试模式已开启，等待 /xiaoai 请求...';
            fetchDebugData(); // Fetch immediately once
            debugInterval = setInterval(fetchDebugData, 2000);
        }

        function stopDebugPolling() {
            if (!debugInterval) return;
            clearInterval(debugInterval);
            debugInterval = null;
            document.getElementById('result').textContent = '调试模式已关闭。';
        }

        function fetchDebugData() {
            const checkbox = document.getElementById('debugSwitch');
            if (!checkbox.checked) {
                 stopDebugPolling();
                 return;
            }
            fetch('/get-last-request')
                .then(r => r.json())
                .then(res => {
                    if (res.debug) {
                        const resultDiv = document.getElementById('result');
                        if (res.data && resultDiv.textContent !== res.data) {
                           resultDiv.textContent = res.data;
                        }
                    } else {
                        checkbox.checked = false;
                        stopDebugPolling();
                    }
                })
                .catch(e => {
                    console.error('Error fetching debug data:', e)
                    stopDebugPolling();
                });
        }
        
        let mqttStatusInterval = null;

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

                    // 更新状态指示器和文本
                    if (!data.enabled) {
                        indicator.className = 'status-indicator status-disabled';
                        statusText.textContent = '未启用';
                        details.style.display = 'none';
                    } else if (data.connected) {
                        indicator.className = 'status-indicator status-connected';
                        statusText.textContent = '已连接';
                        details.style.display = 'block';
                        
                        // 更新详细信息
                        server.textContent = data.server || '-';
                        topic.textContent = data.topic || '-';
                        currentStatus.textContent = data.status || 'off';
                    } else {
                        indicator.className = 'status-indicator status-disconnected';
                        statusText.textContent = '连接断开';
                        details.style.display = 'block';
                        
                        // 更新详细信息
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

        function startMQTTStatusPolling() {
            if (mqttStatusInterval) return;
            updateMQTTStatus(); // 立即获取一次
            mqttStatusInterval = setInterval(updateMQTTStatus, 5000); // 每5秒更新一次
        }

        function stopMQTTStatusPolling() {
            if (mqttStatusInterval) {
                clearInterval(mqttStatusInterval);
                mqttStatusInterval = null;
            }
        }

        window.onload = function() {
            fetch('/get-last-request')
                .then(r => r.json())
                .then(res => {
                    const checkbox = document.getElementById('debugSwitch');
                    checkbox.checked = res.debug;
                    if (res.debug) {
                        startDebugPolling();
                    }
                });
            
            // 启动MQTT状态轮询
            startMQTTStatusPolling();
        };
    </script>
</body>
</html>`

	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	fmt.Fprint(w, html)
}

func handleWol(w http.ResponseWriter, r *http.Request, config Config) {
	if !isPrivateIP(r.RemoteAddr) {
		w.WriteHeader(http.StatusNotFound)
		return
	}

	result := wakeOnLan(config.Mac)
	fmt.Fprint(w, result)
}

func handleXiaoai(w http.ResponseWriter, r *http.Request, config Config, debugXiaoai *bool, lastXiaoaiRequest *string, mu *sync.Mutex) {
	mu.Lock()
	isDebugging := *debugXiaoai
	mu.Unlock()

	if isDebugging {
		defer r.Body.Close()
		bodyBytes, _ := io.ReadAll(r.Body)

		var sb strings.Builder
		sb.WriteString(fmt.Sprintf("Time: %s\n", time.Now().Format("2006-01-02 15:04:05")))
		sb.WriteString(fmt.Sprintf("From: %s\n", r.RemoteAddr))
		sb.WriteString("Headers:\n")
		for name, headers := range r.Header {
			for _, h := range headers {
				sb.WriteString(fmt.Sprintf("- %v: %v\n", name, h))
			}
		}
		sb.WriteString("\nBody:\n")
		var prettyJSON bytes.Buffer
		if err := json.Indent(&prettyJSON, bodyBytes, "", "  "); err == nil {
			sb.Write(prettyJSON.Bytes())
		} else {
			sb.Write(bodyBytes)
		}

		mu.Lock()
		*lastXiaoaiRequest = sb.String()
		mu.Unlock()

		w.Header().Set("Content-Type", "application/json")
		fmt.Fprintln(w, `{"status":"ok", "debug_mode": true}`)
		return
	}

	authHeader := r.Header.Get("Authorization")
	if !strings.HasPrefix(authHeader, fmt.Sprintf("MIAI-HmacSHA256-V1 %s::", config.XiaoaiKey)) {
		w.WriteHeader(http.StatusNotFound)
		return
	}

	var req Request
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		http.Error(w, "无效请求", http.StatusBadRequest)
		return
	}

	resp := Response{
		Version:  "1.0",
		IsEnd:    false,
		Response: struct {
			NotUnderstand bool `json:"not_understand"`
			OpenMic       bool `json:"open_mic"`
			ToSpeak struct {
				Type int8   `json:"type"`
				Text string `json:"text"`
			} `json:"to_speak"`
			ToDisplay struct {
				Type int8   `json:"type"`
				Text string `json:"text"`
			} `json:"to_display"`
		}{
			NotUnderstand: false,
			OpenMic:       true,
			ToSpeak: struct {
				Type int8   `json:"type"`
				Text string `json:"text"`
			}{
				Type: 0,
			},
			ToDisplay: struct {
				Type int8   `json:"type"`
				Text string `json:"text"`
			}{
				Type: 0,
			},
		},
	}

	switch req.Request.Type {
	case 0:
		resp.Response.ToSpeak.Text = "您好主人，我能为您做什么呢？"
		resp.Response.ToDisplay.Text = "您好主人，我能为您做什么呢？"
	case 1:
		if req.Request.NoResponse {
			resp.Response.ToSpeak.Text = "主人，你还在吗？"
			resp.Response.ToDisplay.Text = "主人，你还在吗？"
		} else {
			if req.Request.Intent.Query == "打开我的电脑" {
				wakeOnLan(config.Mac)
				resp.IsEnd = true
				resp.Response.OpenMic = false
				resp.Response.ToSpeak.Text = "好的主人，已经为您打开电脑了"
				resp.Response.ToDisplay.Text = "好的主人，已经为您打开电脑了"
			} else {
				resp.Response.ToSpeak.Text = "对不起主人，我不明白您的意思"
				resp.Response.ToDisplay.Text = "对不起主人，我不明白您的意思"
			}
		}
	case 2:
		resp.IsEnd = true
		resp.Response.OpenMic = false
		resp.Response.ToSpeak.Text = "再见主人，我在这里等你哦!"
		resp.Response.ToDisplay.Text = "再见主人，我在这里等你哦!"
	}

	w.Header().Set("Content-Type", "application/json")
	if err := json.NewEncoder(w).Encode(resp); err != nil {
		fmt.Printf("响应编码失败: %v\n", err)
		http.Error(w, "响应编码失败", http.StatusInternalServerError)
	}
}

func handleToggleDebug(w http.ResponseWriter, r *http.Request, debugXiaoai *bool, lastXiaoaiRequest *string, mu *sync.Mutex) {
	if !isPrivateIP(r.RemoteAddr) {
		w.WriteHeader(http.StatusNotFound)
		return
	}
	mu.Lock()
	*debugXiaoai = !*debugXiaoai
	if !*debugXiaoai {
		*lastXiaoaiRequest = "" // 关闭时清空
	}
	currentDebugStatus := *debugXiaoai
	mu.Unlock()

	w.Header().Set("Content-Type", "application/json")
	fmt.Fprintf(w, `{"debug": %v}`, currentDebugStatus)
}

func handleGetLastRequest(w http.ResponseWriter, r *http.Request, debugXiaoai *bool, lastXiaoaiRequest *string, mu *sync.Mutex) {
	if !isPrivateIP(r.RemoteAddr) {
		w.WriteHeader(http.StatusNotFound)
		return
	}
	mu.Lock()
	defer mu.Unlock()
	response := map[string]interface{}{
		"debug": *debugXiaoai,
		"data":  *lastXiaoaiRequest,
	}
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(response)
}

// handleMQTTStatus 处理MQTT状态查询请求
func handleMQTTStatus(w http.ResponseWriter, r *http.Request, mqttManager *MQTTManager) {
	if !isPrivateIP(r.RemoteAddr) {
		w.WriteHeader(http.StatusNotFound)
		return
	}

	// 如果MQTT管理器未初始化，返回未启用状态
	if mqttManager == nil {
		response := map[string]interface{}{
			"enabled":   false,
			"connected": false,
			"status":    "off",
			"topic":     "",
			"message":   "MQTT功能未启用",
		}
		w.Header().Set("Content-Type", "application/json")
		json.NewEncoder(w).Encode(response)
		return
	}

	// 获取MQTT连接信息和状态
	connectionInfo := mqttManager.GetConnectionInfo()

	// 构建响应数据，符合前端需求
	response := map[string]interface{}{
		"enabled":   true,
		"connected": connectionInfo["connected"],
		"status":    connectionInfo["status"],
		"topic":     connectionInfo["topic"],
		"server":    connectionInfo["server"],
	}

	w.Header().Set("Content-Type", "application/json")
	if err := json.NewEncoder(w).Encode(response); err != nil {
		log.Printf("MQTT状态响应编码失败: %v", err)
		http.Error(w, "响应编码失败", http.StatusInternalServerError)
		return
	}
}

// 获取系统资源信息
func getSystemInfo() string {
	var result strings.Builder

	// 基本系统信息
	result.WriteString("=== 系统资源信息 ===\n")
	result.WriteString(fmt.Sprintf("CPU核心数: %d\n", runtime.NumCPU()))
	result.WriteString(fmt.Sprintf("Goroutine数: %d\n", runtime.NumGoroutine()))
	var m runtime.MemStats
	runtime.ReadMemStats(&m)
	result.WriteString(fmt.Sprintf("已分配内存: %.2f MB\n", float64(m.Alloc)/1024/1024))
	result.WriteString(fmt.Sprintf("系统内存: %.2f MB\n", float64(m.Sys)/1024/1024))
	result.WriteString(fmt.Sprintf("GC次数: %d\n", m.NumGC))

	// Linux特定资源使用情况
	cpuInfo := getLinuxCPUInfo()
	memUsage, memDetails := getLinuxMemInfo()

	if cpuInfo != "" || memUsage != "" {
		result.WriteString("\n=== 资源使用情况 ===\n")
		if cpuInfo != "" {
			result.WriteString(cpuInfo)
		}
		if memUsage != "" {
			result.WriteString(memUsage)
		}
	}

	// Linux特定内存详情
	if memDetails != "" {
		result.WriteString("\n=== 系统内存详情 ===\n")
		result.WriteString(memDetails)
	}

	return result.String()
}

// 获取Linux内存信息
func getLinuxMemInfo() (usageStr, detailsStr string) {
	file, err := os.Open("/proc/meminfo")
	if err != nil {
		return "", ""
	}
	defer file.Close()

	memValues := make(map[string]float64)
	scanner := bufio.NewScanner(file)
	for scanner.Scan() {
		fields := strings.Fields(scanner.Text())
		if len(fields) >= 2 {
			key := fields[0]
			if val, err := strconv.ParseFloat(fields[1], 64); err == nil {
				memValues[key] = val / 1024.0 // 转换为MB
			}
		}
	}

	memTotal, totalOk := memValues["MemTotal:"]
	memAvailable, availableOk := memValues["MemAvailable:"]

	if totalOk && availableOk && memTotal > 0 {
		memUsed := memTotal - memAvailable
		memUsagePercent := (memUsed / memTotal) * 100
		usageStr = fmt.Sprintf("内存使用率: %.2f%%\n", memUsagePercent)
	}

	var detailsBuilder strings.Builder
	if val, ok := memValues["MemTotal:"]; ok {
		detailsBuilder.WriteString(fmt.Sprintf("　总内存: %.2f MB\n", val))
	}
	if val, ok := memValues["MemFree:"]; ok {
		detailsBuilder.WriteString(fmt.Sprintf("空闲内存: %.2f MB\n", val))
	}
	if val, ok := memValues["MemAvailable:"]; ok {
		detailsBuilder.WriteString(fmt.Sprintf("可用内存: %.2f MB\n", val))
	}
	if val, ok := memValues["Buffers:"]; ok {
		detailsBuilder.WriteString(fmt.Sprintf("　缓冲区: %.2f MB\n", val))
	}
	if val, ok := memValues["Cached:"]; ok {
		detailsBuilder.WriteString(fmt.Sprintf("　　缓存: %.2f MB\n", val))
	}

	detailsStr = detailsBuilder.String()
	return
}

// 获取Linux CPU信息
func getLinuxCPUInfo() string {
	// 读取CPU统计信息
	file, err := os.Open("/proc/stat")
	if err != nil {
		return ""
	}
	defer file.Close()

	scanner := bufio.NewScanner(file)
	if !scanner.Scan() {
		return ""
	}

	line := scanner.Text()
	if !strings.HasPrefix(line, "cpu ") {
		return ""
	}

	fields := strings.Fields(line)
	if len(fields) < 8 {
		return ""
	}

	var total, idle int64
	for i := 1; i < len(fields) && i < 8; i++ {
		val, _ := strconv.ParseInt(fields[i], 10, 64)
		total += val
		if i == 4 { // idle time
			idle = val
		}
	}

	if total > 0 {
		usage := float64(total-idle) / float64(total) * 100
		return fmt.Sprintf("CPU使用率: %.2f%%\n", usage)
	}

	return ""
}

func handleTop(w http.ResponseWriter, r *http.Request) {
	if !isPrivateIP(r.RemoteAddr) {
		w.WriteHeader(http.StatusNotFound)
		return
	}

	info := getSystemInfo()
	w.Header().Set("Content-Type", "text/plain; charset=utf-8")
	fmt.Fprint(w, info)
}

func handleRestart(w http.ResponseWriter, r *http.Request) {
	if !isPrivateIP(r.RemoteAddr) {
		w.WriteHeader(http.StatusNotFound)
		return
	}

	// 延迟执行重启命令，先返回响应
	go func() {
		time.Sleep(1 * time.Second)
		// 尝试不同的重启命令
		if err := os.WriteFile("/proc/sys/kernel/sysrq", []byte("1"), 0644); err == nil {
			os.WriteFile("/proc/sysrq-trigger", []byte("b"), 0644)
		} else {
			// 备用方案
			exec.Command("reboot").Run()
		}
	}()

	fmt.Fprint(w, "重启命令已发送，系统将在1秒后重启")
}

func wakeOnLan(macAddr string) string {
	var result strings.Builder
	result.WriteString(fmt.Sprintf("正在唤醒的电脑 MAC 地址: %s\n", macAddr))
	macAddr = strings.ReplaceAll(macAddr, ":", "")
	macAddr = strings.ReplaceAll(macAddr, "-", "")

	if len(macAddr) != 12 {
		return fmt.Sprintln("MAC地址格式无效。它应该是12个十六进制数字。")
	}

	macBytes, err := hex.DecodeString(macAddr)
	if err != nil {
		return fmt.Sprintf("MAC地址解码错误: %v\n", err)
	}

	magicPacket := make([]byte, 6+16*6)
	for i := range magicPacket[:6] {
		magicPacket[i] = 0xff
	}
	for i := 0; i < 16; i++ {
		copy(magicPacket[6+i*6:6+(i+1)*6], macBytes)
	}

	interfaces, err := net.Interfaces()
	if err != nil {
		return fmt.Sprintf("获取网络接口信息失败: %v\n", err)
	}

	sentCount := 0
	sentBroadcasts := make(map[string]bool)

	for _, iface := range interfaces {
		if iface.Flags&net.FlagUp == 0 || iface.Flags&net.FlagBroadcast == 0 || iface.Flags&net.FlagLoopback != 0 || iface.Flags&net.FlagRunning == 0 {
			continue
		}

		addrs, err := iface.Addrs()
		if err != nil {
			continue
		}

		for _, addr := range addrs {
			ipnet, ok := addr.(*net.IPNet)
			if !ok || ipnet.IP.To4() == nil || !ipnet.IP.IsPrivate() {
				continue
			}

			ipv4 := ipnet.IP.To4()
			broadcast := make(net.IP, len(ipv4))
			for i := range ipv4 {
				broadcast[i] = ipv4[i] | ^ipnet.Mask[i]
			}
			broadcastStr := broadcast.String()

			if sentBroadcasts[broadcastStr] {
				continue
			}

			laddr := &net.UDPAddr{IP: ipv4, Port: 0}
			raddr := &net.UDPAddr{IP: broadcast, Port: 9}

			conn, err := net.DialUDP("udp4", laddr, raddr)
			if err != nil {
				continue
			}

			_, err = conn.Write(magicPacket)
			conn.Close()

			if err == nil {
				result.WriteString(fmt.Sprintf("魔术包已发送到 %s (via %s, from %s)\n", broadcastStr, iface.Name, ipv4))
				sentBroadcasts[broadcastStr] = true
				sentCount++
			}
		}
	}

	if sentCount == 0 {
		result.WriteString("没有找到有效的内网接口来发送唤醒包。")
	}

	return result.String()
}