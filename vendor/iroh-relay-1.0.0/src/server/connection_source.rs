use std::net::{IpAddr, Ipv4Addr};

/// Normalized source identity used by TCP admission controls.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum ConnectionSource {
    V4(Ipv4Addr),
    V6Prefix64([u8; 8]),
}

impl ConnectionSource {
    pub(super) fn from_ip(ip: IpAddr) -> Self {
        match ip {
            IpAddr::V4(ip) => Self::V4(ip),
            IpAddr::V6(ip) => {
                if let Some(ip) = ip.to_ipv4_mapped() {
                    return Self::V4(ip);
                }
                let octets = ip.octets();
                Self::V6Prefix64(octets[..8].try_into().expect("IPv6 /64 is eight bytes"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv4_mapped_and_ipv6_prefixes_are_normalized() {
        let ipv4 = Ipv4Addr::new(203, 0, 113, 9);
        assert_eq!(
            ConnectionSource::from_ip(ipv4.into()),
            ConnectionSource::from_ip(ipv4.to_ipv6_mapped().into())
        );

        let first: IpAddr = "2001:db8:1:2::1".parse().unwrap();
        let same_prefix: IpAddr = "2001:db8:1:2:ffff::2".parse().unwrap();
        let other_prefix: IpAddr = "2001:db8:1:3::1".parse().unwrap();
        assert_eq!(
            ConnectionSource::from_ip(first),
            ConnectionSource::from_ip(same_prefix)
        );
        assert_ne!(
            ConnectionSource::from_ip(first),
            ConnectionSource::from_ip(other_prefix)
        );
    }
}
