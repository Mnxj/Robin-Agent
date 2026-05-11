use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use anyhow::bail;

/// CIDR ranges considered internal/private.
static PRIVATE_CIDRS: &[(&str, &str, u8)] = &[
    ("127.0.0.0",   "255.0.0.0",   8),   // loopback
    ("10.0.0.0",    "255.0.0.0",   8),   // RFC 1918
    ("172.16.0.0",  "255.240.0.0", 12),  // RFC 1918
    ("192.168.0.0", "255.255.0.0", 16),  // RFC 1918
    ("169.254.0.0", "255.255.0.0", 16),  // link-local
];

fn is_private_ipv4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    let addr = u32::from_be_bytes(o);
    for (network, mask, prefix) in PRIVATE_CIDRS {
        let net: u32 = {
            let parts: Vec<u8> = network.split('.').map(|x| x.parse().unwrap()).collect();
            u32::from_be_bytes([parts[0], parts[1], parts[2], parts[3]])
        };
        let bits: u32 = *prefix as u32;
        let mask_val = if bits == 0 { 0 } else { !0u32 << (32 - bits) };
        let _ = mask; // computed from prefix
        if addr & mask_val == net & mask_val {
            return true;
        }
    }
    false
}

fn is_private_ipv6(ip: Ipv6Addr) -> bool {
    // ::1/128
    if ip == Ipv6Addr::LOCALHOST {
        return true;
    }
    let segs = ip.segments();
    // fc00::/7 (unique local: fc00:: to fdff::)
    if segs[0] & 0xfe00 == 0xfc00 {
        return true;
    }
    // fe80::/10 (link-local)
    if segs[0] & 0xffc0 == 0xfe80 {
        return true;
    }
    false
}

fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_private_ipv4(v4),
        IpAddr::V6(v6) => is_private_ipv6(v6),
    }
}

/// Checks that a URL does not point to an internal/private network address.
/// This prevents SSRF attacks that could access cloud metadata endpoints,
/// localhost services, or internal network resources.
pub fn validate_url_not_internal(raw_url: &str) -> anyhow::Result<()> {
    let u = url::Url::parse(raw_url).map_err(|e| anyhow::anyhow!("invalid URL: {}", e))?;
    let host = u.host_str().ok_or_else(|| anyhow::anyhow!("URL has no host"))?;

    // Block common metadata hostnames
    let lower = host.to_lowercase();
    if lower == "metadata.google.internal" || lower == "metadata" {
        bail!("access to internal metadata endpoint is blocked");
    }

    // Resolve hostname to IP addresses
    let ips: Vec<IpAddr> = {
        // Try direct IP parse first
        if let Ok(ip) = host.parse::<IpAddr>() {
            vec![ip]
        } else {
            // DNS lookup
            use std::net::ToSocketAddrs;
            let addr_str = format!("{}:80", host);
            addr_str
                .to_socket_addrs()
                .map_err(|_| {
                    anyhow::anyhow!(
                        "cannot resolve hostname {:?} — blocking to prevent SSRF",
                        host
                    )
                })?
                .map(|sa| sa.ip())
                .collect()
        }
    };

    for ip in &ips {
        if is_private_ip(*ip) {
            bail!("access to internal address {} ({}) is blocked", host, ip);
        }
    }

    Ok(())
}