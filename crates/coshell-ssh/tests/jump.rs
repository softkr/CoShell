//! End-to-end tests against two in-process russh servers:
//! `client → jump (direct-tcpip) → target (pty shell)`.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use coshell_ssh::{ConnectOptions, HostKeyPolicy, SshError, connect_chain, open_shell};
use coshell_term::{GridSize, Session, Transport};
use tokio::net::TcpListener;

#[tokio::test(flavor = "multi_thread")]
async fn jump_chain_shell_roundtrip() {
    let (jp, jk) = start(Role::Jump).await;
    let (tp, tk) = start(Role::Target).await;
    let dir = tempfile::tempdir().unwrap();
    let kh = dir.path().join("known_hosts");
    trust(&kh, jp, &jk);
    trust(&kh, tp, &tk);

    let chain = connect_chain(&[hop(JUMP_USER, jp), hop(TARGET_USER, tp)], &opts(&kh, HostKeyPolicy::Strict))
        .await
        .expect("chain connects");
    assert_eq!(chain.labels().len(), 2);

    let (tx, rd) = open_shell(&chain, SIZE).await.expect("shell opens");
    let (rd, out) = read_until(rd, "welcome 한글").await;
    assert!(out.contains("pty xterm-256color 100x30"), "{out}");

    tx.write(b"ls");
    let (rd, out) = read_until(rd, "echo:ls").await;
    assert!(out.contains("echo:ls"));

    tx.resize(GridSize { cols: 132, rows: 40, ..SIZE });
    let (_, out) = read_until(rd, "resized 132x40").await;
    assert!(out.contains("resized 132x40"));
}

#[tokio::test(flavor = "multi_thread")]
async fn ssh_transport_drives_terminal_session() {
    let (tp, tk) = start(Role::Target).await;
    let dir = tempfile::tempdir().unwrap();
    let kh = dir.path().join("known_hosts");
    trust(&kh, tp, &tk);

    let chain = connect_chain(&[hop(TARGET_USER, tp)], &opts(&kh, HostKeyPolicy::Strict)).await.unwrap();
    let (tx, rd) = open_shell(&chain, SIZE).await.unwrap();
    let session = Session::new(Arc::new(tx), rd, SIZE);

    let deadline = tokio::time::Instant::now() + IO_TIMEOUT;
    loop {
        let screen = {
            use coshell_term::alacritty_terminal::grid::Dimensions;
            use coshell_term::alacritty_terminal::index::{Column, Line, Point};
            let t = session.term().lock();
            (0..t.screen_lines() as i32)
                .map(|l| (0..t.columns()).map(|c| t.grid()[Point::new(Line(l), Column(c))].c).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n")
        };
        if screen.contains("welcome") {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "screen never showed banner:\n{screen}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_host_rejected_when_strict() {
    let (tp, _) = start(Role::Target).await;
    let dir = tempfile::tempdir().unwrap();
    let kh = dir.path().join("known_hosts");

    let err = connect_chain(&[hop(TARGET_USER, tp)], &opts(&kh, HostKeyPolicy::Strict)).await.err().unwrap();
    assert!(matches!(err, SshError::UnknownHostKey { port, ref fingerprint, .. } if port == tp && fingerprint.starts_with("SHA256:")), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn every_hop_is_verified() {
    let (jp, jk) = start(Role::Jump).await;
    let (tp, _) = start(Role::Target).await;
    let dir = tempfile::tempdir().unwrap();
    let kh = dir.path().join("known_hosts");
    trust(&kh, jp, &jk); // target deliberately unknown

    let err = connect_chain(&[hop(JUMP_USER, jp), hop(TARGET_USER, tp)], &opts(&kh, HostKeyPolicy::Strict))
        .await
        .err()
        .unwrap();
    assert!(matches!(err, SshError::UnknownHostKey { port, .. } if port == tp), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn accept_new_records_key_then_strict_passes() {
    let (tp, _) = start(Role::Target).await;
    let dir = tempfile::tempdir().unwrap();
    let kh = dir.path().join("known_hosts");

    connect_chain(&[hop(TARGET_USER, tp)], &opts(&kh, HostKeyPolicy::AcceptNew)).await.expect("accept-new connects");
    assert!(std::fs::read_to_string(&kh).unwrap().contains(&format!("[127.0.0.1]:{tp}")));
    connect_chain(&[hop(TARGET_USER, tp)], &opts(&kh, HostKeyPolicy::Strict)).await.expect("now known");
}

#[tokio::test(flavor = "multi_thread")]
async fn changed_key_rejected_even_with_accept_new() {
    let (tp, _) = start(Role::Target).await;
    let dir = tempfile::tempdir().unwrap();
    let kh = dir.path().join("known_hosts");
    trust(&kh, tp, host_key().public_key()); // a different key recorded for this host

    let err = connect_chain(&[hop(TARGET_USER, tp)], &opts(&kh, HostKeyPolicy::AcceptNew)).await.err().unwrap();
    assert!(matches!(err, SshError::HostKeyChanged { port, line, .. } if port == tp && line >= 1), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_password_is_auth_failure() {
    let (tp, tk) = start(Role::Target).await;
    let dir = tempfile::tempdir().unwrap();
    let kh = dir.path().join("known_hosts");
    trust(&kh, tp, &tk);

    let err = connect_chain(&[hop(("alice", "nope"), tp)], &opts(&kh, HostKeyPolicy::Strict)).await.err().unwrap();
    assert!(matches!(err, SshError::AuthFailed(_)), "{err}");
}

/// A server that accepts TCP but never speaks SSH (e.g. sshd PerSourcePenalties,
/// a black-holed port) must end in `Timeout`, not hang the UI forever.
#[tokio::test(flavor = "multi_thread")]
async fn silent_server_times_out() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((sock, _)) = listener.accept().await {
            held.push(sock); // keep open, send nothing
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let o = ConnectOptions { connect_timeout: Duration::from_millis(300), ..opts(&dir.path().join("kh"), HostKeyPolicy::Strict) };

    let started = std::time::Instant::now();
    let err = connect_chain(&[hop(TARGET_USER, port)], &o).await.err().unwrap();
    assert!(matches!(err, SshError::Timeout(_)), "{err}");
    assert!(started.elapsed() < Duration::from_secs(2));
}
