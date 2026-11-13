//! SSH jump-chain transport for CoShell.
//!
//! `Local → hop₁ → … → target`: the first hop is a TCP connection, every
//! following hop is an SSH session tunnelled through a `direct-tcpip` channel
//! of the previous one (OpenSSH `ProxyJump` semantics). Every hop's host key is
//! verified against `known_hosts` independently.

mod auth;
pub mod config;
mod forward;
mod hostkey;
mod prompt;
mod shell;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use russh::client;
use zeroize::Zeroizing;

pub use hostkey::HostKeyPolicy;
pub use prompt::{
    BoxFuture, HostKeyDecision, HostKeyKind, HostKeyQuery, InteractivePrompt, InteractiveQuery, Progress, Prompter,
};
pub use forward::{ForwardSpec, Tunnel, TunnelStats};
pub use shell::{SshTransport, open_shell};

/// Default SSH port.
pub const DEFAULT_PORT: u16 = 22;
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const DEFAULT_KEEPALIVE: Duration = Duration::from_secs(30);
/// Missed keepalives before the connection is considered dead.
const KEEPALIVE_MAX: usize = 3;
/// Extra handshake time allowed while a host-key question is on screen.
const PROMPT_GRACE: Duration = Duration::from_secs(300);

/// How to authenticate to one hop.
#[derive(Clone)]
pub enum Auth {
    /// Keys held by ssh-agent (`SSH_AUTH_SOCK`).
    Agent,
    /// Private key file, optionally encrypted.
    KeyFile { path: PathBuf, passphrase: Option<Zeroizing<String>> },
    Password(Zeroizing<String>),
    /// Ask the [`Prompter`] (keyboard-interactive / password).
    Interactive,
    /// OpenSSH-like: agent → key file (`identity` or ~/.ssh defaults) →
    /// saved `password` (tried once) → prompts.
    Auto { identity: Option<PathBuf>, password: Option<Zeroizing<String>> },
}

impl std::fmt::Debug for Auth {
    // Never print secrets.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Auth::Agent => f.write_str("Agent"),
            Auth::KeyFile { path, .. } => write!(f, "KeyFile({})", path.display()),
            Auth::Password(_) => f.write_str("Password(***)"),
            Auth::Interactive => f.write_str("Interactive"),
            Auth::Auto { identity, password } => {
                write!(f, "Auto({identity:?}, password: {})", if password.is_some() { "***" } else { "none" })
            }
        }
    }
}

/// One machine in the chain.
#[derive(Clone, Debug)]
pub struct HostSpec {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: Auth,
}

impl HostSpec {
    /// `user@host:port` for messages.
    pub fn label(&self) -> String {
        if self.port == DEFAULT_PORT {
            format!("{}@{}", self.user, self.host)
        } else {
            format!("{}@{}:{}", self.user, self.host, self.port)
        }
    }
}

#[derive(Clone, Debug)]
pub struct ConnectOptions {
    pub known_hosts: PathBuf,
    pub policy: HostKeyPolicy,
    pub connect_timeout: Duration,
    pub keepalive: Duration,
}

impl ConnectOptions {
    /// `~/.ssh/known_hosts`, strict checking.
    pub fn new(known_hosts: PathBuf) -> Self {
        Self {
            known_hosts,
            policy: HostKeyPolicy::Strict,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            keepalive: DEFAULT_KEEPALIVE,
        }
    }

    pub fn default_known_hosts() -> Option<PathBuf> {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".ssh").join("known_hosts"))
    }
}

static ENGLISH: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Word user-facing messages (errors, prompts) in English instead of Korean.
pub fn set_english(on: bool) {
    ENGLISH.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub(crate) fn english() -> bool {
    ENGLISH.load(std::sync::atomic::Ordering::Relaxed)
}

#[derive(Debug)]
pub enum SshError {
    UnknownHostKey { host: String, port: u16, algorithm: String, fingerprint: String },
    HostKeyChanged { host: String, port: u16, line: usize, fingerprint: String },
    HostCertificateUnsupported { host: String },
    KnownHosts(String),
    Cancelled(String),
    AuthFailed(String),
    Agent { hop: String, reason: String },
    KeyFile { hop: String, reason: String },
    Timeout(String),
    Ssh { hop: String, source: russh::Error },
    Forward(String),
    PortInUse(u16),
    EmptyChain,
}

impl std::fmt::Display for SshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if english() {
            return match self {
                Self::UnknownHostKey { host, port, algorithm, fingerprint } => {
                    write!(f, "The host key of {host}:{port} is not in known_hosts ({algorithm} {fingerprint})")
                }
                Self::HostKeyChanged { host, port, line, fingerprint } => write!(
                    f,
                    "The host key of {host}:{port} differs from line {line} of known_hosts ({fingerprint}). Stopping: this may be a man-in-the-middle attack"
                ),
                Self::HostCertificateUnsupported { host } => write!(f, "{host}: host certificates are not supported yet"),
                Self::KnownHosts(e) => write!(f, "Could not process known_hosts: {e}"),
                Self::Cancelled(hop) => write!(f, "{hop}: cancelled by the user"),
                Self::AuthFailed(hop) => write!(f, "{hop}: authentication failed"),
                Self::Agent { hop, reason } => write!(f, "{hop}: ssh-agent is unavailable ({reason})"),
                Self::KeyFile { hop, reason } => write!(f, "{hop}: could not read the key file ({reason})"),
                Self::Timeout(hop) => write!(f, "{hop}: connection timed out"),
                Self::Ssh { hop, source } => write!(f, "{hop}: {source}"),
                Self::Forward(e) => write!(f, "Forwarding failed: {e}"),
                Self::PortInUse(port) => write!(f, "Port {port} on this PC is already in use"),
                Self::EmptyChain => write!(f, "Empty connection route"),
            };
        }
        match self {
            Self::UnknownHostKey { host, port, algorithm, fingerprint } => {
                write!(f, "{host}:{port} 의 호스트 키가 known_hosts에 없습니다 ({algorithm} {fingerprint})")
            }
            Self::HostKeyChanged { host, port, line, fingerprint } => write!(
                f,
                "{host}:{port} 의 호스트 키가 known_hosts {line}행과 다릅니다 ({fingerprint}). 중간자 공격 가능성이 있어 접속을 중단합니다"
            ),
            Self::HostCertificateUnsupported { host } => write!(f, "{host}: 호스트 인증서는 아직 지원하지 않습니다"),
            Self::KnownHosts(e) => write!(f, "known_hosts 처리 실패: {e}"),
            Self::Cancelled(hop) => write!(f, "{hop}: 사용자가 취소했습니다"),
            Self::AuthFailed(hop) => write!(f, "{hop}: 인증 실패"),
            Self::Agent { hop, reason } => write!(f, "{hop}: ssh-agent 사용 불가 ({reason})"),
            Self::KeyFile { hop, reason } => write!(f, "{hop}: 키 파일을 읽을 수 없습니다 ({reason})"),
            Self::Timeout(hop) => write!(f, "{hop}: 연결 시간 초과"),
            Self::Ssh { hop, source } => write!(f, "{hop}: {source}"),
            Self::Forward(e) => write!(f, "포워딩 실패: {e}"),
            Self::PortInUse(port) => write!(f, "이 PC의 {port}번 포트는 이미 사용 중입니다"),
            Self::EmptyChain => write!(f, "빈 연결 경로"),
        }
    }
}

impl std::error::Error for SshError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Ssh { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub(crate) type Handle = client::Handle<hostkey::ClientHandler>;

/// An authenticated chain; the last handle is the target. Cheap to clone;
/// the connection lives until [`Connection::disconnect`] or the last clone drops.
#[derive(Clone)]
pub struct Connection(Arc<Inner>);

struct Inner {
    handles: Vec<Handle>,
    labels: Vec<String>,
    /// `-R` routing table of the target hop's handler.
    routes: forward::RemoteRoutes,
}

impl Connection {
    pub fn labels(&self) -> &[String] {
        &self.0.labels
    }

    pub(crate) fn target(&self) -> &Handle {
        self.0.handles.last().expect("non-empty chain")
    }

    pub(crate) fn routes(&self) -> &forward::RemoteRoutes {
        &self.0.routes
    }

    /// Close every hop, target first.
    pub async fn disconnect(&self) {
        for h in self.0.handles.iter().rev() {
            let _ = h.disconnect(russh::Disconnect::ByApplication, "", "en").await;
        }
    }
}

fn client_config(opts: &ConnectOptions) -> Arc<client::Config> {
    Arc::new(client::Config {
        inactivity_timeout: None,
        keepalive_interval: Some(opts.keepalive),
        keepalive_max: KEEPALIVE_MAX,
        nodelay: true,
        ..Default::default()
    })
}

/// Connect and authenticate every hop in order (no interactive prompts).
pub async fn connect_chain(chain: &[HostSpec], opts: &ConnectOptions) -> Result<Connection, SshError> {
    connect_chain_with(chain, opts, None).await
}

/// Like [`connect_chain`], asking `prompter` for host keys / 2FA and reporting progress.
pub async fn connect_chain_with(
    chain: &[HostSpec],
    opts: &ConnectOptions,
    prompter: Option<Arc<dyn Prompter>>,
) -> Result<Connection, SshError> {
    let notify = |p: Progress| {
        if let Some(pr) = &prompter {
            pr.progress(p);
        }
    };
    if chain.is_empty() {
        return Err(SshError::EmptyChain);
    }
    let mut handles: Vec<Handle> = Vec::with_capacity(chain.len());
    let mut routes = forward::RemoteRoutes::default();
    for (index, hop) in chain.iter().enumerate() {
        let label = hop.label();
        notify(Progress::Connecting { hop: index });
        let started = Instant::now();
        let handler = hostkey::ClientHandler::new(index, hop, opts, prompter.clone());
        let issue = handler.issue();
        let waited = handler.waited();
        routes = handler.routes();
        let config = client_config(opts);

        let connecting = async {
            match handles.last() {
                None => client::connect(config, (hop.host.as_str(), hop.port), handler).await,
                Some(prev) => {
                    let tunnel = prev
                        .channel_open_direct_tcpip(hop.host.clone(), u32::from(hop.port), "127.0.0.1", 0)
                        .await?;
                    client::connect_stream(config, tunnel.into_stream(), handler).await
                }
            }
        };
        // Host-key questions wait for a human; the deadline must not count that time.
        let deadline = if opts.policy == HostKeyPolicy::Ask && prompter.is_some() {
            opts.connect_timeout + PROMPT_GRACE
        } else {
            opts.connect_timeout
        };
        let mut handle = match tokio::time::timeout(deadline, connecting).await {
            Err(_) => return Err(SshError::Timeout(label)),
            Ok(Err(e)) => {
                // A rejected host key surfaces as a generic russh error; report the real reason.
                return Err(issue.lock().ok().and_then(|mut i| i.take()).unwrap_or(SshError::Ssh { hop: label, source: e }));
            }
            Ok(Ok(h)) => h,
        };
        let latency = started.elapsed().saturating_sub(waited.lock().map(|w| *w).unwrap_or_default());
        notify(Progress::Authenticating { hop: index });
        auth::authenticate(&mut handle, hop, index, prompter.as_ref()).await?;
        notify(Progress::Authenticated { hop: index, latency });
        handles.push(handle);
    }
    Ok(Connection(Arc::new(Inner { handles, labels: chain.iter().map(HostSpec::label).collect(), routes })))
}

#[cfg(test)]
mod language_tests {
    use super::*;

    #[test]
    fn errors_are_worded_in_the_selected_language() {
        let e = SshError::PortInUse(3306);
        assert_eq!(e.to_string(), "이 PC의 3306번 포트는 이미 사용 중입니다");
        set_english(true);
        let english = (e.to_string(), SshError::AuthFailed("u@h".into()).to_string(), forward::bad_port("x"));
        set_english(false);
        assert_eq!(english.0, "Port 3306 on this PC is already in use");
        assert_eq!(english.1, "u@h: authentication failed");
        assert_eq!(english.2, "Invalid port: x");
    }
}
