//! 特殊 IPv4/IPv6 地址识别（内网/回环/链路本地/组播/文档/保留）。
//!
//! 这些地址不进入地理库查询，直接返回 [`crate::types::IpLocation::reserved`]，
//! 避免把 `10.0.0.1` 误判为某个公网位置。

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use crate::types::IpClass;

/// 将 IPv4-mapped IPv6（`::ffff:a.b.c.d`）归一为 IPv4。
pub fn normalize_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6(v6),
        },
        IpAddr::V4(_) => ip,
    }
}

/// 判断地址是否属于特殊用途网段；公网地址返回 `None`。
pub fn classify(ip: IpAddr) -> Option<IpClass> {
    match normalize_ip(ip) {
        IpAddr::V4(v4) => classify_v4(v4),
        IpAddr::V6(v6) => classify_v6(v6),
    }
}

/// IPv4 特殊网段。
pub fn classify_v4(ip: Ipv4Addr) -> Option<IpClass> {
    let v = u32::from(ip);
    let class = if v == 0 {
        IpClass::Unspecified
    } else if v & 0xff00_0000 == 0x7f00_0000 {
        // 127.0.0.0/8
        IpClass::Loopback
    } else if v & 0xff00_0000 == 0x0a00_0000 {
        // 10.0.0.0/8
        IpClass::Private
    } else if v & 0xfff0_0000 == 0xac10_0000 {
        // 172.16.0.0/12
        IpClass::Private
    } else if v & 0xffff_0000 == 0xc0a8_0000 {
        // 192.168.0.0/16
        IpClass::Private
    } else if v & 0xffc0_0000 == 0x6440_0000 {
        // 100.64.0.0/10（运营商级 NAT）
        IpClass::Cgnat
    } else if v & 0xffff_0000 == 0xa9fe_0000 {
        // 169.254.0.0/16
        IpClass::LinkLocal
    } else if v & 0xffff_ff00 == 0xc000_0000 {
        // 192.0.0.0/24（IETF 协议分配）
        IpClass::Reserved
    } else if v & 0xffff_ff00 == 0xc000_0200 {
        // 192.0.2.0/24（TEST-NET-1）
        IpClass::Documentation
    } else if v & 0xffff_ff00 == 0xc058_6300 {
        // 192.88.99.0/24（6to4 中继，已废弃）
        IpClass::Reserved
    } else if v & 0xfffe_0000 == 0xc612_0000 {
        // 198.18.0.0/15（基准测试）
        IpClass::Benchmark
    } else if v & 0xffff_ff00 == 0xc633_6400 {
        // 198.51.100.0/24（TEST-NET-2）
        IpClass::Documentation
    } else if v & 0xffff_ff00 == 0xcb00_7100 {
        // 203.0.113.0/24（TEST-NET-3）
        IpClass::Documentation
    } else if v & 0xf000_0000 == 0xe000_0000 {
        // 224.0.0.0/4
        IpClass::Multicast
    } else if v & 0xf000_0000 == 0xf000_0000 {
        // 240.0.0.0/4（含 255.255.255.255）
        IpClass::Reserved
    } else if v & 0xff00_0000 == 0x0000_0000 {
        // 0.0.0.0/8（除 0.0.0.0 外均为“本网络”）
        IpClass::Reserved
    } else {
        return None;
    };
    Some(class)
}

/// IPv6 特殊网段。
pub fn classify_v6(ip: Ipv6Addr) -> Option<IpClass> {
    let b = ip.octets();
    let class = if b.iter().all(|x| *x == 0) {
        IpClass::Unspecified
    } else if b[..15].iter().all(|x| *x == 0) && b[15] == 1 {
        IpClass::Loopback
    } else if b[..12] == [0, 0x64, 0xff, 0x9b, 0, 0, 0, 0, 0, 0, 0, 0] {
        // 64:ff9b::/96（NAT64）
        IpClass::Nat64
    } else if b[..8] == [0x01, 0x00, 0, 0, 0, 0, 0, 0] {
        // 100::/64（丢弃前缀）
        IpClass::Shared
    } else if b[0] == 0x20 && b[1] == 0x01 && b[2] == 0x0d && b[3] == 0xb8 {
        // 2001:db8::/32
        IpClass::Documentation
    } else if b[0] == 0x20 && b[1] == 0x01 && b[2] == 0x00 && (b[3] & 0xf0) == 0x10 {
        // 2001:10::/28（ORCHID，已废弃）
        IpClass::Orchid
    } else if b[0] & 0xfe == 0xfc {
        // fc00::/7（唯一本地地址）
        IpClass::Private
    } else if b[0] == 0xfe && b[1] & 0xc0 == 0x80 {
        // fe80::/10（链路本地）
        IpClass::LinkLocal
    } else if b[0] == 0xff {
        // ff00::/8（组播）
        IpClass::Multicast
    } else {
        return None;
    };
    Some(class)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v4(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn v6(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn public_addresses_are_not_classified() {
        for ip in [
            "8.8.8.8",
            "1.1.1.1",
            "114.114.114.114",
            "223.5.5.5",
            "2001:4860:4860::8888",
            "2400:3200::1",
        ] {
            assert_eq!(classify(ip.parse().unwrap()), None, "{ip}");
        }
    }

    #[test]
    fn ipv4_special_ranges() {
        assert_eq!(
            classify_v4(Ipv4Addr::UNSPECIFIED),
            Some(IpClass::Unspecified)
        );
        assert_eq!(classify(v4("127.0.0.2")), Some(IpClass::Loopback));
        assert_eq!(classify(v4("10.0.0.0")), Some(IpClass::Private));
        assert_eq!(classify(v4("10.255.255.255")), Some(IpClass::Private));
        assert_eq!(classify(v4("172.15.255.255")), None);
        assert_eq!(classify(v4("172.16.0.0")), Some(IpClass::Private));
        assert_eq!(classify(v4("172.31.255.255")), Some(IpClass::Private));
        assert_eq!(classify(v4("172.32.0.0")), None);
        assert_eq!(classify(v4("192.168.1.1")), Some(IpClass::Private));
        assert_eq!(classify(v4("100.64.0.1")), Some(IpClass::Cgnat));
        assert_eq!(classify(v4("169.254.10.10")), Some(IpClass::LinkLocal));
        assert_eq!(classify(v4("192.0.2.1")), Some(IpClass::Documentation));
        assert_eq!(classify(v4("198.18.0.1")), Some(IpClass::Benchmark));
        assert_eq!(classify(v4("198.51.100.1")), Some(IpClass::Documentation));
        assert_eq!(classify(v4("203.0.113.1")), Some(IpClass::Documentation));
        assert_eq!(classify(v4("224.0.0.1")), Some(IpClass::Multicast));
        assert_eq!(classify(v4("240.0.0.1")), Some(IpClass::Reserved));
        assert_eq!(classify(v4("255.255.255.255")), Some(IpClass::Reserved));
    }

    #[test]
    fn ipv6_special_ranges() {
        assert_eq!(
            classify_v6(Ipv6Addr::UNSPECIFIED),
            Some(IpClass::Unspecified)
        );
        assert_eq!(classify_v6(Ipv6Addr::LOCALHOST), Some(IpClass::Loopback));
        assert_eq!(classify(v6("fc00::1")), Some(IpClass::Private));
        assert_eq!(classify(v6("fd12:3456::1")), Some(IpClass::Private));
        assert_eq!(classify(v6("fe80::1")), Some(IpClass::LinkLocal));
        assert_eq!(classify(v6("2001:db8::1")), Some(IpClass::Documentation));
        assert_eq!(classify(v6("ff02::1")), Some(IpClass::Multicast));
        assert_eq!(classify(v6("64:ff9b::1.2.3.4")), Some(IpClass::Nat64));
        assert_eq!(classify(v6("100::1")), Some(IpClass::Shared));
    }

    #[test]
    fn ipv4_mapped_is_normalized() {
        let ip = v6("::ffff:192.168.1.10");
        assert_eq!(normalize_ip(ip), v4("192.168.1.10"));
        assert_eq!(classify(ip), Some(IpClass::Private));
        assert_eq!(classify(v6("::ffff:8.8.8.8")), None);
    }
}
