//! Read-only `~/.ssh/config` support: `Host`, `HostName`, `User`, `Port`,
//! `ProxyJump`, `IdentityFile`. OpenSSH first-match-wins semantics; `Match`
//! and `Include` blocks are skipped.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::{Auth, DEFAULT_PORT, ForwardSpec, HostSpec};

/// Maximum ProxyJump nesting (aliases referring to aliases).
const MAX_JUMP_DEPTH: usize = 8;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostEntry {
    pub alias: String,
    pub hostname: Option<String>,
    pub user: Option<String>,
    pub port: Option<u16>,
    /// Raw `ProxyJump` value (comma separated), `None` for absent / `none`.
    pub proxy_jump: Option<String>,
    pub identity_file: Option<PathBuf>,
    /// `LocalForward` / `RemoteForward` / `DynamicForward` (cumulative).
    pub forwards: Vec<ForwardSpec>,
}

impl HostEntry {
    pub fn host(&self) -> &str {
        self.hostname.as_deref().unwrap_or(&self.alias)
    }
}

struct Block {
    patterns: Vec<String>,
    opts: Vec<(String, String)>,
}

/// `*` / `?` glob, case-insensitive like OpenSSH host matching.
fn glob(pat: &str, s: &str) -> bool {
    fn go(p: &[u8], s: &[u8]) -> bool {
        match (p.first(), s.first()) {
            (None, None) => true,
            (Some(b'*'), _) => go(&p[1..], s) || (!s.is_empty() && go(p, &s[1..])),
            (Some(b'?'), Some(_)) => go(&p[1..], &s[1..]),
            (Some(a), Some(b)) if a.eq_ignore_ascii_case(b) => go(&p[1..], &s[1..]),
            _ => false,
        }
    }
    go(pat.as_bytes(), s.as_bytes())
}

fn block_matches(patterns: &[String], name: &str) -> bool {
    let mut hit = false;
    for p in patterns {
        if let Some(neg) = p.strip_prefix('!') {
            if glob(neg, name) {
                return false;
            }
        } else if glob(p, name) {
            hit = true;
        }
    }
    hit
}

fn split_kv(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let idx = line.find(|c: char| c.is_whitespace() || c == '=')?;
    let key = line[..idx].to_ascii_lowercase();
    let val = line[idx..].trim_start_matches(|c: char| c.is_whitespace() || c == '=').trim();
    let val = val.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(val);
    Some((key, val.to_string()))
}

fn expand_tilde(p: &str, home: &Path) -> PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => PathBuf::from(p),
    }
}

pub struct SshConfig {
    blocks: Vec<Block>,
    home: PathBuf,
}

impl SshConfig {
    pub fn parse(text: &str, home: &Path) -> Self {
        // Options before the first Host apply to every host.
        let mut blocks = vec![Block { patterns: vec!["*".into()], opts: vec![] }];
        let mut skipping = false;
        for (key, val) in text.lines().filter_map(split_kv) {
            match key.as_str() {
                "host" => {
                    skipping = false;
                    blocks.push(Block { patterns: val.split_whitespace().map(String::from).collect(), opts: vec![] });
                }
                "match" => skipping = true,
                _ if skipping => {}
                _ => blocks.last_mut().expect("root block").opts.push((key, val)),
            }
        }
        Self { blocks, home: home.to_path_buf() }
    }

    pub fn load(path: &Path, home: &Path) -> std::io::Result<Self> {
        Ok(Self::parse(&std::fs::read_to_string(path)?, home))
    }

    /// Concrete (non-wildcard) aliases, in file order.
    pub fn aliases(&self) -> Vec<String> {
        let mut seen = HashSet::new();
        self.blocks
            .iter()
            .flat_map(|b| b.patterns.iter())
            .filter(|p| !p.contains(['*', '?', '!']))
            .filter(|p| seen.insert(p.to_string()))
            .cloned()
            .collect()
    }

    /// Effective settings for `name` (first value wins).
    pub fn entry(&self, name: &str) -> HostEntry {
        let mut e = HostEntry { alias: name.to_string(), ..Default::default() };
        let mut proxy_set = false;
        for b in self.blocks.iter().filter(|b| block_matches(&b.patterns, name)) {
            for (k, v) in &b.opts {
                match k.as_str() {
                    "hostname" if e.hostname.is_none() => e.hostname = Some(v.clone()),
                    "user" if e.user.is_none() => e.user = Some(v.clone()),
                    "port" if e.port.is_none() => e.port = v.parse().ok(),
                    "proxyjump" if !proxy_set => {
                        proxy_set = true;
                        e.proxy_jump = (!v.eq_ignore_ascii_case("none")).then(|| v.clone());
                    }
                    "identityfile" if e.identity_file.is_none() => e.identity_file = Some(expand_tilde(v, &self.home)),
                    "localforward" | "remoteforward" | "dynamicforward" => {
                        let kind = k.chars().next().unwrap_or('l');
                        // "[bind:]port host:hostport" → "[bind:]port:host:hostport"
                        let joined = v.split_whitespace().collect::<Vec<_>>().join(":");
                        if let Ok(f) = ForwardSpec::parse(kind, &joined)
                            && !e.forwards.contains(&f)
                        {
                            e.forwards.push(f);
                        }
                    }
                    _ => {}
                }
            }
        }
        e
    }

    pub fn entries(&self) -> Vec<HostEntry> {
        self.aliases().iter().map(|a| self.entry(a)).collect()
    }

    /// Full chain (jumps…, target) for `name`, resolving ProxyJump recursively.
    /// `name` may be an alias or `[user@]host[:port]`.
    pub fn resolve(&self, name: &str, default_user: &str) -> Result<Vec<HostSpec>, String> {
        self.resolve_depth(name, default_user, 0)
    }

    fn resolve_depth(&self, name: &str, default_user: &str, depth: usize) -> Result<Vec<HostSpec>, String> {
        if depth > MAX_JUMP_DEPTH {
            return Err(if crate::english() {
                format!("ProxyJump is too deep or loops: {name}")
            } else {
                format!("ProxyJump이 너무 깊거나 순환합니다: {name}")
            });
        }
        let (user, rest) = match name.split_once('@') {
            Some((u, r)) => (Some(u.to_string()), r),
            None => (None, name),
        };
        let (alias, port) = match rest.rsplit_once(':') {
            Some((h, p)) => (h, Some(p.parse::<u16>().map_err(|_| crate::forward::bad_port(name))?)),
            None => (rest, None),
        };
        let e = self.entry(alias);
        let mut chain = Vec::new();
        if let Some(jumps) = &e.proxy_jump {
            for j in jumps.split(',').map(str::trim).filter(|j| !j.is_empty()) {
                chain.extend(self.resolve_depth(j, default_user, depth + 1)?);
            }
        }
        chain.push(HostSpec {
            host: e.host().to_string(),
            port: port.or(e.port).unwrap_or(DEFAULT_PORT),
            user: user.or(e.user.clone()).unwrap_or_else(|| default_user.to_string()),
            auth: Auth::Auto { identity: e.identity_file.clone(), password: None },
        });
        Ok(chain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CFG: &str = r#"
# global
ServerAliveInterval 30

Host bastion
    HostName 13.124.88.12
    User ec2-user
    IdentityFile ~/.ssh/bastion.pem

Host prod-web prod-web-01
    HostName 10.0.1.42
    User ubuntu
    ProxyJump bastion

Host stage-db
    HostName=10.20.2.15
    LocalForward 15432 127.0.0.1:5432
    DynamicForward 127.0.0.1:1080
    Port 2222
    ProxyJump bastion,ops@10.20.0.5

Match host *.internal
    User nobody

Host *.corp !secret.corp
    User corpuser
    ProxyJump none

Host *
    LocalForward 9999 localhost:9999
    User fallback
    Port 22
"#;

    fn cfg() -> SshConfig {
        SshConfig::parse(CFG, Path::new("/home/me"))
    }

    #[test]
    fn aliases_skip_wildcards() {
        assert_eq!(cfg().aliases(), ["bastion", "prod-web", "prod-web-01", "stage-db"]);
    }

    #[test]
    fn first_match_wins_and_tilde_expands() {
        let b = cfg().entry("bastion");
        assert_eq!(b.hostname.as_deref(), Some("13.124.88.12"));
        assert_eq!(b.user.as_deref(), Some("ec2-user")); // not overridden by `Host *`
        assert_eq!(b.port, Some(22));
        assert_eq!(b.identity_file, Some(PathBuf::from("/home/me/.ssh/bastion.pem")));
    }

    #[test]
    fn resolves_nested_proxyjump() {
        let chain = cfg().resolve("stage-db", "me").unwrap();
        let got: Vec<_> = chain.iter().map(|h| (h.user.as_str(), h.host.as_str(), h.port)).collect();
        assert_eq!(got, [("ec2-user", "13.124.88.12", 22), ("ops", "10.20.0.5", 22), ("fallback", "10.20.2.15", 2222)]);
        assert!(matches!(&chain[0].auth, Auth::Auto { identity: Some(p), .. } if p.ends_with("bastion.pem")));
    }

    #[test]
    fn forwards_accumulate_across_blocks() {
        let f = cfg().entry("stage-db").forwards;
        assert_eq!(
            f,
            [
                ForwardSpec::Local { bind: "127.0.0.1".into(), port: 15432, host: "127.0.0.1".into(), host_port: 5432 },
                ForwardSpec::Dynamic { bind: "127.0.0.1".into(), port: 1080 },
                ForwardSpec::Local { bind: "127.0.0.1".into(), port: 9999, host: "localhost".into(), host_port: 9999 },
            ]
        );
    }

    #[test]
    fn negation_proxyjump_none_and_match_skipped() {
        let c = cfg();
        assert_eq!(c.entry("a.corp").user.as_deref(), Some("corpuser"));
        assert_eq!(c.entry("a.corp").proxy_jump, None);
        assert_eq!(c.entry("secret.corp").user.as_deref(), Some("fallback"));
        assert_eq!(c.entry("x.internal").user.as_deref(), Some("fallback"));
    }

    #[test]
    fn direct_spec_and_cycles() {
        let chain = cfg().resolve("root@1.2.3.4:2200", "me").unwrap();
        assert_eq!((chain[0].user.as_str(), chain[0].port), ("root", 2200));
        let looped = SshConfig::parse("Host a\n ProxyJump b\nHost b\n ProxyJump a\n", Path::new("/h"));
        assert!(looped.resolve("a", "me").is_err());
    }
}
