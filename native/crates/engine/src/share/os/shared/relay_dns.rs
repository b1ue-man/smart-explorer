//! Host names of the Iroh endpoint (relays) resolved the way the signaling
//! connection resolves its server: by the operating system (`getaddrinfo`,
//! which follows the system's DNS servers, VPN rules and hosts file). Iroh's
//! own resolver (`n0-dns-resolver` since Iroh 1.3, Smart Explorer 0.5.181)
//! runs alongside and answers TXT queries and names the system does not know.
//! It reads only the system's DNS configuration: its public fallback
//! nameservers (Cloudflare, Google, Quad9) are switched off, so no DNS query
//! goes to a public resolver that the system configuration does not name
//! (decision E15, 2026-11-07). Windows devices whose relay stayed unreachable
//! while their signaling to the same host worked (2026-10-22) published no
//! relay at all.
use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::pin::Pin;
use std::task::Poll;

use iroh::dns::{
    BoxIter, DnsError, DnsResolver, FallbackMode, Resolver, TxtRecordData, DNS_TIMEOUT,
};

type Lookup<T> = Pin<Box<dyn Future<Output = Result<BoxIter<T>, DnsError>> + Send + 'static>>;

/// The resolver of the Share endpoint.
pub(super) fn resolver() -> DnsResolver {
    DnsResolver::custom(SystemFirstResolver {
        fallback: system_only_resolver(),
    })
}

/// Iroh's own resolver with the system's DNS configuration as its only source.
/// `FallbackMode::Never` removes the public fallback tier that Iroh's default
/// resolver (`DnsResolver::new()`, mode `Deferred`) queries once the system
/// nameservers fail. With no system nameserver, this resolver answers with an
/// error; the system resolver (`getaddrinfo`) still answers first.
fn system_only_resolver() -> DnsResolver {
    DnsResolver::builder()
        .with_system_defaults()
        .with_fallback_mode(FallbackMode::Never)
        .build()
}

#[cfg(test)]
#[path = "relay_dns_task_tests.rs"]
mod task_tests;

#[derive(Clone, Debug)]
struct SystemFirstResolver {
    /// Iroh's own resolver, reading the system DNS configuration only.
    fallback: DnsResolver,
}

impl Resolver for SystemFirstResolver {
    fn lookup_ipv4(&self, host: String) -> Lookup<Ipv4Addr> {
        let fallback = self.fallback.clone();
        Box::pin(async move {
            let own = async {
                fallback
                    .lookup_ipv4(host.clone(), DNS_TIMEOUT)
                    .await
                    .map(|addrs| addrs.filter_map(ipv4).collect::<Vec<Ipv4Addr>>())
            };
            first_answer(system_addresses(&host, ipv4), own)
                .await
                .map(|addrs| Box::new(addrs.into_iter()) as BoxIter<Ipv4Addr>)
        })
    }

    fn lookup_ipv6(&self, host: String) -> Lookup<Ipv6Addr> {
        let fallback = self.fallback.clone();
        Box::pin(async move {
            let own = async {
                fallback
                    .lookup_ipv6(host.clone(), DNS_TIMEOUT)
                    .await
                    .map(|addrs| addrs.filter_map(ipv6).collect::<Vec<Ipv6Addr>>())
            };
            first_answer(system_addresses(&host, ipv6), own)
                .await
                .map(|addrs| Box::new(addrs.into_iter()) as BoxIter<Ipv6Addr>)
        })
    }

    fn lookup_txt(&self, host: String) -> Lookup<TxtRecordData> {
        let fallback = self.fallback.clone();
        Box::pin(async move {
            fallback.lookup_txt(host, DNS_TIMEOUT).await.map(|records| {
                let records: Vec<TxtRecordData> = records.collect();
                Box::new(records.into_iter()) as BoxIter<TxtRecordData>
            })
        })
    }

    fn clear_cache(&self) {
        self.fallback.clear_cache();
    }

    fn reset(&self) -> Box<dyn Resolver> {
        // The system resolver keeps no state here; Iroh's swaps its own.
        self.fallback.reset();
        Box::new(self.clone())
    }
}

fn ipv4(ip: IpAddr) -> Option<Ipv4Addr> {
    match ip {
        IpAddr::V4(ip) => Some(ip),
        IpAddr::V6(_) => None,
    }
}

fn ipv6(ip: IpAddr) -> Option<Ipv6Addr> {
    match ip {
        IpAddr::V6(ip) => Some(ip),
        IpAddr::V4(_) => None,
    }
}

/// The system's addresses of `host` of one family; none when it has none.
async fn system_addresses<T>(host: &str, family: fn(IpAddr) -> Option<T>) -> Vec<T> {
    match tokio::net::lookup_host((host, 0u16)).await {
        Ok(addrs) => addrs.filter_map(|addr| family(addr.ip())).collect(),
        Err(_) => Vec::new(),
    }
}

/// The first non-empty answer of the system and of Iroh's resolver. When
/// neither has addresses, Iroh's result (its error) is the answer.
async fn first_answer<T>(
    system: impl Future<Output = Vec<T>>,
    own: impl Future<Output = Result<Vec<T>, DnsError>>,
) -> Result<Vec<T>, DnsError> {
    let mut system = std::pin::pin!(system);
    let mut own = std::pin::pin!(own);
    let mut system_done = false;
    let mut own_result = None;
    std::future::poll_fn(move |cx| {
        if !system_done {
            if let Poll::Ready(addrs) = system.as_mut().poll(cx) {
                if !addrs.is_empty() {
                    return Poll::Ready(Ok(addrs));
                }
                system_done = true;
            }
        }
        if own_result.is_none() {
            if let Poll::Ready(result) = own.as_mut().poll(cx) {
                match result {
                    Ok(addrs) if !addrs.is_empty() => return Poll::Ready(Ok(addrs)),
                    other => own_result = Some(other),
                }
            }
        }
        match own_result.take() {
            Some(result) if system_done => Poll::Ready(result),
            kept => {
                own_result = kept;
                Poll::Pending
            }
        }
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::{first_answer, resolver};
    use iroh::dns::DnsError;
    use std::net::{IpAddr, Ipv4Addr};
    use std::time::Duration;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn windows_direct_task_relay_names_resolve_through_the_system() {
        let (addrs, described) = runtime().block_on(async {
            let resolver = resolver();
            let addrs: Vec<IpAddr> = resolver
                .lookup_ipv4("localhost", Duration::from_secs(10))
                .await
                .unwrap()
                .collect();
            (addrs, format!("{resolver:?}"))
        });
        assert!(
            addrs.contains(&IpAddr::V4(Ipv4Addr::LOCALHOST)),
            "{addrs:?}"
        );
        assert!(described.contains("SystemFirstResolver"), "{described}");
    }

    #[test]
    fn windows_direct_task_first_answer_prefers_any_addresses() {
        let one = Ipv4Addr::new(192, 0, 2, 1);
        let two = Ipv4Addr::new(192, 0, 2, 2);
        runtime().block_on(async {
            // The system answers while Iroh's resolver still waits.
            let own = std::future::pending::<Result<Vec<Ipv4Addr>, DnsError>>();
            let answer = first_answer(std::future::ready(vec![one]), own).await;
            assert_eq!(answer.unwrap(), vec![one]);
            // The system knows nothing; Iroh's resolver does.
            let answer = first_answer(
                std::future::ready(Vec::new()),
                std::future::ready(Ok::<_, DnsError>(vec![two])),
            )
            .await;
            assert_eq!(answer.unwrap(), vec![two]);
            // Iroh's resolver answers first while the system lookup hangs.
            let answer = first_answer(
                std::future::pending::<Vec<Ipv4Addr>>(),
                std::future::ready(Ok::<_, DnsError>(vec![two])),
            )
            .await;
            assert_eq!(answer.unwrap(), vec![two]);
            // Neither has addresses: the empty answer of Iroh's resolver.
            let answer = first_answer(
                std::future::ready(Vec::<Ipv4Addr>::new()),
                std::future::ready(Ok::<_, DnsError>(Vec::new())),
            )
            .await;
            assert!(answer.unwrap().is_empty());
        });
    }
}

/// Decision E15: Iroh's resolver of the Share endpoint reads the system's DNS
/// configuration only. Linux only: it compares with `/etc/resolv.conf`.
#[cfg(all(test, target_os = "linux"))]
mod public_fallback_tests {
    use super::system_only_resolver;
    use iroh::dns::DNS_TIMEOUT;
    use std::net::{IpAddr, SocketAddr};
    use std::time::Duration;

    /// The public resolvers of Iroh's default fallback tier: Cloudflare,
    /// Google and Quad9, IPv4 and IPv6.
    pub(super) const PUBLIC_RESOLVERS: [&str; 12] = [
        "1.1.1.1",
        "1.0.0.1",
        "8.8.8.8",
        "8.8.4.4",
        "9.9.9.9",
        "149.112.112.112",
        "2606:4700:4700::1111",
        "2606:4700:4700::1001",
        "2001:4860:4860::8888",
        "2001:4860:4860::8844",
        "2620:fe::fe",
        "2620:fe::9",
    ];

    /// Every IP address `text` names, alone or as a socket address.
    pub(super) fn addresses_in(text: &str) -> Vec<IpAddr> {
        text.split(|c: char| !(c.is_ascii_hexdigit() || c == '.' || c == ':'))
            .filter_map(|token| {
                token
                    .parse::<IpAddr>()
                    .ok()
                    .or_else(|| token.parse::<SocketAddr>().ok().map(|addr| addr.ip()))
            })
            .collect()
    }

    /// The nameservers of the system configuration.
    pub(super) fn system_nameservers() -> Vec<IpAddr> {
        let text = std::fs::read_to_string("/etc/resolv.conf").unwrap_or_default();
        text.lines()
            .filter_map(|line| line.trim().strip_prefix("nameserver"))
            .filter_map(|rest| rest.split_whitespace().next())
            .filter_map(|server| server.split('%').next()?.parse().ok())
            .collect()
    }

    #[test]
    fn system_only_resolver_has_no_public_fallback() {
        // The reader finds addresses in the formats a Debug text uses.
        let sample = "Nameserver { addr: 8.8.8.8:53, v6: [2606:4700:4700::1111]:53, 1.0.0.1 }";
        let found = addresses_in(sample);
        assert_eq!(found.len(), 3, "{found:?}");
        assert!(found.contains(&"8.8.8.8".parse::<IpAddr>().expect("literal")));
        assert!(found.contains(&"2606:4700:4700::1111".parse::<IpAddr>().expect("literal")));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        let (outcome, described) = runtime.block_on(async {
            let resolver = system_only_resolver();
            // A name that never resolves (RFC 6761): nothing the system knows,
            // and no public resolver to ask next.
            let lookup = tokio::time::timeout(
                DNS_TIMEOUT + Duration::from_secs(30),
                resolver.lookup_ipv4("relay.invalid", DNS_TIMEOUT),
            )
            .await
            .expect("lookup deadline");
            (
                lookup.map(|addrs| addrs.collect::<Vec<_>>()),
                format!("{resolver:?}"),
            )
        });
        assert!(outcome.is_err(), "relay.invalid resolved: {outcome:?}");
        // Only an address that the system configuration itself names may appear.
        let system = system_nameservers();
        let public: Vec<IpAddr> = PUBLIC_RESOLVERS
            .iter()
            .map(|address| address.parse().expect("literal"))
            .collect();
        for address in addresses_in(&described) {
            assert!(
                !public.contains(&address) || system.contains(&address),
                "public resolver {address} in {described}"
            );
        }
    }
}
