//! Server host key verification against `known_hosts`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use russh::client;
use russh::keys::{self, HashAlg, PublicKey, PublicKeyOrCertificate};

use crate::forward::{self, RemoteRoutes};
use crate::prompt::{HostKeyDecision, HostKeyKind, HostKeyQuery, Prompter};
use crate::{ConnectOptions, HostSpec, SshError};

/// Guard against pathological files with many stale lines for one host.
const MAX_STALE_LINES: usize = 16;

/// Behaviour for keys not matching `known_hosts`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostKeyPolicy {
    /// Reject unknown and changed keys (OpenSSH `StrictHostKeyChecking=yes`).
    Strict,
    /// Record unknown keys silently; reject changed keys (OpenSSH `accept-new`).
    AcceptNew,
    /// Ask the [`Prompter`] for unknown and changed keys.
    Ask,
}

pub(crate) struct ClientHandler {
    hop: usize,
    host: String,
    port: u16,
    known_hosts: PathBuf,
    policy: HostKeyPolicy,
    prompter: Option<Arc<dyn Prompter>>,
    issue: Arc<Mutex<Option<SshError>>>,
    waited: Arc<Mutex<Duration>>,
    routes: RemoteRoutes,
}

impl ClientHandler {
    pub(crate) fn new(hop_index: usize, hop: &HostSpec, opts: &ConnectOptions, prompter: Option<Arc<dyn Prompter>>) -> Self {
        Self {
            hop: hop_index,
            host: hop.host.clone(),
            port: hop.port,
            known_hosts: opts.known_hosts.clone(),
            policy: opts.policy,
            prompter,
            issue: Arc::new(Mutex::new(None)),
            waited: Arc::new(Mutex::new(Duration::ZERO)),
            routes: RemoteRoutes::default(),
        }
    }

    /// Why the key was rejected, filled in before russh aborts the handshake.
    pub(crate) fn issue(&self) -> Arc<Mutex<Option<SshError>>> {
        self.issue.clone()
    }

    pub(crate) fn routes(&self) -> RemoteRoutes {
        self.routes.clone()
    }

    /// Time spent waiting for the user (excluded from latency).
    pub(crate) fn waited(&self) -> Arc<Mutex<Duration>> {
        self.waited.clone()
    }

    fn reject(&self, e: SshError) -> bool {
        if let Ok(mut slot) = self.issue.lock() {
            *slot = Some(e);
        }
        false
    }

    fn learn(&self, key: &PublicKey) -> bool {
        match keys::known_hosts::learn_known_hosts_path(&self.host, self.port, key, &self.known_hosts) {
            Ok(()) => true,
            Err(e) => self.reject(SshError::KnownHosts(e.to_string())),
        }
    }

    async fn ask(&self, key: &PublicKey, kind: HostKeyKind) -> Option<HostKeyDecision> {
        let prompter = self.prompter.as_ref().filter(|_| self.policy == HostKeyPolicy::Ask)?;
        let query = HostKeyQuery {
            hop: self.hop,
            host: self.host.clone(),
            port: self.port,
            algorithm: key.algorithm().to_string(),
            fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
            kind,
        };
        let t = Instant::now();
        let decision = prompter.host_key(query).await;
        if let Ok(mut w) = self.waited.lock() {
            *w += t.elapsed();
        }
        Some(decision)
    }

    async fn verify(&self, key: &PublicKey) -> bool {
        let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
        let (host, port) = (self.host.clone(), self.port);
        match keys::check_known_hosts_path(&self.host, self.port, key, &self.known_hosts) {
            Ok(true) => true,
            Ok(false) => match self.policy {
                HostKeyPolicy::AcceptNew => self.learn(key),
                HostKeyPolicy::Strict => self.reject(SshError::UnknownHostKey {
                    host,
                    port,
                    algorithm: key.algorithm().to_string(),
                    fingerprint,
                }),
                HostKeyPolicy::Ask => match self.ask(key, HostKeyKind::Unknown).await {
                    Some(HostKeyDecision::AcceptOnce) => true,
                    Some(HostKeyDecision::AcceptAndSave) => self.learn(key),
                    Some(HostKeyDecision::Reject | HostKeyDecision::ReplaceAndSave) => {
                        self.reject(SshError::Cancelled(format!("{host}:{port}")))
                    }
                    None => self.reject(SshError::UnknownHostKey {
                        host,
                        port,
                        algorithm: key.algorithm().to_string(),
                        fingerprint,
                    }),
                },
            },
            Err(keys::Error::KeyChanged { line }) => {
                // Only an explicit Replace may proceed; "once"/"save" are never valid for a changed key.
                if self.ask(key, HostKeyKind::Changed { line }).await == Some(HostKeyDecision::ReplaceAndSave) {
                    match replace_known_host(&self.known_hosts, &self.host, self.port, key) {
                        Ok(()) => true,
                        Err(e) => self.reject(e),
                    }
                } else {
                    self.reject(SshError::HostKeyChanged { host, port, line, fingerprint })
                }
            }
            Err(e) => self.reject(SshError::KnownHosts(e.to_string())),
        }
    }
}

/// Remove every known_hosts line that conflicts with `key` for this host,
/// keeping a `.old` backup (like `ssh-keygen -R`), then record `key`.
pub(crate) fn replace_known_host(path: &Path, host: &str, port: u16, key: &PublicKey) -> Result<(), SshError> {
    let io = |e: std::io::Error| SshError::KnownHosts(e.to_string());
    std::fs::copy(path, path.with_extension("old")).map_err(io)?;
    for _ in 0..MAX_STALE_LINES {
        let line = match keys::check_known_hosts_path(host, port, key, path) {
            Err(keys::Error::KeyChanged { line }) => line,
            Ok(_) => {
                return keys::known_hosts::learn_known_hosts_path(host, port, key, path)
                    .map_err(|e| SshError::KnownHosts(e.to_string()));
            }
            Err(e) => return Err(SshError::KnownHosts(e.to_string())),
        };
        let text = std::fs::read_to_string(path).map_err(io)?;
        let kept: Vec<&str> = text.split_inclusive('\n').enumerate().filter(|(i, _)| i + 1 != line).map(|(_, l)| l).collect();
        let tmp = path.with_extension("coshell-tmp");
        std::fs::write(&tmp, kept.concat()).map_err(io)?;
        std::fs::rename(&tmp, path).map_err(io)?;
    }
    Err(SshError::KnownHosts(format!("{host}:{port}: too many stale entries")))
}

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        Ok(match key {
            PublicKeyOrCertificate::PublicKey { key, .. } => self.verify(key).await,
            PublicKeyOrCertificate::Certificate(_) => {
                self.reject(SshError::HostCertificateUnsupported { host: self.host.clone() })
            }
        })
    }

    /// Server opened a channel for one of our `-R` forwards.
    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: russh::Channel<client::Msg>,
        _connected_address: &str,
        connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        let Some((host, port, stats)) = forward::serve_remote(&self.routes, connected_port) else {
            reply.reject(russh::ChannelOpenFailure::AdministrativelyProhibited).await;
            return Ok(());
        };
        match tokio::net::TcpStream::connect((host.as_str(), port)).await {
            Ok(sock) => {
                reply.accept().await;
                tokio::spawn(forward::pipe(sock, channel.into_stream(), stats));
            }
            Err(e) => {
                if let Ok(mut l) = stats.last_error.lock() {
                    *l = Some(format!("{host}:{port}: {e}"));
                }
                reply.reject(russh::ChannelOpenFailure::ConnectFailed).await;
            }
        }
        Ok(())
    }
}
