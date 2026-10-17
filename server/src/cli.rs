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

/// Tailscale assigns addresses from the shared-address range 100.64.0.0/10.
fn in_tailnet_range(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 100 && (64..128).contains(&o[1])
}

/// The Tailscale IPv4 from `tailscale status --json`, but only while it is actually connected.
/// A stopped client still reports its assigned address (`tailscale ip` does too), so the backend
/// state has to be checked or "off" would look like "on".
fn parse_tailscale_status(json: &[u8]) -> Option<Ipv4Addr> {
    let status: serde_json::Value = serde_json::from_slice(json).ok()?;
    if status.get("BackendState")?.as_str()? != "Running" {
        return None;
    }
    status
        .get("TailscaleIPs")?
        .as_array()?
        .iter()
        .filter_map(|ip| ip.as_str()?.parse::<Ipv4Addr>().ok())
        .find(|ip| in_tailnet_range(*ip))
}

/// Asks the Tailscale CLI. Covers platforms where the adapter isn't named "tailscale" (macOS uses `utunN`).
fn tailscale_cli_ip() -> Option<Ipv4Addr> {
    const CANDIDATES: [&str; 3] = [
        "tailscale",
        r"C:\Program Files\Tailscale\tailscale.exe",
        "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
    ];
    CANDIDATES.iter().find_map(|exe| {
        let mut cmd = std::process::Command::new(exe);
        cmd.args(["status", "--json"]).stderr(std::process::Stdio::null());
        #[cfg(windows)]
        std::os::windows::process::CommandExt::creation_flags(&mut cmd, 0x0800_0000); // CREATE_NO_WINDOW
        parse_tailscale_status(&cmd.output().ok()?.stdout)
    })
}

/// This machine's Tailscale IPv4, from the adapter if it is recognisable, else from the CLI.
pub fn tailscale_ip() -> Option<Ipv4Addr> {
    let from_adapter = if_addrs::get_if_addrs().ok().and_then(|ifaces| {
        ifaces.into_iter().find_map(|i| {
            let named = i.name.to_lowercase().contains("tailscale");
            match i.addr {
                if_addrs::IfAddr::V4(v4) if named && in_tailnet_range(v4.ip) => Some(v4.ip),
                _ => None,
            }
        })
    });
    from_adapter.or_else(tailscale_cli_ip)
}

/// Scheme + authority the phone should open. `host` comes from `host=` in mpv-remote.conf:
/// empty = auto-detected LAN address, `tailscale` = this machine's Tailscale address when
/// Tailscale is connected (else the LAN address), a full URL (`https://pc.tailnet.ts.net`) is used as is, anything else is an IP or name
/// (a `:port` suffix is respected, otherwise `port` is appended).
fn base_url(
    host: Option<&str>,
    port: u16,
    lan: impl FnOnce() -> Option<Ipv4Addr>,
    tailscale: impl FnOnce() -> Option<Ipv4Addr>,
) -> Result<String, String> {
    match host.map(str::trim).filter(|h| !h.is_empty()) {
        None => lan()
            .map(|ip| format!("http://{ip}:{port}"))
            .ok_or_else(|| "no private LAN IPv4 found; set host= in mpv-remote.conf".to_string()),
        // Tailscale when it is connected, the ordinary LAN address when it isn't, so one setting
        // works both away from home (Tailscale on) and at home (Tailscale off).
        Some(h) if h.eq_ignore_ascii_case("tailscale") => tailscale()
            .or_else(lan)
            .map(|ip| format!("http://{ip}:{port}"))
            .ok_or_else(|| "found neither a Tailscale nor a LAN address; set host=<ip>".to_string()),
        Some(h) if h.contains("://") => Ok(h.trim_end_matches('/').to_owned()),
        Some(h) if h.contains(':') => Ok(format!("http://{h}")),
        Some(h) => Ok(format!("http://{h}:{port}")),
    }
}

/// Prints the pairing URL, then the QR matrix as rows of `0`/`1` for the plugin to draw.
pub fn pair(host: Option<String>, port: u16, token: &str) -> Result<(), String> {
    let base = base_url(host.as_deref(), port, lan_ip, tailscale_ip)?;
    let url = format!("{base}/?t={token}");
    let code = QrCode::new(url.as_bytes()).map_err(|e| e.to_string())?;
    println!("{url}");
    for row in code.to_colors().chunks(code.width()) {
        println!("{}", row.iter().map(|c| if *c == Color::Dark { '1' } else { '0' }).collect::<String>());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAN: Option<Ipv4Addr> = Some(Ipv4Addr::new(192, 168, 1, 93));
    const TS: Option<Ipv4Addr> = Some(Ipv4Addr::new(100, 101, 102, 103));

    fn url(host: Option<&str>) -> Result<String, String> {
        base_url(host, 8765, || LAN, || TS)
    }

    #[test]
    fn tailnet_range_is_100_64_to_100_127() {
        assert!(in_tailnet_range(Ipv4Addr::new(100, 64, 0, 1)));
        assert!(in_tailnet_range(Ipv4Addr::new(100, 127, 255, 254)));
        assert!(!in_tailnet_range(Ipv4Addr::new(100, 63, 0, 1)));
        assert!(!in_tailnet_range(Ipv4Addr::new(100, 128, 0, 1)));
        assert!(!in_tailnet_range(Ipv4Addr::new(192, 168, 1, 1)));
    }

    #[test]
    fn default_uses_lan_address() {
        assert_eq!(url(None).unwrap(), "http://192.168.1.93:8765");
        assert_eq!(url(Some("")).unwrap(), "http://192.168.1.93:8765");
        assert_eq!(url(Some("  ")).unwrap(), "http://192.168.1.93:8765");
    }

    #[test]
    fn tailscale_keyword_uses_tailnet_address() {
        assert_eq!(url(Some("tailscale")).unwrap(), "http://100.101.102.103:8765");
        assert_eq!(url(Some("Tailscale")).unwrap(), "http://100.101.102.103:8765");
    }

    #[test]
    fn tailscale_keyword_falls_back_to_lan_when_tailscale_is_off() {
        let url = base_url(Some("tailscale"), 8765, || LAN, || None).unwrap();
        assert_eq!(url, "http://192.168.1.93:8765");
    }

    #[test]
    fn tailscale_keyword_prefers_tailscale_when_both_exist() {
        let url = base_url(Some("tailscale"), 8765, || LAN, || TS).unwrap();
        assert_eq!(url, "http://100.101.102.103:8765");
    }

    #[test]
    fn tailscale_keyword_errors_only_when_nothing_is_available() {
        assert!(base_url(Some("tailscale"), 8765, || None, || None).is_err());
    }

    #[test]
    fn explicit_hosts_are_respected() {
        assert_eq!(url(Some("10.0.0.5")).unwrap(), "http://10.0.0.5:8765");
        assert_eq!(url(Some("pc.tail1234.ts.net")).unwrap(), "http://pc.tail1234.ts.net:8765");
        assert_eq!(url(Some("pc:9000")).unwrap(), "http://pc:9000");
        assert_eq!(url(Some("https://pc.tail1234.ts.net")).unwrap(), "https://pc.tail1234.ts.net");
        assert_eq!(url(Some("https://pc.tail1234.ts.net/")).unwrap(), "https://pc.tail1234.ts.net");
    }

    #[test]
    fn lan_missing_is_an_error() {
        assert!(base_url(None, 8765, || None, || TS).is_err());
    }
}

#[cfg(test)]
mod status_tests {
    use super::*;

    #[test]
    fn running_client_yields_its_ipv4() {
        let json = br#"{"BackendState":"Running","TailscaleIPs":["fd7a:115c:a1e0::1","100.102.235.81"]}"#;
        assert_eq!(parse_tailscale_status(json), Some(Ipv4Addr::new(100, 102, 235, 81)));
    }

    #[test]
    fn stopped_client_is_treated_as_off_even_though_it_reports_an_address() {
        let json = br#"{"BackendState":"Stopped","TailscaleIPs":["100.102.235.81"]}"#;
        assert_eq!(parse_tailscale_status(json), None);
    }

    #[test]
    fn other_states_and_garbage_are_off() {
        for json in [
            &br#"{"BackendState":"NeedsLogin","TailscaleIPs":null}"#[..],
            &br#"{"BackendState":"Starting"}"#[..],
            &b"Tailscale is stopped."[..],
            &b""[..],
        ] {
            assert_eq!(parse_tailscale_status(json), None);
        }
    }

    #[test]
    fn running_without_an_ipv4_in_the_tailnet_range_is_ignored() {
        let json = br#"{"BackendState":"Running","TailscaleIPs":["192.168.1.5","fd7a:115c:a1e0::1"]}"#;
        assert_eq!(parse_tailscale_status(json), None);
    }
}
