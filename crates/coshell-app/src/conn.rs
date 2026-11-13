//! SSH connect task + the UI side of `coshell_ssh::Prompter`.
//!
//! The connection future runs on iced's (tokio) executor. Questions and
//! progress flow to the UI as `ConnEvent`s; answers go back through oneshots.

use std::io::Read;
use std::sync::{Arc, Mutex};

use coshell_ai::SecretStore;
use coshell_ssh::{
    Auth, BoxFuture, ConnectOptions, Connection, HostKeyDecision, HostKeyQuery, HostSpec, InteractiveQuery, Progress, Prompter,
    SshTransport, connect_chain_with, open_shell,
};
use coshell_term::GridSize;
use futures::channel::{mpsc, oneshot};
use zeroize::Zeroizing;

pub type Reader = Box<dyn Read + Send>;
pub type Connected = Result<(SshTransport, Reader, Connection), String>;

/// A one-shot value carried inside a `Clone` message (taken once).
pub struct Once<T>(Arc<Mutex<Option<T>>>);

impl<T> Once<T> {
    pub fn new(v: T) -> Self {
        Self(Arc::new(Mutex::new(Some(v))))
    }
    pub fn take(&self) -> Option<T> {
        self.0.lock().ok()?.take()
    }
}
impl<T> Clone for Once<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<T> std::fmt::Debug for Once<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Once(..)")
    }
}

pub type Answers = Option<Vec<Zeroizing<String>>>;

#[derive(Clone, Debug)]
pub enum ConnEvent {
    Progress(Progress),
    HostKey(HostKeyQuery, Once<oneshot::Sender<HostKeyDecision>>),
    Interactive(InteractiveQuery, Once<oneshot::Sender<Answers>>),
    Done(Once<Connected>),
}

struct UiPrompter {
    tx: mpsc::UnboundedSender<ConnEvent>,
}

impl Prompter for UiPrompter {
    fn host_key(&self, q: HostKeyQuery) -> BoxFuture<HostKeyDecision> {
        let (tx, rx) = oneshot::channel();
        let _ = self.tx.unbounded_send(ConnEvent::HostKey(q, Once::new(tx)));
        // Dropped reply (tab closed) = reject.
        Box::pin(async move { rx.await.unwrap_or(HostKeyDecision::Reject) })
    }

    fn interactive(&self, q: InteractiveQuery) -> BoxFuture<Answers> {
        let (tx, rx) = oneshot::channel();
        let _ = self.tx.unbounded_send(ConnEvent::Interactive(q, Once::new(tx)));
        Box::pin(async move { rx.await.unwrap_or(None) })
    }

    fn progress(&self, e: Progress) {
        let _ = self.tx.unbounded_send(ConnEvent::Progress(e));
    }
}

/// Keychain account for a server's saved password: `user@host:port`.
pub fn password_key(h: &HostSpec) -> String {
    format!("{}@{}:{}", h.user, h.host, h.port)
}

/// Give every `Auth::Auto` hop its saved password, if any.
pub(crate) fn fill_saved_passwords(chain: &mut [HostSpec], passwords: &dyn SecretStore) {
    for h in chain {
        let key = password_key(h);
        if let Auth::Auto { password, .. } = &mut h.auth {
            *password = passwords.get(&key);
        }
    }
}

/// Start connecting. Returns (event stream, driver future). The caller must
/// run both; the stream ends after `Done`. Saved passwords are read from
/// `passwords` inside the driver (keychain access stays off the UI thread).
pub fn connect(
    mut chain: Vec<HostSpec>,
    opts: ConnectOptions,
    passwords: Arc<dyn SecretStore>,
) -> (mpsc::UnboundedReceiver<ConnEvent>, impl Future<Output = ()> + Send + 'static) {
    let (tx, rx) = mpsc::unbounded();
    let prompter: Arc<dyn Prompter> = Arc::new(UiPrompter { tx: tx.clone() });
    let driver = async move {
        fill_saved_passwords(&mut chain, passwords.as_ref());
        let result: Connected = async {
            let chain = connect_chain_with(&chain, &opts, Some(prompter.clone())).await.map_err(|e| e.to_string())?;
            prompter.progress(Progress::OpeningShell);
            let (t, r) = open_shell(&chain, GridSize::DEFAULT).await.map_err(|e| e.to_string())?;
            Ok((t, r, chain))
        }
        .await;
        let _ = tx.unbounded_send(ConnEvent::Done(Once::new(result)));
        tx.close_channel();
    };
    (rx, driver)
}
