//! Port forwarding over an established connection:
//! `-L` local → remote, `-R` remote → local, `-D` SOCKS5 dynamic.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{JoinHandle, JoinSet};

use crate::{Connection, SshError};

const COPY_BUF: usize = 16 * 1024;
const SOCKS_VERSION: u8 = 5;
const SOCKS_NO_AUTH: u8 = 0;
const SOCKS_NO_ACCEPTABLE: u8 = 0xFF;
const SOCKS_CMD_CONNECT: u8 = 1;
const SOCKS_OK: u8 = 0;
const SOCKS_FAILURE: u8 = 1;
const SOCKS_CONN_REFUSED: u8 = 5;
const SOCKS_CMD_UNSUPPORTED: u8 = 7;
const SOCKS_ATYP_UNSUPPORTED: u8 = 8;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ForwardSpec {
    /// `-L [bind:]port:host:hostport` — listen here, connect from the target.
    Local { bind: String, port: u16, host: String, host_port: u16 },
    /// `-R [bind:]port:host:hostport` — listen on the target, connect from here.
    Remote { bind: String, port: u16, host: String, host_port: u16 },
    /// `-D [bind:]port` — SOCKS5 proxy here, connections leave from the target.
    Dynamic { bind: String, port: u16 },
}

/// "Invalid port" message for `what` (a port or the spec containing it).
pub(crate) fn bad_port(what: &str) -> String {
    if crate::english() { format!("Invalid port: {what}") } else { format!("잘못된 포트: {what}") }
}

impl ForwardSpec {
    pub fn kind(&self) -> char {
        match self {
            ForwardSpec::Local { .. } => 'L',
            ForwardSpec::Remote { .. } => 'R',
            ForwardSpec::Dynamic { .. } => 'D',
        }
    }

    /// Listening on something other than loopback exposes the tunnel to the network.
    pub fn is_exposed(&self) -> bool {
        let bind = match self {
            ForwardSpec::Local { bind, .. } | ForwardSpec::Remote { bind, .. } | ForwardSpec::Dynamic { bind, .. } => bind,
        };
        !matches!(bind.as_str(), "127.0.0.1" | "localhost" | "::1" | "")
    }

    /// Parse OpenSSH-style specs: `L:[bind:]port:host:hostport`, `R:…`, `D:[bind:]port`.
    pub fn parse(kind: char, s: &str) -> Result<Self, String> {
        let parts: Vec<&str> = s.split(':').collect();
        let port = |p: &str| p.trim().parse::<u16>().map_err(|_| bad_port(p));
        let lo = || "127.0.0.1".to_string();
        match (kind.to_ascii_uppercase(), parts.as_slice()) {
            ('L', [p, h, hp]) => Ok(Self::Local { bind: lo(), port: port(p)?, host: h.to_string(), host_port: port(hp)? }),
            ('L', [b, p, h, hp]) => Ok(Self::Local { bind: b.to_string(), port: port(p)?, host: h.to_string(), host_port: port(hp)? }),
            ('R', [p, h, hp]) => Ok(Self::Remote { bind: lo(), port: port(p)?, host: h.to_string(), host_port: port(hp)? }),
            ('R', [b, p, h, hp]) => Ok(Self::Remote { bind: b.to_string(), port: port(p)?, host: h.to_string(), host_port: port(hp)? }),
            ('D', [p]) => Ok(Self::Dynamic { bind: lo(), port: port(p)? }),
            ('D', [b, p]) => Ok(Self::Dynamic { bind: b.to_string(), port: port(p)? }),
            _ if crate::english() => Err(format!("Invalid tunnel format: {kind} {s}")),
            _ => Err(format!("터널 형식이 올바르지 않습니다: {kind} {s}")),
        }
    }
}

impl std::fmt::Display for ForwardSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ForwardSpec::Local { bind, port, host, host_port } => write!(f, "{bind}:{port} → {host}:{host_port}"),
            ForwardSpec::Remote { bind, port, host, host_port } => write!(f, "remote {bind}:{port} → {host}:{host_port}"),
            ForwardSpec::Dynamic { bind, port } => write!(f, "{bind}:{port} SOCKS5"),
        }
    }
}

#[derive(Default, Debug)]
pub struct TunnelStats {
    pub up: AtomicU64,
    pub down: AtomicU64,
    pub open: AtomicU64,
    pub total: AtomicU64,
    pub last_error: Mutex<Option<String>>,
}

impl TunnelStats {
    fn error(&self, e: impl ToString) {
        if let Ok(mut l) = self.last_error.lock() {
            *l = Some(e.to_string());
        }
    }
}

/// Destination for connections the server forwards to us (`-R`).
#[derive(Clone)]
pub(crate) struct RemoteRoute {
    host: String,
    port: u16,
    stats: Arc<TunnelStats>,
}

/// Server-bound port → local destination.
pub(crate) type RemoteRoutes = Arc<Mutex<HashMap<u32, RemoteRoute>>>;

/// A running forward. Dropping it stops listening and closes its connections
/// (call [`Tunnel::stop`] first for `-R` to also release the server port).
pub struct Tunnel {
    pub spec: ForwardSpec,
    /// Port actually bound (differs from the spec when it asked for 0).
    pub bound_port: u16,
    pub stats: Arc<TunnelStats>,
    task: Option<JoinHandle<()>>,
    conn: Connection,
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        if let Some(t) = self.task.take() {
            t.abort();
        }
        if let ForwardSpec::Remote { .. } = self.spec
            && let Ok(mut r) = self.conn.routes().lock()
        {
            r.remove(&u32::from(self.bound_port));
        }
    }
}

impl Tunnel {
    /// Stop; for `-R` also cancel the server-side listener.
    pub async fn stop(mut self) {
        if let Some(t) = self.task.take() {
            t.abort();
        }
        if let ForwardSpec::Remote { bind, .. } = &self.spec {
            let _ = self.conn.target().cancel_tcpip_forward(bind.clone(), u32::from(self.bound_port)).await;
        }
    }
}

/// Copy both directions, counting bytes as they flow.
pub(crate) async fn pipe<A, B>(a: A, b: B, stats: Arc<TunnelStats>)
where
    A: AsyncRead + AsyncWrite + Unpin,
    B: AsyncRead + AsyncWrite + Unpin,
{
    async fn copy<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(mut r: R, mut w: W, counter: &AtomicU64) {
        let mut buf = vec![0u8; COPY_BUF];
        loop {
            match r.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if w.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                    counter.fetch_add(n as u64, Ordering::Relaxed);
                }
            }
        }
        let _ = w.shutdown().await;
    }
    stats.open.fetch_add(1, Ordering::Relaxed);
    stats.total.fetch_add(1, Ordering::Relaxed);
    let (ar, aw) = tokio::io::split(a);
    let (br, bw) = tokio::io::split(b);
    // a = local side, b = SSH side: a→b is "up".
    tokio::join!(copy(ar, bw, &stats.up), copy(br, aw, &stats.down));
    stats.open.fetch_sub(1, Ordering::Relaxed);
}

impl Connection {
    /// Start a forward. Must run inside a tokio runtime.
    pub async fn start_forward(&self, spec: ForwardSpec) -> Result<Tunnel, SshError> {
        let stats = Arc::new(TunnelStats::default());
        let io = |e: std::io::Error| match (&spec, e.kind()) {
            (ForwardSpec::Local { port, .. } | ForwardSpec::Dynamic { port, .. }, std::io::ErrorKind::AddrInUse) => SshError::PortInUse(*port),
            _ => SshError::Forward(format!("{spec}: {e}")),
        };
        let (bound_port, task) = match &spec {
            ForwardSpec::Local { bind, port, host, host_port } => {
                let listener = TcpListener::bind((bind.as_str(), *port)).await.map_err(io)?;
                let bound = listener.local_addr().map_err(io)?.port();
                let (conn, st, host, hp) = (self.clone(), stats.clone(), host.clone(), *host_port);
                (bound, tokio::spawn(accept_loop(listener, st, move |sock, peer, st| {
                    let (conn, host) = (conn.clone(), host.clone());
                    async move {
                        match conn.target().channel_open_direct_tcpip(host, u32::from(hp), peer.ip().to_string(), u32::from(peer.port())).await {
                            Ok(ch) => pipe(sock, ch.into_stream(), st).await,
                            Err(e) => st.error(e),
                        }
                    }
                })))
            }
            ForwardSpec::Dynamic { bind, port } => {
                let listener = TcpListener::bind((bind.as_str(), *port)).await.map_err(io)?;
                let bound = listener.local_addr().map_err(io)?.port();
                let conn = self.clone();
                (bound, tokio::spawn(accept_loop(listener, stats.clone(), move |sock, peer, st| {
                    let conn = conn.clone();
                    async move {
                        if let Err(e) = socks5(sock, peer, &conn, st.clone()).await {
                            st.error(e);
                        }
                    }
                })))
            }
            ForwardSpec::Remote { bind, port, host, host_port } => {
                let bound = self
                    .target()
                    .tcpip_forward(bind.clone(), u32::from(*port))
                    .await
                    .map_err(|e| {
                        SshError::Forward(if crate::english() {
                            format!("{spec}: refused by the server ({e})")
                        } else {
                            format!("{spec}: 서버가 거부했습니다 ({e})")
                        })
                    })?;
                // OpenSSH replies 0 when the requested (non-zero) port was used as-is.
                let bound = if bound == 0 { u32::from(*port) } else { bound };
                if let Ok(mut r) = self.routes().lock() {
                    r.insert(bound, RemoteRoute { host: host.clone(), port: *host_port, stats: stats.clone() });
                }
                (bound as u16, tokio::spawn(std::future::pending::<()>()))
            }
        };
        Ok(Tunnel { spec, bound_port, stats, task: Some(task), conn: self.clone() })
    }
}

impl Connection {
    /// Like [`Connection::start_forward`], but if the requested local port is
    /// taken, listen on a free one instead (see [`Tunnel::bound_port`]).
    pub async fn start_forward_auto(&self, spec: ForwardSpec) -> Result<Tunnel, SshError> {
        match self.start_forward(spec.clone()).await {
            Err(SshError::PortInUse(_)) => {
                let any = match spec {
                    ForwardSpec::Local { bind, host, host_port, .. } => ForwardSpec::Local { bind, port: 0, host, host_port },
                    ForwardSpec::Dynamic { bind, .. } => ForwardSpec::Dynamic { bind, port: 0 },
                    remote => remote,
                };
                self.start_forward(any).await
            }
            r => r,
        }
    }
}

/// Accept connections; each is handled in a child task that dies with the loop.
async fn accept_loop<F, Fut>(listener: TcpListener, stats: Arc<TunnelStats>, handle: F)
where
    F: Fn(TcpStream, SocketAddr, Arc<TunnelStats>) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let mut children = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((sock, peer)) => {
                    let _ = sock.set_nodelay(true);
                    children.spawn(handle(sock, peer, stats.clone()));
                }
                Err(e) => stats.error(e),
            },
            Some(_) = children.join_next(), if !children.is_empty() => {}
        }
    }
}

/// Handle one forwarded-tcpip channel from the server (`-R`).
pub(crate) fn serve_remote(routes: &RemoteRoutes, connected_port: u32) -> Option<(String, u16, Arc<TunnelStats>)> {
    let r = routes.lock().ok()?;
    r.get(&connected_port).map(|x| (x.host.clone(), x.port, x.stats.clone()))
}

/// Minimal SOCKS5 (RFC 1928): no auth, CONNECT, IPv4 / domain / IPv6.
async fn socks5(mut sock: TcpStream, peer: SocketAddr, conn: &Connection, stats: Arc<TunnelStats>) -> std::io::Result<()> {
    let mut head = [0u8; 2];
    sock.read_exact(&mut head).await?;
    if head[0] != SOCKS_VERSION {
        return Err(std::io::Error::other(if crate::english() { "The SOCKS version is not 5" } else { "SOCKS 버전이 5가 아닙니다" }));
    }
    let mut methods = vec![0u8; head[1] as usize];
    sock.read_exact(&mut methods).await?;
    if !methods.contains(&SOCKS_NO_AUTH) {
        sock.write_all(&[SOCKS_VERSION, SOCKS_NO_ACCEPTABLE]).await?;
        return Ok(());
    }
    sock.write_all(&[SOCKS_VERSION, SOCKS_NO_AUTH]).await?;

    let mut req = [0u8; 4];
    sock.read_exact(&mut req).await?;
    let reply = |code: u8| [SOCKS_VERSION, code, 0, 1, 0, 0, 0, 0, 0, 0];
    if req[1] != SOCKS_CMD_CONNECT {
        sock.write_all(&reply(SOCKS_CMD_UNSUPPORTED)).await?;
        return Ok(());
    }
    let host = match req[3] {
        1 => {
            let mut a = [0u8; 4];
            sock.read_exact(&mut a).await?;
            std::net::Ipv4Addr::from(a).to_string()
        }
        3 => {
            let mut len = [0u8; 1];
            sock.read_exact(&mut len).await?;
            let mut name = vec![0u8; len[0] as usize];
            sock.read_exact(&mut name).await?;
            String::from_utf8_lossy(&name).into_owned()
        }
        4 => {
            let mut a = [0u8; 16];
            sock.read_exact(&mut a).await?;
            std::net::Ipv6Addr::from(a).to_string()
        }
        _ => {
            sock.write_all(&reply(SOCKS_ATYP_UNSUPPORTED)).await?;
            return Ok(());
        }
    };
    let mut port = [0u8; 2];
    sock.read_exact(&mut port).await?;
    let port = u16::from_be_bytes(port);

    match conn.target().channel_open_direct_tcpip(host, u32::from(port), peer.ip().to_string(), u32::from(peer.port())).await {
        Ok(ch) => {
            sock.write_all(&reply(SOCKS_OK)).await?;
            pipe(sock, ch.into_stream(), stats).await;
            Ok(())
        }
        Err(russh::Error::ChannelOpenFailure(_)) => {
            sock.write_all(&reply(SOCKS_CONN_REFUSED)).await?;
            Ok(())
        }
        Err(e) => {
            sock.write_all(&reply(SOCKS_FAILURE)).await?;
            Err(std::io::Error::other(e.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_openssh_style() {
        assert_eq!(
            ForwardSpec::parse('L', "8080:127.0.0.1:80"),
            Ok(ForwardSpec::Local { bind: "127.0.0.1".into(), port: 8080, host: "127.0.0.1".into(), host_port: 80 })
        );
        assert_eq!(
            ForwardSpec::parse('R', "0.0.0.0:9000:localhost:3000"),
            Ok(ForwardSpec::Remote { bind: "0.0.0.0".into(), port: 9000, host: "localhost".into(), host_port: 3000 })
        );
        assert_eq!(ForwardSpec::parse('D', "1080"), Ok(ForwardSpec::Dynamic { bind: "127.0.0.1".into(), port: 1080 }));
        assert!(ForwardSpec::parse('L', "8080").is_err());
        assert!(ForwardSpec::parse('D', "notaport").is_err());
        assert!(ForwardSpec::parse('R', "0.0.0.0:9000:localhost:3000").unwrap().is_exposed());
        assert!(!ForwardSpec::parse('D', "1080").unwrap().is_exposed());
    }
}
