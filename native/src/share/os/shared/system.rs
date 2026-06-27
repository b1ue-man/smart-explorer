pub(crate) fn lan_ips() -> Vec<String> {
    let mut v = Vec::new();
    if let Ok(ifaces) = if_addrs::get_if_addrs() {
        for iface in ifaces {
            let ip = iface.ip();
            if ip.is_loopback() || ip.is_unspecified() {
                continue;
            }
            if let std::net::IpAddr::V4(v4) = ip {
                v.push(v4.to_string());
            }
        }
    }
    if let Ok(s) = std::net::UdpSocket::bind("0.0.0.0:0") {
        if s.connect("8.8.8.8:80").is_ok() {
            if let Ok(a) = s.local_addr() {
                v.push(a.ip().to_string());
            }
        }
    }
    v.sort();
    v.dedup();
    v
}
