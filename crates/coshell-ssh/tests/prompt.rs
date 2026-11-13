//! Interactive flows: host-key questions, 2FA / password prompts, progress.

mod common;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use common::*;
use coshell_ssh::{
    Auth, BoxFuture, HostKeyDecision, HostKeyKind, HostKeyPolicy, HostKeyQuery, HostSpec, InteractiveQuery, Progress,
    Prompter, SshError, connect_chain, connect_chain_with,
};
use zeroize::Zeroizing;

#[derive(Default)]
struct Mock {
    decision: Option<HostKeyDecision>,
    answers: Mutex<VecDeque<Option<Vec<&'static str>>>>,
    host_queries: Mutex<Vec<HostKeyQuery>>,
    ki_queries: Mutex<Vec<InteractiveQuery>>,
    progress: Mutex<Vec<Progress>>,
}

impl Mock {
    fn host(d: HostKeyDecision) -> Arc<Self> {
        Arc::new(Self { decision: Some(d), ..Default::default() })
    }
    fn answering(answers: &[Option<Vec<&'static str>>]) -> Arc<Self> {
        Arc::new(Self { answers: Mutex::new(answers.iter().cloned().collect()), ..Default::default() })
    }
}

impl Prompter for Mock {
    fn host_key(&self, q: HostKeyQuery) -> BoxFuture<HostKeyDecision> {
        self.host_queries.lock().unwrap().push(q);
        let d = self.decision.unwrap_or(HostKeyDecision::Reject);
        Box::pin(async move { d })
    }
    fn interactive(&self, q: InteractiveQuery) -> BoxFuture<Option<Vec<Zeroizing<String>>>> {
        self.ki_queries.lock().unwrap().push(q);
        let a = self.answers.lock().unwrap().pop_front().flatten();
        Box::pin(async move { a.map(|v| v.into_iter().map(|s| Zeroizing::new(s.to_string())).collect()) })
    }
    fn progress(&self, e: Progress) {
        self.progress.lock().unwrap().push(e);
    }
}

fn dyn_p(m: &Arc<Mock>) -> Option<Arc<dyn Prompter>> {
    Some(m.clone() as Arc<dyn Prompter>)
}

#[tokio::test(flavor = "multi_thread")]
async fn ask_unknown_accept_and_save() {
    let (tp, _) = start(Role::Target).await;
    let dir = tempfile::tempdir().unwrap();
    let kh = dir.path().join("known_hosts");
    let m = Mock::host(HostKeyDecision::AcceptAndSave);

    connect_chain_with(&[hop(TARGET_USER, tp)], &opts(&kh, HostKeyPolicy::Ask), dyn_p(&m)).await.unwrap();
    let q = m.host_queries.lock().unwrap()[0].clone();
    assert_eq!((q.hop, q.port, &q.kind), (0, tp, &HostKeyKind::Unknown));
    assert!(q.fingerprint.starts_with("SHA256:"));
    connect_chain(&[hop(TARGET_USER, tp)], &opts(&kh, HostKeyPolicy::Strict)).await.expect("saved");
}

#[tokio::test(flavor = "multi_thread")]
async fn ask_unknown_accept_once_does_not_save() {
    let (tp, _) = start(Role::Target).await;
    let dir = tempfile::tempdir().unwrap();
    let kh = dir.path().join("known_hosts");
    let m = Mock::host(HostKeyDecision::AcceptOnce);

    connect_chain_with(&[hop(TARGET_USER, tp)], &opts(&kh, HostKeyPolicy::Ask), dyn_p(&m)).await.unwrap();
    let err = connect_chain(&[hop(TARGET_USER, tp)], &opts(&kh, HostKeyPolicy::Strict)).await.err().unwrap();
    assert!(matches!(err, SshError::UnknownHostKey { .. }), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn ask_unknown_reject_is_cancelled() {
    let (tp, _) = start(Role::Target).await;
    let dir = tempfile::tempdir().unwrap();
    let m = Mock::host(HostKeyDecision::Reject);
    let err = connect_chain_with(&[hop(TARGET_USER, tp)], &opts(&dir.path().join("kh"), HostKeyPolicy::Ask), dyn_p(&m))
        .await
        .err()
        .unwrap();
    assert!(matches!(err, SshError::Cancelled(_)), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn ask_changed_replace_rewrites_known_hosts() {
    let (tp, real) = start(Role::Target).await;
    let dir = tempfile::tempdir().unwrap();
    let kh = dir.path().join("known_hosts");
    trust(&kh, 1, &host_key().public_key().clone()); // unrelated entry must survive
    let stale = host_key().public_key().clone();
    trust(&kh, tp, &stale);
    let m = Mock::host(HostKeyDecision::ReplaceAndSave);

    connect_chain_with(&[hop(TARGET_USER, tp)], &opts(&kh, HostKeyPolicy::Ask), dyn_p(&m)).await.unwrap();
    assert!(matches!(m.host_queries.lock().unwrap()[0].kind, HostKeyKind::Changed { .. }));

    let text = std::fs::read_to_string(&kh).unwrap();
    assert!(text.contains("[127.0.0.1]:1 "), "unrelated entry kept");
    assert!(!text.contains(&stale.to_openssh().unwrap()), "stale key removed");
    assert!(text.contains(&real.to_openssh().unwrap().split(' ').nth(1).unwrap().to_string()));
    assert!(kh.with_extension("old").exists(), "backup written");
    connect_chain(&[hop(TARGET_USER, tp)], &opts(&kh, HostKeyPolicy::Strict)).await.expect("now trusted");
}

#[tokio::test(flavor = "multi_thread")]
async fn changed_key_never_accepted_once_or_saved() {
    for d in [HostKeyDecision::AcceptOnce, HostKeyDecision::AcceptAndSave] {
        let (tp, _) = start(Role::Target).await;
        let dir = tempfile::tempdir().unwrap();
        let kh = dir.path().join("known_hosts");
        trust(&kh, tp, &host_key().public_key().clone());
        let err = connect_chain_with(&[hop(TARGET_USER, tp)], &opts(&kh, HostKeyPolicy::Ask), dyn_p(&Mock::host(d)))
            .await
            .err()
            .unwrap();
        assert!(matches!(err, SshError::HostKeyChanged { .. }), "{d:?}: {err}");
    }
}

async fn otp_target() -> (u16, tempfile::TempDir, std::path::PathBuf) {
    let (port, key) = start(Role::Otp).await;
    let dir = tempfile::tempdir().unwrap();
    let kh = dir.path().join("known_hosts");
    trust(&kh, port, &key);
    (port, dir, kh)
}

#[tokio::test(flavor = "multi_thread")]
async fn password_then_otp_partial_success() {
    let (port, _d, kh) = otp_target().await;
    let m = Mock::answering(&[Some(vec![OTP_CODE])]);
    connect_chain_with(&[hop(OTP_USER, port)], &opts(&kh, HostKeyPolicy::Strict), dyn_p(&m)).await.expect("2FA ok");
    let q = &m.ki_queries.lock().unwrap()[0];
    assert_eq!(q.name, "2FA");
    assert_eq!(q.prompts.len(), 1);
    assert_eq!((q.prompts[0].text.as_str(), q.prompts[0].echo), ("Verification code: ", false));
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_otp_fails_after_bounded_rounds() {
    let (port, _d, kh) = otp_target().await;
    let m = Mock::answering(&[Some(vec!["000000"]), Some(vec!["111111"]), Some(vec!["222222"]), Some(vec![OTP_CODE])]);
    let err = connect_chain_with(&[hop(OTP_USER, port)], &opts(&kh, HostKeyPolicy::Strict), dyn_p(&m)).await.err().unwrap();
    assert!(matches!(err, SshError::AuthFailed(_)), "{err}");
    assert_eq!(m.ki_queries.lock().unwrap().len(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn otp_cancel_is_cancelled() {
    let (port, _d, kh) = otp_target().await;
    let m = Mock::answering(&[None]);
    let err = connect_chain_with(&[hop(OTP_USER, port)], &opts(&kh, HostKeyPolicy::Strict), dyn_p(&m)).await.err().unwrap();
    assert!(matches!(err, SshError::Cancelled(_)), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn interactive_auth_prompts_for_password() {
    let (tp, tk) = start(Role::Target).await; // password + publickey only
    let dir = tempfile::tempdir().unwrap();
    let kh = dir.path().join("known_hosts");
    trust(&kh, tp, &tk);
    let spec = HostSpec { auth: Auth::Interactive, ..hop(TARGET_USER, tp) };
    let m = Mock::answering(&[Some(vec![TARGET_USER.1])]);
    connect_chain_with(&[spec], &opts(&kh, HostKeyPolicy::Strict), dyn_p(&m)).await.expect("password via prompt");
    let q = &m.ki_queries.lock().unwrap()[0];
    assert!(!q.prompts[0].echo);
    assert!(q.password, "login password prompt must be flagged so the UI can offer to save it");
}

/// Auto auth with a saved password; no agent/key can match (bogus identity).
fn auto_spec(user: (&str, &str), port: u16, saved: &str) -> HostSpec {
    let auth = Auth::Auto { identity: Some("/nonexistent/key".into()), password: Some(Zeroizing::new(saved.into())) };
    HostSpec { auth, ..hop(user, port) }
}

async fn trusted(role: Role) -> (u16, tempfile::TempDir, std::path::PathBuf) {
    let (port, key) = start(role).await;
    let dir = tempfile::tempdir().unwrap();
    let kh = dir.path().join("known_hosts");
    trust(&kh, port, &key);
    (port, dir, kh)
}

#[tokio::test(flavor = "multi_thread")]
async fn saved_password_connects_without_prompt() {
    let (port, _d, kh) = trusted(Role::Target).await;
    let m = Arc::new(Mock::default());
    let spec = auto_spec(TARGET_USER, port, TARGET_USER.1);
    connect_chain_with(&[spec], &opts(&kh, HostKeyPolicy::Strict), dyn_p(&m)).await.expect("saved password");
    assert!(m.ki_queries.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_saved_password_falls_back_to_prompt() {
    let (port, _d, kh) = trusted(Role::Target).await;
    let m = Mock::answering(&[Some(vec![TARGET_USER.1])]);
    let spec = auto_spec(TARGET_USER, port, "stale");
    connect_chain_with(&[spec], &opts(&kh, HostKeyPolicy::Strict), dyn_p(&m)).await.expect("typed password");
    let qs = m.ki_queries.lock().unwrap();
    assert_eq!(qs.len(), 1);
    assert!(qs[0].password);
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_saved_password_keeps_all_prompt_rounds() {
    let (port, _d, kh) = trusted(Role::Target).await;
    let m = Mock::answering(&[Some(vec!["x"]), Some(vec!["y"]), Some(vec!["z"])]);
    let spec = auto_spec(TARGET_USER, port, "stale");
    let err = connect_chain_with(&[spec], &opts(&kh, HostKeyPolicy::Strict), dyn_p(&m)).await.err().unwrap();
    assert!(matches!(err, SshError::AuthFailed(_)), "{err}");
    assert_eq!(m.ki_queries.lock().unwrap().len(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn saved_password_never_answers_otp() {
    let (port, _d, kh) = trusted(Role::Otp).await;
    let m = Mock::answering(&[Some(vec![OTP_CODE])]);
    let spec = auto_spec(OTP_USER, port, OTP_USER.1);
    connect_chain_with(&[spec], &opts(&kh, HostKeyPolicy::Strict), dyn_p(&m)).await.expect("password + OTP");
    let qs = m.ki_queries.lock().unwrap();
    assert_eq!(qs.len(), 1, "only the OTP is asked");
    assert!(!qs[0].password);
    assert_eq!(qs[0].prompts[0].text, "Verification code: ");
}

#[tokio::test(flavor = "multi_thread")]
async fn saved_password_works_without_prompter() {
    let (port, _d, kh) = trusted(Role::Target).await;
    let spec = auto_spec(TARGET_USER, port, TARGET_USER.1);
    connect_chain_with(&[spec], &opts(&kh, HostKeyPolicy::Strict), None).await.expect("no UI needed");
}

#[tokio::test(flavor = "multi_thread")]
async fn progress_reports_each_hop_in_order() {
    let (jp, jk) = start(Role::Jump).await;
    let (tp, tk) = start(Role::Target).await;
    let dir = tempfile::tempdir().unwrap();
    let kh = dir.path().join("known_hosts");
    trust(&kh, jp, &jk);
    trust(&kh, tp, &tk);
    let m = Arc::new(Mock::default());
    connect_chain_with(&[hop(JUMP_USER, jp), hop(TARGET_USER, tp)], &opts(&kh, HostKeyPolicy::Strict), dyn_p(&m))
        .await
        .unwrap();
    let kinds: Vec<String> = m
        .progress
        .lock()
        .unwrap()
        .iter()
        .map(|p| match p {
            Progress::Connecting { hop } => format!("C{hop}"),
            Progress::Authenticating { hop } => format!("A{hop}"),
            Progress::Authenticated { hop, .. } => format!("OK{hop}"),
            Progress::OpeningShell => "S".into(),
        })
        .collect();
    assert_eq!(kinds, ["C0", "A0", "OK0", "C1", "A1", "OK1"]);
}
