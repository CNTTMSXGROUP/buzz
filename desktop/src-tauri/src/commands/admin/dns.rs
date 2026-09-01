//! Custom DNS resolver for the admin HTTP client.
//!
//! [`LocalhostDnsResolver`] pins RFC 6761 `.localhost` names (e.g.
//! `admin.localhost`) to the loopback address (`127.0.0.1`) and delegates
//! all other names to the system getaddrinfo resolver via a blocking thread.
//!
//! # Why a custom resolver
//!
//! The `http://admin.localhost:<port>` origin is the canonical form used by
//! the relay's `just admin` target. RFC 6761 §6.3 requires `.localhost`
//! subdomains to resolve to loopback, but system getaddrinfo does not honour
//! this on all supported platforms:
//!
//! - **macOS**: resolves correctly via mDNSResponder.
//! - **Linux/glibc**: relies on `nsswitch.conf` ordering; GitHub Actions
//!   ubuntu-latest runners do NOT resolve `.localhost` subdomains.
//! - **Windows**: not guaranteed by the system resolver.
//!
//! The resolver intercepts only names ending in `.localhost` and returns
//! `127.0.0.1:0`; all other names fall through to the system resolver,
//! so non-localhost resolution is unchanged.

use std::net::SocketAddr;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};

/// DNS resolver used by the admin HTTP client.
///
/// - Names ending in `.localhost` are pinned to `127.0.0.1:0` (RFC 6761 §6.3).
/// - All other names are forwarded to the system `getaddrinfo` resolver.
#[derive(Debug, Clone)]
pub struct LocalhostDnsResolver;

impl Resolve for LocalhostDnsResolver {
    fn resolve(&self, name: Name) -> Resolving {
        // RFC 6761 §6.3: `.localhost` subdomains must resolve to loopback.
        // The admin client's exact `resolve("localhost", …)` already covers the
        // bare hostname; this resolver handles the subdomain case.
        if name.as_str().ends_with(".localhost") {
            let addrs: Addrs = Box::new(std::iter::once(SocketAddr::from(([127, 0, 0, 1], 0))));
            return Box::pin(async move { Ok(addrs) });
        }

        // Fall through to the system resolver for all other names.
        let host = name.as_str().to_owned();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                use std::net::ToSocketAddrs;
                let addrs = (host.as_str(), 0u16)
                    .to_socket_addrs()
                    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
                Ok::<Addrs, Box<dyn std::error::Error + Send + Sync>>(Box::new(addrs))
            })
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?
        })
    }
}
