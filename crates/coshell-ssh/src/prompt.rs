//! Questions the connection asks the user (host keys, 2FA, passwords),
//! and progress notifications for the connect overlay.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use zeroize::Zeroizing;

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostKeyKind {
    /// Host not in known_hosts.
    Unknown,
    /// known_hosts has a different key for this host (possible MITM).
    Changed { line: usize },
}

#[derive(Clone, Debug)]
pub struct HostKeyQuery {
    /// Index of the hop in the chain (0 = first jump).
    pub hop: usize,
    pub host: String,
    pub port: u16,
    pub algorithm: String,
    /// `SHA256:…`
    pub fingerprint: String,
    pub kind: HostKeyKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostKeyDecision {
    Reject,
    /// Connect this time only (unknown keys only).
    AcceptOnce,
    /// Record in known_hosts, then connect (unknown keys only).
    AcceptAndSave,
    /// Remove the stale known_hosts entry, record the new key, connect
    /// (changed keys only; the UI must make the user confirm explicitly).
    ReplaceAndSave,
}

#[derive(Clone, Debug)]
pub struct InteractivePrompt {
    pub text: String,
    /// `false` → secret input (OTP, password).
    pub echo: bool,
}

#[derive(Clone, Debug)]
pub struct InteractiveQuery {
    pub hop: usize,
    /// `user@host[:port]`
    pub label: String,
    pub name: String,
    pub instructions: String,
    pub prompts: Vec<InteractivePrompt>,
    /// The single prompt asks for this hop's login password (the UI may
    /// offer to save it). `false` for OTPs and key passphrases.
    pub password: bool,
}

/// Connection progress, per hop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Progress {
    Connecting { hop: usize },
    Authenticating { hop: usize },
    Authenticated { hop: usize, latency: Duration },
    OpeningShell,
}

/// Implemented by the UI. Answers are awaited by the connection task.
pub trait Prompter: Send + Sync + 'static {
    fn host_key(&self, query: HostKeyQuery) -> BoxFuture<HostKeyDecision>;
    /// `None` = user cancelled. Must return one answer per prompt.
    fn interactive(&self, query: InteractiveQuery) -> BoxFuture<Option<Vec<Zeroizing<String>>>>;
    fn progress(&self, _event: Progress) {}
}
