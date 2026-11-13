//! Headless connectivity probe (dev tool).
//!
//! cargo run -p coshell-ssh --example probe -- <known_hosts> <key> <user> <host:port[,host:port...]> <command>
//! Connects through the chain with key-file auth (accept-new), runs `command`
//! in a PTY shell and prints the output.

use std::io::Read;
use std::path::PathBuf;

use coshell_ssh::{Auth, ConnectOptions, HostKeyPolicy, HostSpec, connect_chain, open_shell};
use coshell_term::{GridSize, Transport};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let [kh, key, user, hops, cmd] = <[String; 5]>::try_from(a).map_err(|_| "usage: probe KNOWN_HOSTS KEY USER HOST:PORT[,..] CMD")?;
    let auth = Auth::KeyFile { path: PathBuf::from(key), passphrase: None };
    let chain: Vec<HostSpec> = hops
        .split(',')
        .map(|h| {
            let (host, port) = h.rsplit_once(':').expect("host:port");
            HostSpec { host: host.into(), port: port.parse().expect("port"), user: user.clone(), auth: auth.clone() }
        })
        .collect();
    let policy = if std::env::var_os("PROBE_STRICT").is_some() { HostKeyPolicy::Strict } else { HostKeyPolicy::AcceptNew };
    let opts = ConnectOptions { policy, ..ConnectOptions::new(kh.into()) };

    let chain = connect_chain(&chain, &opts).await?;
    println!("connected: {}", chain.labels().join(" -> "));
    let (tx, mut rd) = open_shell(&chain, GridSize::DEFAULT).await?;
    tx.write(format!("{cmd}; exit\n").as_bytes());
    let out = tokio::task::spawn_blocking(move || {
        let mut s = Vec::new();
        let _ = rd.read_to_end(&mut s);
        s
    })
    .await?;
    print!("{}", String::from_utf8_lossy(&out));
    Ok(())
}
