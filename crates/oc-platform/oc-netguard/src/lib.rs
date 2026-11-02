//! SSRF guard for server-side fetches of user-chosen URLs. A safe fetch checks the name ([`is_public_host`]), resolves
//! it once ([`resolve_public`]), connects to exactly those addresses (`resolve_to_addrs`) and follows no redirect
//! without repeating all of this.

use std::net::{IpAddr, SocketAddr};

/// `isPublicAddress(ip)` (services/api/src/netguard.ts): refuses loopback, private, link-local (cloud metadata),
/// CGNAT, documentation, benchmarking, multicast and reserved ranges, and IPv6 forms that can wrap an IPv4 address
/// (mapped, compatible, NAT64, 6to4, Teredo).
pub fn is_public_ip(ip: IpAddr) -> bool {
    let v4_blocked: [(u32, u32); 15] = [
        (0x0000_0000, 8), (0x0a00_0000, 8), (0x6440_0000, 10), (0x7f00_0000, 8), (0xa9fe_0000, 16), (0xac10_0000, 12), (0xc000_0000, 24), (0xc000_0200, 24),
        (0xc058_6300, 24), (0xc0a8_0000, 16), (0xc612_0000, 15), (0xc633_6400, 24), (0xcb00_7100, 24), (0xe000_0000, 4), (0xf000_0000, 4),
    ];
    let v6_blocked: [(u128, u32); 12] = [
        (0, 96), (0xffff_0000_0000, 96), (0x0064_ff9b_0000_0000_0000_0000_0000_0000, 96), (0x0064_ff9b_0001_0000_0000_0000_0000_0000, 48),
        (0x0100_0000_0000_0000_0000_0000_0000_0000, 64), (0x2001_0000_0000_0000_0000_0000_0000_0000, 32), (0x2001_0db8_0000_0000_0000_0000_0000_0000, 32),
        (0x2002_0000_0000_0000_0000_0000_0000_0000, 16), (0xfc00_0000_0000_0000_0000_0000_0000_0000, 7), (0xfe80_0000_0000_0000_0000_0000_0000_0000, 10),
        (0xfec0_0000_0000_0000_0000_0000_0000_0000, 10), (0xff00_0000_0000_0000_0000_0000_0000_0000, 8),
    ];
    match ip {
        IpAddr::V4(a) => {
            let x = u32::from(a);
            !v4_blocked.iter().any(|&(net, bits)| x >> (32 - bits) == net >> (32 - bits))
        }
        IpAddr::V6(a) => {
            let x = u128::from(a);
            !v6_blocked.iter().any(|&(net, bits)| x >> (128 - bits) == net >> (128 - bits))
        }
    }
}

/// `isPublicHost(hostname)`: the URL's host as text, IPv6 literals with or without brackets. Internal names and
/// non-public IP literals are refused; other names still have to pass [`resolve_public`].
pub fn is_public_host(hostname: &str) -> bool {
    let h = hostname.trim_end_matches('.').to_lowercase();
    if h == "localhost" || h.ends_with(".localhost") || h.ends_with(".local") || h.ends_with(".internal") {
        return false;
    }
    match h.trim_start_matches('[').trim_end_matches(']').parse::<IpAddr>() {
        Ok(ip) => is_public_ip(ip),
        Err(_) => true,
    }
}

/// `publicLookup`: every address `host` resolves to, refused if ANY is not public. The client then connects to
/// exactly these addresses (`resolve_to_addrs`), so DNS rebinding can't slip in between the check and the connect.
pub async fn resolve_public(host: &str, port: u16) -> Result<Vec<SocketAddr>, String> {
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port)).await.map_err(|e| e.to_string())?.collect();
    if addrs.is_empty() || addrs.iter().any(|a| !is_public_ip(a.ip())) {
        return Err(format!("{host} resolves to a non-public address"));
    }
    Ok(addrs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts() {
        for h in [
            "localhost", "localhost.", "a.localhost", "x.local", "db.internal", "[::1]", "[::]", "fd00::1", "[::ffff:169.254.169.254]", "[::ffff:a9fe:a9fe]", "[64:ff9b::7f00:1]", "10.1.2.3", "127.0.0.1", "169.254.169.254", "172.16.0.1", "192.168.1.1", "100.64.0.1",
            "0.0.0.0",
        ] {
            assert!(!is_public_host(h), "{h}");
        }
        for h in ["example.com", "8.8.8.8", "[2606:4700:4700::1111]", "172.32.0.1", "100.128.0.1"] {
            assert!(is_public_host(h), "{h}");
        }
    }
}
