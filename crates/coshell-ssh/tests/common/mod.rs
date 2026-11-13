//! In-process russh servers and helpers shared by the integration tests.
#![allow(dead_code)]

use std::io::Read;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use coshell_ssh::{Auth, ConnectOptions, HostKeyPolicy, HostSpec};
use coshell_term::GridSize;
use russh::keys::ssh_key::private::Ed25519Keypair;
use russh::keys::{PrivateKey, PublicKey};
use russh::server::{self, Msg, Server as _, Session as ServerSession};
use russh::{Channel, ChannelId, ChannelOpenFailure, MethodKind, MethodSet};
use tokio::net::{TcpListener, TcpStream};
use zeroize::Zeroizing;

pub const JUMP_USER: (&str, &str) = ("jump", "jump-pw");
pub const TARGET_USER: (&str, &str) = ("alice", "alice-pw");
pub const IO_TIMEOUT: Duration = Duration::from_secs(5);
pub const SIZE: GridSize = GridSize { cols: 100, rows: 30, cell_width: 8, cell_height: 18 };

pub const OTP_USER: (&str, &str) = ("bob", "bob-pw");
pub const OTP_CODE: &str = "123456";

#[derive(Clone, Copy, PartialEq)]
pub enum Role {
    /// Relays direct-tcpip.
    Jump,
    /// PTY shell, password auth (also keyboard-interactive disabled).
    Target,
    /// Password then mandatory keyboard-interactive OTP (partial success).
    Otp,
}

#[derive(Clone)]
struct TestServer(Role);

impl server::Server for TestServer {
    type Handler = Self;
    fn new_client(&mut self, _: Option<SocketAddr>) -> Self {
        self.clone()
    }
}

impl server::Handler for TestServer {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, pw: &str) -> Result<server::Auth, Self::Error> {
        Ok(match self.0 {
            Role::Jump if (user, pw) == JUMP_USER => server::Auth::Accept,
            Role::Target if (user, pw) == TARGET_USER => server::Auth::Accept,
            Role::Otp if (user, pw) == OTP_USER => server::Auth::Reject {
                proceed_with_methods: Some(MethodSet::from(&[MethodKind::KeyboardInteractive][..])),
                partial_success: true,
            },
            // Like OpenSSH, keep offering password after a wrong one
            // (russh's default drops it after the first failure).
            Role::Target => server::Auth::Reject {
                proceed_with_methods: Some(MethodSet::from(&[MethodKind::Password, MethodKind::PublicKey][..])),
                partial_success: false,
            },
            _ => server::Auth::reject(),
        })
    }

    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        _user: &str,
        _submethods: &str,
        response: Option<server::Response<'a>>,
    ) -> Result<server::Auth, Self::Error> {
        if self.0 != Role::Otp {
            return Ok(server::Auth::reject());
        }
        Ok(match response {
            None => server::Auth::Partial {
                name: "2FA".into(),
                instructions: "Enter the code from your authenticator".into(),
                prompts: vec![("Verification code: ".into(), false)].into(),
            },
            Some(mut r) => {
                let code = r.next().map(|b| String::from_utf8_lossy(&b).to_string());
                if code.as_deref() == Some(OTP_CODE) { server::Auth::Accept } else { server::Auth::reject() }
            }
        })
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        reply: server::ChannelOpenHandle,
        _session: &mut ServerSession,
    ) -> Result<(), Self::Error> {
        if self.0 != Role::Jump {
            reply.accept().await;
        } else {
            reply.reject(ChannelOpenFailure::AdministrativelyProhibited).await;
        }
        Ok(())
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<Msg>,
        host: &str,
        port: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: server::ChannelOpenHandle,
        _session: &mut ServerSession,
    ) -> Result<(), Self::Error> {
        if self.0 != Role::Jump {
            reply.reject(ChannelOpenFailure::AdministrativelyProhibited).await;
            return Ok(());
        }
        match TcpStream::connect((host, port as u16)).await {
            Ok(mut tcp) => {
                reply.accept().await;
                tokio::spawn(async move {
                    let mut stream = channel.into_stream();
                    let _ = tokio::io::copy_bidirectional(&mut stream, &mut tcp).await;
                });
            }
            Err(_) => reply.reject(ChannelOpenFailure::ConnectFailed).await,
        }
        Ok(())
    }

    /// `-R` support for the relay role: listen locally, open forwarded-tcpip per connection.
    async fn tcpip_forward(&mut self, address: &str, port: &mut u32, session: &mut ServerSession) -> Result<bool, Self::Error> {
        if self.0 != Role::Jump {
            return Ok(false);
        }
        let Ok(listener) = TcpListener::bind(("127.0.0.1", *port as u16)).await else { return Ok(false) };
        let bound = listener.local_addr().unwrap().port();
        *port = u32::from(bound);
        let handle = session.handle();
        let address = address.to_string();
        tokio::spawn(async move {
            while let Ok((mut sock, peer)) = listener.accept().await {
                let (handle, address) = (handle.clone(), address.clone());
                tokio::spawn(async move {
                    if let Ok(ch) = handle
                        .channel_open_forwarded_tcpip(address, u32::from(bound), peer.ip().to_string(), u32::from(peer.port()))
                        .await
                    {
                        let mut stream = ch.into_stream();
                        let _ = tokio::io::copy_bidirectional(&mut stream, &mut sock).await;
                    }
                });
            }
        });
        Ok(true)
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        term: &str,
        cols: u32,
        rows: u32,
        _pw: u32,
        _ph: u32,
        _modes: &[(russh::Pty, u32)],
        session: &mut ServerSession,
    ) -> Result<(), Self::Error> {
        session.data(channel, format!("pty {term} {cols}x{rows}\r\n").into_bytes())?;
        Ok(())
    }

    async fn shell_request(&mut self, channel: ChannelId, session: &mut ServerSession) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        session.data(channel, "welcome 한글\r\n".as_bytes().to_vec())?;
        Ok(())
    }

    async fn data(&mut self, channel: ChannelId, data: &[u8], session: &mut ServerSession) -> Result<(), Self::Error> {
        // The jump host only relays; direct-tcpip bytes must pass through untouched.
        if self.0 == Role::Jump {
            return Ok(());
        }
        session.data(channel, format!("echo:{}\r\n", String::from_utf8_lossy(data)).into_bytes())?;
        Ok(())
    }

    async fn window_change_request(
        &mut self,
        channel: ChannelId,
        cols: u32,
        rows: u32,
        _pw: u32,
        _ph: u32,
        session: &mut ServerSession,
    ) -> Result<(), Self::Error> {
        session.data(channel, format!("resized {cols}x{rows}\r\n").into_bytes())?;
        Ok(())
    }
}

static SEED: AtomicU8 = AtomicU8::new(1);

pub fn host_key() -> PrivateKey {
    let seed = [SEED.fetch_add(1, Ordering::Relaxed); 32];
    PrivateKey::from(Ed25519Keypair::from_seed(&seed))
}

/// Start a server on an ephemeral port; returns (port, host public key).
pub async fn start(role: Role) -> (u16, PublicKey) {
    let key = host_key();
    let public = key.public_key().clone();
    let methods: &[MethodKind] = match role {
        Role::Target => &[MethodKind::Password, MethodKind::PublicKey],
        _ => &[MethodKind::Password, MethodKind::PublicKey, MethodKind::KeyboardInteractive],
    };
    let config = Arc::new(server::Config {
        keys: vec![key],
        methods: MethodSet::from(methods),
        auth_rejection_time: Duration::from_millis(10),
        auth_rejection_time_initial: Some(Duration::ZERO),
        ..Default::default()
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let mut srv = TestServer(role);
        let _ = srv.run_on_socket(config, &listener).await;
    });
    (port, public)
}

pub fn hop((user, pw): (&str, &str), port: u16) -> HostSpec {
    HostSpec {
        host: "127.0.0.1".into(),
        port,
        user: user.into(),
        auth: Auth::Password(Zeroizing::new(pw.into())),
    }
}

pub fn opts(kh: &Path, policy: HostKeyPolicy) -> ConnectOptions {
    ConnectOptions { policy, ..ConnectOptions::new(kh.to_path_buf()) }
}

pub fn trust(kh: &Path, port: u16, key: &PublicKey) {
    russh::keys::known_hosts::learn_known_hosts_path("127.0.0.1", port, key, kh).unwrap();
}

pub type Reader = Box<dyn Read + Send>;

/// Read (blocking, off the runtime) until `needle` appears.
pub async fn read_until(rd: Reader, needle: &str) -> (Reader, String) {
    let needle = needle.to_string();
    let job = tokio::task::spawn_blocking(move || {
        let mut rd = rd;
        let mut acc = String::new();
        let mut buf = [0u8; 4096];
        while !acc.contains(&needle) {
            let n = rd.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            acc.push_str(&String::from_utf8_lossy(&buf[..n]));
        }
        (rd, acc)
    });
    tokio::time::timeout(IO_TIMEOUT, job).await.expect("read timed out").unwrap()
}

/// TCP echo server on an ephemeral port.
pub async fn echo_server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (mut r, mut w) = sock.split();
                let _ = tokio::io::copy(&mut r, &mut w).await;
            });
        }
    });
    port
}
