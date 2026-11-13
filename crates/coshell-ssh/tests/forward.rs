//! Port forwarding through an in-process relay server (-L / -D / -R).

mod common;

use std::sync::atomic::Ordering;
use std::time::Duration;

use common::*;
use coshell_ssh::{Connection, ForwardSpec, HostKeyPolicy, connect_chain};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

async fn relay() -> (Connection, tempfile::TempDir) {
    let (jp, jk) = start(Role::Jump).await;
    let dir = tempfile::tempdir().unwrap();
    let kh = dir.path().join("known_hosts");
    trust(&kh, jp, &jk);
    (connect_chain(&[hop(JUMP_USER, jp)], &opts(&kh, HostKeyPolicy::Strict)).await.unwrap(), dir)
}

async fn roundtrip(sock: &mut TcpStream, msg: &[u8]) -> Vec<u8> {
    sock.write_all(msg).await.unwrap();
    let mut buf = vec![0u8; msg.len()];
    tokio::time::timeout(IO_TIMEOUT, sock.read_exact(&mut buf)).await.expect("echo timed out").unwrap();
    buf
}

fn local(port: u16, host_port: u16) -> ForwardSpec {
    ForwardSpec::Local { bind: "127.0.0.1".into(), port, host: "127.0.0.1".into(), host_port }
}

#[tokio::test(flavor = "multi_thread")]
async fn local_forward_roundtrip_and_stats() {
    let echo = echo_server().await;
    let (conn, _d) = relay().await;
    let t = conn.start_forward(local(0, echo)).await.unwrap();
    assert_ne!(t.bound_port, 0);

    let mut s = TcpStream::connect(("127.0.0.1", t.bound_port)).await.unwrap();
    assert_eq!(roundtrip(&mut s, "ping 한글".as_bytes()).await, "ping 한글".as_bytes());
    assert_eq!(t.stats.up.load(Ordering::Relaxed), 11);
    assert_eq!(t.stats.down.load(Ordering::Relaxed), 11);
    assert_eq!(t.stats.open.load(Ordering::Relaxed), 1);
    drop(s);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(t.stats.open.load(Ordering::Relaxed), 0);
    assert_eq!(t.stats.total.load(Ordering::Relaxed), 1);
}

async fn socks_connect(proxy: u16, target: &[u8], port: u16) -> (TcpStream, u8) {
    let mut s = TcpStream::connect(("127.0.0.1", proxy)).await.unwrap();
    s.write_all(&[5, 1, 0]).await.unwrap();
    let mut m = [0u8; 2];
    s.read_exact(&mut m).await.unwrap();
    assert_eq!(m, [5, 0]);
    let mut req = vec![5, 1, 0];
    req.extend_from_slice(target);
    req.extend_from_slice(&port.to_be_bytes());
    s.write_all(&req).await.unwrap();
    let mut rep = [0u8; 10];
    s.read_exact(&mut rep).await.unwrap();
    (s, rep[1])
}

#[tokio::test(flavor = "multi_thread")]
async fn dynamic_socks5_ipv4_and_domain() {
    let echo = echo_server().await;
    let (conn, _d) = relay().await;
    let t = conn.start_forward(ForwardSpec::Dynamic { bind: "127.0.0.1".into(), port: 0 }).await.unwrap();

    let (mut s, code) = socks_connect(t.bound_port, &[1, 127, 0, 0, 1], echo).await;
    assert_eq!(code, 0);
    assert_eq!(roundtrip(&mut s, b"via-ipv4").await, b"via-ipv4");

    let mut domain = vec![3, 9];
    domain.extend_from_slice(b"localhost");
    let (mut s, code) = socks_connect(t.bound_port, &domain, echo).await;
    assert_eq!(code, 0);
    assert_eq!(roundtrip(&mut s, b"via-domain").await, b"via-domain");
}

#[tokio::test(flavor = "multi_thread")]
async fn socks5_refuses_clients_without_no_auth() {
    let (conn, _d) = relay().await;
    let t = conn.start_forward(ForwardSpec::Dynamic { bind: "127.0.0.1".into(), port: 0 }).await.unwrap();
    let mut s = TcpStream::connect(("127.0.0.1", t.bound_port)).await.unwrap();
    s.write_all(&[5, 1, 2]).await.unwrap(); // username/password only
    let mut m = [0u8; 2];
    s.read_exact(&mut m).await.unwrap();
    assert_eq!(m, [5, 0xFF]);
}

#[tokio::test(flavor = "multi_thread")]
async fn remote_forward_roundtrip() {
    let echo = echo_server().await;
    let (conn, _d) = relay().await;
    let t = conn
        .start_forward(ForwardSpec::Remote { bind: "127.0.0.1".into(), port: 0, host: "127.0.0.1".into(), host_port: echo })
        .await
        .unwrap();
    assert_ne!(t.bound_port, 0, "server allocated a port");
    // Connect to the *server-side* listener; traffic comes back through us to the echo server.
    let mut s = TcpStream::connect(("127.0.0.1", t.bound_port)).await.unwrap();
    assert_eq!(roundtrip(&mut s, b"reverse").await, b"reverse");
    assert_eq!(t.stats.total.load(Ordering::Relaxed), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn dropping_tunnel_stops_listener() {
    let echo = echo_server().await;
    let (conn, _d) = relay().await;
    let t = conn.start_forward(local(0, echo)).await.unwrap();
    let port = t.bound_port;
    drop(t);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn unreachable_target_records_error() {
    let (conn, _d) = relay().await;
    // Port 1 on loopback: nothing listens there.
    let t = conn.start_forward(local(0, 1)).await.unwrap();
    let mut s = TcpStream::connect(("127.0.0.1", t.bound_port)).await.unwrap();
    let mut buf = [0u8; 1];
    let n = tokio::time::timeout(IO_TIMEOUT, s.read(&mut buf)).await.unwrap().unwrap_or(0);
    assert_eq!(n, 0, "client connection closed");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(t.stats.last_error.lock().unwrap().is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn busy_local_port_is_reported_as_port_in_use() {
    let echo = echo_server().await;
    let (conn, _d) = relay().await;
    let first = conn.start_forward(local(0, echo)).await.unwrap();
    let err = conn.start_forward(local(first.bound_port, echo)).await.err().unwrap();
    assert!(matches!(err, coshell_ssh::SshError::PortInUse(p) if p == first.bound_port), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn auto_forward_falls_back_to_a_free_port() {
    let echo = echo_server().await;
    let (conn, _d) = relay().await;
    let first = conn.start_forward(local(0, echo)).await.unwrap();
    let second = conn.start_forward_auto(local(first.bound_port, echo)).await.expect("falls back");
    assert_ne!(second.bound_port, first.bound_port);
    let mut s = TcpStream::connect(("127.0.0.1", second.bound_port)).await.unwrap();
    assert_eq!(roundtrip(&mut s, b"moved").await, b"moved");
}
