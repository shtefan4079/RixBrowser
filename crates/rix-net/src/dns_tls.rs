
use crate::error::{NetError, Result};
use hickory_resolver::config::{ResolverConfig, ResolverOpts};
use hickory_resolver::TokioAsyncResolver;
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, RootCertStore};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;
use tokio_rustls::TlsConnector;

pub const ALPN_PROTOCOLS: &[&[u8]] = &[b"h2", b"http/1.1"];

#[derive(Clone)]
pub struct DnsResolver {
    inner: TokioAsyncResolver,
    cache: Arc<RwLock<HashMap<String, (Option<Vec<IpAddr>>, Instant)>>>,
}

impl DnsResolver {
    pub fn new() -> Result<Self> {
        let (config, opts) =
            hickory_resolver::system_conf::read_system_conf().map_err(|error| {
                NetError::Dns(format!("System DNS configuration unavailable: {error}"))
            })?;
        Ok(Self::from_config(config, opts))
    }

    pub fn from_config(config: ResolverConfig, mut opts: ResolverOpts) -> Self {
        opts.cache_size = 1024;
        opts.timeout = Duration::from_millis(500);
        opts.attempts = 2;
        let resolver = TokioAsyncResolver::tokio(config, opts);
        Self {
            inner: resolver,
            cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn resolve(&self, host: &str) -> Result<Vec<IpAddr>> {
        let host = host
            .strip_prefix('[')
            .and_then(|host| host.strip_suffix(']'))
            .unwrap_or(host);
        if let Ok(ip) = host.parse::<IpAddr>() {
            return Ok(vec![ip]);
        }

        if let Ok(guard) = self.cache.read() {
            if let Some((entry, expiry)) = guard.get(host) {
                if Instant::now() < *expiry {
                    return match entry {
                        Some(ips) => Ok(ips.clone()),
                        None => Err(NetError::Dns(format!("No IP addresses found for '{host}'"))),
                    };
                }
            }
        }

        let mut addrs = Vec::new();
        if let Ok(response) = self.inner.ipv4_lookup(host).await {
            for ip in response.iter() {
                addrs.push(IpAddr::V4(ip.0));
            }
            if let Ok(Ok(response)) = tokio::time::timeout(Duration::from_millis(50), self.inner.ipv6_lookup(host)).await {
                for ip in response.iter() {
                    addrs.push(IpAddr::V6(ip.0));
                }
            }
        } else if let Ok(Ok(response)) = tokio::time::timeout(Duration::from_millis(50), self.inner.ipv6_lookup(host)).await {
            for ip in response.iter() {
                addrs.push(IpAddr::V6(ip.0));
            }
        }

        if addrs.is_empty() {
            if let Ok(mut guard) = self.cache.write() {
                guard.insert(
                    host.to_string(),
                    (None, Instant::now() + Duration::from_secs(30)),
                );
            }
            return Err(NetError::Dns(format!("No IP addresses found for '{host}'")));
        }

        if let Ok(mut guard) = self.cache.write() {
            guard.insert(
                host.to_string(),
                (Some(addrs.clone()), Instant::now() + Duration::from_secs(300)),
            );
        }

        Ok(addrs)
    }

    pub async fn resolve_socket_addrs(&self, host: &str, port: u16) -> Result<Vec<SocketAddr>> {
        let ips = self.resolve(host).await?;
        Ok(ips
            .into_iter()
            .map(|ip| SocketAddr::new(ip, port))
            .collect())
    }
}

#[derive(Clone)]
pub struct TlsManager {
    config: Arc<ClientConfig>,
    connector: TlsConnector,
}

impl TlsManager {
    pub fn new() -> Result<Self> {
        let mut root_store = RootCertStore::empty();
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

        let mut client_config = ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();

        client_config.alpn_protocols = ALPN_PROTOCOLS.iter().map(|p| p.to_vec()).collect();

        let config = Arc::new(client_config);
        let connector = TlsConnector::from(config.clone());

        Ok(Self { config, connector })
    }

    pub fn config(&self) -> &Arc<ClientConfig> {
        &self.config
    }

    pub fn quinn_client_config(&self) -> quinn::ClientConfig {
        let mut config = (*self.config).clone();
        config.alpn_protocols = vec![b"h3".to_vec()];
        let quic_crypto = quinn::crypto::rustls::QuicClientConfig::try_from(Arc::new(config))
            .expect("rustls-ring provides TLS13_AES_128_GCM_SHA256 for QUIC");
        quinn::ClientConfig::new(std::sync::Arc::new(quic_crypto))
    }

    pub fn negotiated_alpn(protocol: Option<&[u8]>) -> NegotiatedProtocol {
        match protocol {
            Some(b"h3") => NegotiatedProtocol::H3,
            Some(b"h2") => NegotiatedProtocol::H2,
            _ => NegotiatedProtocol::Http11,
        }
    }

    pub async fn connect(&self, domain: &str, stream: TcpStream) -> Result<TlsStream<TcpStream>> {
        let server_name = ServerName::try_from(domain.to_string())
            .map_err(|e| NetError::Tls(format!("Invalid DNS name for TLS '{domain}': {e}")))?;

        let tls_stream = self
            .connector
            .connect(server_name, stream)
            .await
            .map_err(|e| NetError::Tls(format!("TLS handshake failed with '{domain}': {e}")))?;

        Ok(tls_stream)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum NegotiatedProtocol {
    H3,
    H2,
    Http11,
}

impl NegotiatedProtocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::H3 => "h3",
            Self::H2 => "h2",
            Self::Http11 => "http/1.1",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_resolve_ip_literals() {
        let resolver = DnsResolver::new().expect("resolver init");
        let v4 = resolver.resolve("127.0.0.1").await.unwrap();
        assert_eq!(v4, vec!["127.0.0.1".parse::<IpAddr>().unwrap()]);

        let v6 = resolver.resolve("[::1]").await.unwrap();
        assert_eq!(v6, vec!["::1".parse::<IpAddr>().unwrap()]);
    }
}
