//! Host names of the Iroh endpoint (relays) resolved the way the signaling
//! connection resolves its server: by the operating system (`getaddrinfo`,
//! which follows the system's DNS servers, VPN rules and hosts file). Iroh's
//! own resolver (`n0-dns-resolver` since Iroh 1.3, Smart Explorer 0.5.181)
//! runs alongside and answers TXT queries and names the system does not know.
//! Windows devices whose relay stayed unreachable while their signaling to
//! the same host worked (2026-10-22) published no relay at all.
use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::pin::Pin;
use std::task::Poll;

use iroh::dns::{BoxIter, DnsError, DnsResolver, Resolver, TxtRecordData, DNS_TIMEOUT};

type Lookup<T> = Pin<Box<dyn Future<Output = Result<BoxIter<T>, DnsError>> + Send + 'static>>;

/// The resolver of the Share endpoint.
pub(super) fn resolver() -> DnsResolver {
    DnsResolver::custom(SystemFirstResolver {
        fallback: DnsResolver::new(),
    })
}

#[derive(Clone, Debug)]
struct SystemFirstResolver {
    /// Iroh's own resolver.
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
