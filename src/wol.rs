use std::fmt::Write;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};

pub fn build_magic_packet(mac_addr: &str) -> Result<[u8; 102], String> {
    let mut cleaned = [0u8; 12];
    let mut idx = 0usize;

    for b in mac_addr.bytes() {
        if b == b':' || b == b'-' {
            continue;
        }
        if idx >= 12 {
            return Err("MAC地址格式无效。它应该是12个十六进制数字。".to_string());
        }
        cleaned[idx] = b;
        idx += 1;
    }

    if idx != 12 {
        return Err("MAC地址格式无效。它应该是12个十六进制数字。".to_string());
    }

    let mut mac_bytes = [0u8; 6];
    for i in 0..6 {
        let hi = hex_nibble(cleaned[i * 2])
            .ok_or_else(|| "MAC地址解码错误: 包含非法十六进制字符".to_string())?;
        let lo = hex_nibble(cleaned[i * 2 + 1])
            .ok_or_else(|| "MAC地址解码错误: 包含非法十六进制字符".to_string())?;
        mac_bytes[i] = (hi << 4) | lo;
    }

    let mut packet = [0u8; 102];
    packet[..6].fill(0xff);
    for i in 0..16 {
        let start = 6 + i * 6;
        packet[start..start + 6].copy_from_slice(&mac_bytes);
    }
    Ok(packet)
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

pub fn wake_on_lan(mac_addr: &str) -> String {
    let mut result = String::new();
    let _ = writeln!(result, "正在唤醒的电脑 MAC 地址: {mac_addr}");

    let packet = match build_magic_packet(mac_addr) {
        Ok(p) => p,
        Err(err) => return format!("{err}\n"),
    };

    let targets = discover_private_broadcast_targets();
    let mut sent_count = 0usize;
    let mut sent_broadcasts: Vec<Ipv4Addr> = Vec::new();

    for target in targets {
        if sent_broadcasts.contains(&target.broadcast) {
            continue;
        }
        if send_wol_packet(target.local_ip, target.broadcast, &packet).is_ok() {
            let _ = writeln!(
                result,
                "魔术包已发送到 {} (via {}, from {})",
                target.broadcast, target.iface_name, target.local_ip
            );
            sent_broadcasts.push(target.broadcast);
            sent_count += 1;
        }
    }

    // 兜底：若枚举不到具体私有网段广播地址（例如非 Linux 平台或特殊网络环境），尝试 255.255.255.255
    if sent_count == 0
        && send_wol_packet(Ipv4Addr::UNSPECIFIED, Ipv4Addr::BROADCAST, &packet).is_ok()
    {
        let _ = writeln!(result, "魔术包已发送到 255.255.255.255 (fallback)");
        sent_count += 1;
    }

    if sent_count == 0 {
        result.push_str("没有找到有效的内网接口来发送唤醒包。");
    }

    result
}

fn send_wol_packet(
    local_ip: Ipv4Addr,
    broadcast: Ipv4Addr,
    packet: &[u8; 102],
) -> std::io::Result<()> {
    let socket = UdpSocket::bind(SocketAddrV4::new(local_ip, 0))?;
    socket.set_broadcast(true)?;
    socket.send_to(packet, SocketAddrV4::new(broadcast, 9))?;
    Ok(())
}

struct BroadcastTarget {
    iface_name: String,
    local_ip: Ipv4Addr,
    broadcast: Ipv4Addr,
}

#[cfg(unix)]
fn discover_private_broadcast_targets() -> Vec<BroadcastTarget> {
    use std::ffi::CStr;

    const AF_INET: i32 = 2;
    const IFF_UP: u32 = 0x1;
    const IFF_BROADCAST: u32 = 0x2;
    const IFF_LOOPBACK: u32 = 0x8;
    const IFF_RUNNING: u32 = 0x40;

    #[repr(C)]
    struct SockAddr {
        sa_family: u16,
        sa_data: [u8; 14],
    }

    #[repr(C)]
    struct SockAddrIn {
        sin_family: u16,
        sin_port: u16,
        sin_addr: u32,
        sin_zero: [u8; 8],
    }

    #[repr(C)]
    struct IfAddrs {
        ifa_next: *mut IfAddrs,
        ifa_name: *mut i8,
        ifa_flags: u32,
        ifa_addr: *mut SockAddr,
        ifa_netmask: *mut SockAddr,
        ifa_ifu: *mut SockAddr,
        ifa_data: *mut core::ffi::c_void,
    }

    unsafe extern "C" {
        fn getifaddrs(ifap: *mut *mut IfAddrs) -> i32;
        fn freeifaddrs(ifa: *mut IfAddrs);
    }

    let mut head: *mut IfAddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs writes a valid linked list head or returns -1 on error.
    if unsafe { getifaddrs(&mut head) } != 0 || head.is_null() {
        return Vec::new();
    }

    let mut targets = Vec::new();
    let mut cur = head;

    while !cur.is_null() {
        // SAFETY: cur is a valid node within the list returned by getifaddrs.
        let ifa = unsafe { &*cur };
        cur = ifa.ifa_next;

        let flags = ifa.ifa_flags;
        if (flags & IFF_UP) == 0
            || (flags & IFF_BROADCAST) == 0
            || (flags & IFF_LOOPBACK) != 0
            || (flags & IFF_RUNNING) == 0
        {
            continue;
        }

        if ifa.ifa_addr.is_null() || ifa.ifa_netmask.is_null() {
            continue;
        }

        // SAFETY: Checked non-null; sa_family is the common prefix of sockaddr and sockaddr_in.
        let family = unsafe { (*ifa.ifa_addr).sa_family } as i32;
        if family != AF_INET {
            continue;
        }

        // SAFETY: AF_INET guarantees ifa_addr and ifa_netmask point to sockaddr_in.
        let addr_in = unsafe { &*(ifa.ifa_addr as *const SockAddrIn) };
        let mask_in = unsafe { &*(ifa.ifa_netmask as *const SockAddrIn) };

        let ip = Ipv4Addr::from(u32::from_be(addr_in.sin_addr));
        if !ip.is_private() {
            continue;
        }

        let mask = Ipv4Addr::from(u32::from_be(mask_in.sin_addr));
        let ip_oct = ip.octets();
        let mask_oct = mask.octets();
        let broadcast = Ipv4Addr::new(
            ip_oct[0] | !mask_oct[0],
            ip_oct[1] | !mask_oct[1],
            ip_oct[2] | !mask_oct[2],
            ip_oct[3] | !mask_oct[3],
        );

        let iface_name = if !ifa.ifa_name.is_null() {
            // SAFETY: ifa_name is a NUL-terminated C string provided by libc.
            unsafe { CStr::from_ptr(ifa.ifa_name.cast()) }
                .to_string_lossy()
                .into_owned()
        } else {
            "unknown".to_string()
        };

        targets.push(BroadcastTarget {
            iface_name,
            local_ip: ip,
            broadcast,
        });
    }

    // SAFETY: head was returned by getifaddrs and is freed once here.
    unsafe { freeifaddrs(head) };
    targets
}

#[cfg(not(unix))]
fn discover_private_broadcast_targets() -> Vec<BroadcastTarget> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_valid_magic_packet() {
        let packet = build_magic_packet("00-11-22-AA-bb-FF").expect("valid mac");
        assert_eq!(&packet[..6], &[0xff; 6]);
        let expected_mac = [0x00, 0x11, 0x22, 0xaa, 0xbb, 0xff];
        for i in 0..16 {
            let start = 6 + i * 6;
            assert_eq!(&packet[start..start + 6], &expected_mac);
        }
    }

    #[test]
    fn rejects_invalid_mac() {
        assert!(build_magic_packet("00:11:22").is_err());
        assert!(build_magic_packet("GG:11:22:33:44:55").is_err());
    }
}
