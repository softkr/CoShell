//! Command-line launch spec (spike: until the host manager UI exists).
//!
//! ```text
//! coshell                                   # local shell
//! coshell ssh [-i KEY] [--accept-new|--strict] [--known-hosts PATH] [-J [user@]jump[:port][,...]] [user@]host[:port]
//! ```
//! Auth is ssh-agent unless `-i` is given (applied to every hop).
//! Passwords are never taken from the command line.

use crate::i18n::{tf, tr};
use std::path::PathBuf;

use coshell_ssh::{Auth, ConnectOptions, DEFAULT_PORT, HostKeyPolicy, HostSpec};

#[derive(Debug)]
pub enum Launch {
    Local,
    /// `coshell ssh <name>` without options: resolved through ~/.ssh/config.
    Alias(String),
    Ssh { chain: Vec<HostSpec>, opts: ConnectOptions },
}

pub const USAGE: &str =
    "usage: coshell [ssh [-i KEY] [--accept-new|--strict] [--known-hosts PATH] [-J [user@]jump[:port][,...]] [user@]host[:port]]";

fn parse_host(s: &str, default_user: &str, auth: &Auth) -> Result<HostSpec, String> {
    let (user, rest) = match s.split_once('@') {
        Some((u, r)) if !u.is_empty() => (u.to_string(), r),
        Some(_) => return Err(tf("잘못된 사용자: {0}", &[&s])),
        None => (default_user.to_string(), s),
    };
    let (host, port) = match rest.rsplit_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().map_err(|_| tf("잘못된 포트: {0}", &[&s]))?),
        None => (rest, DEFAULT_PORT),
    };
    if host.is_empty() {
        return Err(tf("호스트가 비어 있습니다: {0}", &[&s]));
    }
    Ok(HostSpec { host: host.to_string(), port, user, auth: auth.clone() })
}

/// Parse `args` (without argv[0]).
pub fn parse(args: &[String], default_user: &str, known_hosts: Option<PathBuf>) -> Result<Launch, String> {
    let Some((cmd, rest)) = args.split_first() else { return Ok(Launch::Local) };
    if cmd != "ssh" {
        return Err(USAGE.into());
    }
    let mut key: Option<PathBuf> = None;
    let mut jumps: Option<String> = None;
    let mut policy = HostKeyPolicy::Ask;
    let mut dest: Option<String> = None;
    let mut known_hosts = known_hosts;
    let mut known_hosts_set = None;
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-i" => key = Some(it.next().ok_or(USAGE)?.into()),
            "-J" => jumps = Some(it.next().ok_or(USAGE)?.clone()),
            "--accept-new" => policy = HostKeyPolicy::AcceptNew,
            "--strict" => policy = HostKeyPolicy::Strict,
            "--known-hosts" => {
                known_hosts = Some(it.next().ok_or(USAGE)?.into());
                known_hosts_set = Some(());
            }
            s if s.starts_with('-') => return Err(tf("알 수 없는 옵션 {0}\n{1}", &[&s, &USAGE])),
            s if dest.is_none() => dest = Some(s.to_string()),
            _ => return Err(USAGE.into()),
        }
    }
    let dest = dest.ok_or(USAGE)?;
    if key.is_none() && jumps.is_none() && policy == HostKeyPolicy::Ask && known_hosts_set.is_none() {
        parse_host(&dest, default_user, &Auth::Agent)?; // validate user@host:port syntax early
        return Ok(Launch::Alias(dest));
    }
    let auth = match key {
        Some(path) => Auth::KeyFile { path, passphrase: None },
        None => Auth::Agent,
    };
    let mut chain = Vec::new();
    for j in jumps.iter().flat_map(|j| j.split(',')).filter(|j| !j.is_empty()) {
        chain.push(parse_host(j, default_user, &auth)?);
    }
    chain.push(parse_host(&dest, default_user, &auth)?);

    let known_hosts = known_hosts.ok_or(tr("HOME을 알 수 없어 known_hosts 경로를 정할 수 없습니다"))?;
    Ok(Launch::Ssh { chain, opts: ConnectOptions { policy, ..ConnectOptions::new(known_hosts) } })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }
    fn kh() -> Option<PathBuf> {
        Some("/tmp/kh".into())
    }

    #[test]
    fn no_args_is_local() {
        assert!(matches!(parse(&[], "me", kh()), Ok(Launch::Local)));
    }

    #[test]
    fn jump_chain_with_users_and_ports() {
        let Ok(Launch::Ssh { chain, opts }) =
            parse(&args("ssh -J ec2-user@13.124.88.12,ops@10.20.0.5:2222 ubuntu@10.0.1.42"), "me", kh())
        else {
            panic!()
        };
        let got: Vec<_> = chain.iter().map(|h| (h.user.as_str(), h.host.as_str(), h.port)).collect();
        assert_eq!(got, [("ec2-user", "13.124.88.12", 22), ("ops", "10.20.0.5", 2222), ("ubuntu", "10.0.1.42", 22)]);
        assert_eq!(opts.policy, HostKeyPolicy::Ask);
        assert!(matches!(chain[0].auth, Auth::Agent));
    }

    #[test]
    fn defaults_user_key_and_accept_new() {
        let Ok(Launch::Ssh { chain, opts }) = parse(&args("ssh --accept-new -i /k/id host:2201"), "me", kh()) else {
            panic!()
        };
        assert_eq!((chain[0].user.as_str(), chain[0].port), ("me", 2201));
        assert!(matches!(&chain[0].auth, Auth::KeyFile { path, .. } if path == &PathBuf::from("/k/id")));
        assert_eq!(opts.policy, HostKeyPolicy::AcceptNew);
    }

    #[test]
    fn known_hosts_override() {
        let Ok(Launch::Ssh { opts, .. }) = parse(&args("ssh --strict --known-hosts /x/kh host"), "me", None) else { panic!() };
        assert_eq!(opts.known_hosts, PathBuf::from("/x/kh"));
        assert_eq!(opts.policy, HostKeyPolicy::Strict);
    }

    #[test]
    fn bare_name_goes_through_ssh_config() {
        assert!(matches!(parse(&args("ssh prod-web"), "me", kh()), Ok(Launch::Alias(a)) if a == "prod-web"));
        assert!(matches!(parse(&args("ssh -J b prod-web"), "me", kh()), Ok(Launch::Ssh { .. })));
    }

    #[test]
    fn rejects_bad_input() {
        assert!(parse(&args("ssh"), "me", kh()).is_err());
        assert!(parse(&args("ssh host:notaport"), "me", kh()).is_err());
        assert!(parse(&args("ssh -x host"), "me", kh()).is_err());
        assert!(parse(&args("telnet host"), "me", kh()).is_err());
    }
}
