use std::fmt::Write;
use std::fs;
use std::process::Command;
use std::thread;
use std::time::Duration;

pub fn get_system_info() -> String {
    let mut result = String::with_capacity(512);
    result.push_str("=== 系统资源信息 ===\n");

    let cpus = thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let _ = writeln!(result, "CPU核心数: {cpus}");

    if let Some((rss_kb, threads)) = get_self_proc_status() {
        let _ = writeln!(result, "当前进程线程数: {threads}");
        let _ = writeln!(
            result,
            "当前进程内存(RSS): {:.2} MB",
            rss_kb as f64 / 1024.0
        );
    }

    let cpu_info = get_linux_cpu_info();
    let (mem_usage, mem_details) = get_linux_mem_info();

    if !cpu_info.is_empty() || !mem_usage.is_empty() {
        result.push_str("\n=== 资源使用情况 ===\n");
        result.push_str(&cpu_info);
        result.push_str(&mem_usage);
    }

    if !mem_details.is_empty() {
        result.push_str("\n=== 系统内存详情 ===\n");
        result.push_str(&mem_details);
    }

    result
}

fn get_self_proc_status() -> Option<(u64, u64)> {
    let content = fs::read_to_string("/proc/self/status").ok()?;
    let mut rss_kb = None;
    let mut threads = None;

    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            rss_kb = rest
                .split_whitespace()
                .next()
                .and_then(|v| v.parse::<u64>().ok());
        } else if let Some(rest) = line.strip_prefix("Threads:") {
            threads = rest
                .split_whitespace()
                .next()
                .and_then(|v| v.parse::<u64>().ok());
        }
    }

    Some((rss_kb?, threads.unwrap_or(1)))
}

fn get_linux_mem_info() -> (String, String) {
    let Ok(content) = fs::read_to_string("/proc/meminfo") else {
        return (String::new(), String::new());
    };

    let mut mem_total = None;
    let mut mem_free = None;
    let mut mem_available = None;
    let mut buffers = None;
    let mut cached = None;

    for line in content.lines() {
        let mut parts = line.split_whitespace();
        let Some(key) = parts.next() else { continue };
        let Some(val_str) = parts.next() else {
            continue;
        };
        let Ok(val_kb) = val_str.parse::<f64>() else {
            continue;
        };
        let mb = val_kb / 1024.0;
        match key {
            "MemTotal:" => mem_total = Some(mb),
            "MemFree:" => mem_free = Some(mb),
            "MemAvailable:" => mem_available = Some(mb),
            "Buffers:" => buffers = Some(mb),
            "Cached:" => cached = Some(mb),
            _ => {}
        }
    }

    let mut usage = String::new();
    if let (Some(total), Some(avail)) = (mem_total, mem_available)
        && total > 0.0
    {
        let used_pct = ((total - avail) / total) * 100.0;
        let _ = writeln!(usage, "内存使用率: {used_pct:.2}%");
    }

    let mut details = String::new();
    if let Some(v) = mem_total {
        let _ = writeln!(details, "　总内存: {v:.2} MB");
    }
    if let Some(v) = mem_free {
        let _ = writeln!(details, "空闲内存: {v:.2} MB");
    }
    if let Some(v) = mem_available {
        let _ = writeln!(details, "可用内存: {v:.2} MB");
    }
    if let Some(v) = buffers {
        let _ = writeln!(details, "　缓冲区: {v:.2} MB");
    }
    if let Some(v) = cached {
        let _ = writeln!(details, "　　缓存: {v:.2} MB");
    }

    (usage, details)
}

fn get_linux_cpu_info() -> String {
    let Ok(content) = fs::read_to_string("/proc/stat") else {
        return String::new();
    };
    let Some(first_line) = content.lines().next() else {
        return String::new();
    };
    if !first_line.starts_with("cpu ") {
        return String::new();
    }

    let fields: Vec<&str> = first_line.split_whitespace().collect();
    if fields.len() < 8 {
        return String::new();
    }

    let mut total: u64 = 0;
    let mut idle: u64 = 0;
    for (idx, item) in fields.iter().enumerate().take(8).skip(1) {
        let val = item.parse::<u64>().unwrap_or(0);
        total = total.saturating_add(val);
        if idx == 4 {
            idle = val;
        }
    }

    if total > 0 {
        let usage = (total.saturating_sub(idle) as f64 / total as f64) * 100.0;
        return format!("CPU使用率: {usage:.2}%\n");
    }

    String::new()
}

pub fn trigger_reboot_async() {
    let _ = thread::Builder::new()
        .name("reboot".to_string())
        .stack_size(128 * 1024)
        .spawn(|| {
            thread::sleep(Duration::from_secs(1));
            if fs::write("/proc/sys/kernel/sysrq", b"1").is_ok() {
                let _ = fs::write("/proc/sysrq-trigger", b"b");
            } else {
                let _ = Command::new("reboot").status();
            }
        });
}
