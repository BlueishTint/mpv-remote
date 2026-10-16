//! Helper subcommands used by the Lua plugin (`token`, `pair`).

use std::net::Ipv4Addr;

use qrcode::{Color, QrCode};

/// 24 hex characters from the OS random source.
pub fn token() -> String {
    let mut buf = [0u8; 12];
    getrandom::fill(&mut buf).expect("OS random source unavailable");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// Adapter names that never carry the phone's LAN (VPNs, containers, hypervisors).
const VIRTUAL_HINTS: &[&str] = &[
    "warp", "cloudflare", "tailscale", "zerotier", "wireguard", "vethernet", "wsl", "hyper-v",
    "vmware", "virtualbox", "vbox", "docker", "br-", "veth", "virbr", "utun", "tun", "tap",
    "loopback", "npcap",
];

/// Picks the private IPv4 of the physical LAN adapter. WARP/Tailscale etc. are skipped so the
/// QR never advertises an address the phone can't reach.
pub fn lan_ip() -> Option<Ipv4Addr> {
    let mut best: Option<(u8, Ipv4Addr)> = None;
    for iface in if_addrs::get_if_addrs().ok()? {
        let name = iface.name.to_lowercase();
        if iface.is_loopback() || VIRTUAL_HINTS.iter().any(|h| name.contains(h)) {
            continue;
        }
        let if_addrs::IfAddr::V4(v4) = &iface.addr else { continue };
        let o = v4.ip.octets();
        let rank = match o {
            [192, 168, ..] => 0,
            [10, ..] => 1,
            [172, b, ..] if (16..32).contains(&b) => 2,
            _ => continue,
        };
        if best.is_none_or(|(r, _)| rank < r) {
            best = Some((rank, v4.ip));
        }
    }
    best.map(|(_, ip)| ip)
}

/// Prints the pairing URL, then the QR matrix as rows of `0`/`1` for the plugin to draw.
pub fn pair(host: Option<String>, port: u16, token: &str) -> Result<(), String> {
    let host = match host.filter(|h| !h.is_empty()) {
        Some(h) => h,
        None => lan_ip().ok_or("no private LAN IPv4 found; set host= in mpv-remote.conf")?.to_string(),
    };
    let url = format!("http://{host}:{port}/?t={token}");
    let code = QrCode::new(url.as_bytes()).map_err(|e| e.to_string())?;
    println!("{url}");
    for row in code.to_colors().chunks(code.width()) {
        println!("{}", row.iter().map(|c| if *c == Color::Dark { '1' } else { '0' }).collect::<String>());
    }
    Ok(())
}
