//! CoShell's own host store (`~/.config/coshell/hosts.toml`).
//!
//! - Hosts that only exist here are full definitions.
//! - Hosts from `~/.ssh/config` get an environment tag here; their connection
//!   settings stay in `~/.ssh/config` (never written by CoShell).
//!
//! Definitions are rendered as ssh-config blocks placed *before* the user's
//! config, so ProxyJump, `Host *` defaults and forwards resolve uniformly.

use crate::i18n::{tf, tr};
use std::path::{Path, PathBuf};

use coshell_ssh::ForwardSpec;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Env {
    #[default]
    None,
    Dev,
    Stage,
    Prod,
}

impl Env {
    pub const ALL: [Env; 4] = [Env::Prod, Env::Stage, Env::Dev, Env::None];

    pub fn label(self) -> &'static str {
        match self {
            Env::Prod => tr("운영"),
            Env::Stage => tr("스테이징"),
            Env::Dev => tr("개발"),
            Env::None => tr("없음"),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostDef {
    pub alias: String,
    #[serde(default)]
    pub env: Env,
    /// Display name (free text, e.g. "jumphost 서버"); never written to ssh config.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Group shown in the server list (e.g. "Softkr").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_jump: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_file: Option<String>,
    /// `L:[bind:]port:host:port`, `R:…`, `D:[bind:]port`
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forwards: Vec<String>,
}

/// A single ssh-config token: non-empty, no whitespace, no control characters.
fn token(field: &str, v: &str) -> Result<(), String> {
    if v.is_empty() || v.chars().any(|c| c.is_whitespace() || c.is_control() || c == '"' || c == '#') {
        return Err(tf("{0}: 공백·따옴표·#·제어 문자는 쓸 수 없습니다", &[&field]));
    }
    Ok(())
}

/// Free-text label (name / group): single line, no control characters.
fn label(field: &str, v: &str) -> Result<(), String> {
    if v.trim().is_empty() || v.chars().any(char::is_control) {
        return Err(tf("{0}: 비어 있거나 제어 문자가 있습니다", &[&field]));
    }
    Ok(())
}

/// ASCII alias from a display name ("jumphost 서버" → "jumphost").
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if (ch == '-' || ch == '_' || ch == '.' || ch.is_whitespace()) && !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// `L:8080:127.0.0.1:80` → spec.
pub fn parse_forward(s: &str) -> Result<ForwardSpec, String> {
    let (kind, rest) = s.split_once(':').ok_or_else(|| tf("형식: L:포트:호스트:포트 / R:… / D:포트 ({0})", &[&s]))?;
    let mut k = kind.chars();
    match (k.next(), k.next()) {
        (Some(c), None) => ForwardSpec::parse(c, rest),
        _ => Err(tf("터널 종류는 L, R, D 중 하나입니다 ({0})", &[&s])),
    }
}

fn forward_line(f: &ForwardSpec) -> String {
    match f {
        ForwardSpec::Local { bind, port, host, host_port } => format!("LocalForward {bind}:{port} {host}:{host_port}"),
        ForwardSpec::Remote { bind, port, host, host_port } => format!("RemoteForward {bind}:{port} {host}:{host_port}"),
        ForwardSpec::Dynamic { bind, port } => format!("DynamicForward {bind}:{port}"),
    }
}

impl HostDef {
    /// Has connection settings (vs. a tag-only overlay for a ~/.ssh/config host).
    pub fn is_definition(&self) -> bool {
        self.hostname.is_some()
            || self.user.is_some()
            || self.port.is_some()
            || self.proxy_jump.is_some()
            || self.identity_file.is_some()
            || !self.forwards.is_empty()
    }

    /// Label for lists: display name, else alias.
    pub fn display(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.alias)
    }

    pub fn validate(&self) -> Result<(), String> {
        token(tr("별칭"), &self.alias)?;
        if let Some(n) = &self.name {
            label(tr("이름"), n)?;
        }
        if let Some(g) = &self.group {
            label(tr("그룹"), g)?;
        }
        if self.alias.contains(['*', '?', '!', ',']) {
            return Err(tr("별칭에 * ? ! , 는 쓸 수 없습니다").into());
        }
        for (name, v) in [(tr("주소"), &self.hostname), (tr("사용자"), &self.user), (tr("경유 서버"), &self.proxy_jump), (tr("SSH 키 파일"), &self.identity_file)] {
            if let Some(v) = v {
                token(name, v)?;
            }
        }
        if self.proxy_jump.as_deref().is_some_and(|j| j.split(',').any(|x| x == self.alias)) {
            return Err(tr("자기 자신을 경유할 수 없습니다").into());
        }
        if self.port == Some(0) {
            return Err(tr("포트는 1–65535 사이의 숫자로 입력하세요").into());
        }
        for f in &self.forwards {
            parse_forward(f)?;
        }
        Ok(())
    }

    fn to_ssh_config(&self) -> String {
        let mut s = format!("Host {}\n", self.alias);
        let mut kv = |k: &str, v: &Option<String>| {
            if let Some(v) = v {
                s.push_str(&format!("  {k} {v}\n"));
            }
        };
        kv("HostName", &self.hostname);
        kv("User", &self.user);
        kv("Port", &self.port.map(|p| p.to_string()));
        kv("ProxyJump", &self.proxy_jump);
        kv("IdentityFile", &self.identity_file);
        for f in self.forwards.iter().filter_map(|f| parse_forward(f).ok()) {
            s.push_str(&format!("  {}\n", forward_line(&f)));
        }
        s
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostStore {
    #[serde(default, rename = "host")]
    pub hosts: Vec<HostDef>,
}

impl HostStore {
    pub fn default_path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::home_dir().map(|h| h.join(".config")))?;
        Some(base.join("coshell").join("hosts.toml"))
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(t) => toml::from_str(&t).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())
    }

    pub fn get(&self, alias: &str) -> Option<&HostDef> {
        self.hosts.iter().find(|h| h.alias == alias)
    }

    pub fn env_of(&self, alias: &str) -> Env {
        self.get(alias).map(|h| h.env).unwrap_or_default()
    }

    /// Insert or replace (renames drop the old alias).
    pub fn upsert(&mut self, original: Option<&str>, def: HostDef) {
        let key = original.unwrap_or(&def.alias).to_string();
        self.hosts.retain(|h| h.alias != key && h.alias != def.alias);
        // Overlays with nothing to say are not worth keeping.
        if def.is_definition() || def.env != Env::None || def.name.is_some() || def.group.is_some() {
            self.hosts.push(def);
        }
    }

    pub fn remove(&mut self, alias: &str) {
        self.hosts.retain(|h| h.alias != alias);
    }

    /// Definitions as ssh-config text (prepended to the user's config).
    pub fn to_ssh_config(&self) -> String {
        self.hosts.iter().filter(|h| h.is_definition() && h.validate().is_ok()).map(HostDef::to_ssh_config).collect::<Vec<_>>().join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coshell_ssh::config::SshConfig;

    fn def(alias: &str) -> HostDef {
        HostDef { alias: alias.into(), ..Default::default() }
    }

    #[test]
    fn toml_roundtrip_and_overlay_vs_definition() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.toml");
        let mut s = HostStore::default();
        s.upsert(None, HostDef { env: Env::Prod, ..def("bastion") });
        s.upsert(None, HostDef { env: Env::Stage, hostname: Some("10.20.2.15".into()), proxy_jump: Some("bastion".into()), forwards: vec!["L:15432:127.0.0.1:5432".into()], ..def("stage-db") });
        s.upsert(None, def("noise")); // nothing to store
        s.save(&path).unwrap();
        let back = HostStore::load(&path).unwrap();
        assert_eq!(back, s);
        assert_eq!(back.hosts.len(), 2);
        assert!(!back.hosts[0].is_definition() && back.hosts[1].is_definition());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[[host]]") && text.contains("env = \"prod\""));
    }

    #[test]
    fn rejects_config_injection_and_bad_values() {
        let bad = [
            HostDef { hostname: Some("10.0.0.1\n  ProxyCommand evil".into()), ..def("a") },
            HostDef { user: Some("root extra".into()), ..def("a") },
            def("web*"),
            HostDef { proxy_jump: Some("a".into()), ..def("a") },
            HostDef { forwards: vec!["X:1:2:3".into()], ..def("a") },
            HostDef { port: Some(0), ..def("a") },
        ];
        for b in bad {
            assert!(b.validate().is_err(), "{b:?}");
        }
        assert!(HostDef { hostname: Some("10.0.0.1".into()), forwards: vec!["D:1080".into()], ..def("ok-host") }.validate().is_ok());
    }

    #[test]
    fn definitions_resolve_through_user_config() {
        let mut s = HostStore::default();
        s.upsert(None, HostDef { env: Env::Prod, hostname: Some("10.0.1.42".into()), user: Some("ubuntu".into()), proxy_jump: Some("bastion".into()), forwards: vec!["L:8080:127.0.0.1:80".into()], ..def("prod-web") });
        let user_cfg = "Host bastion\n  HostName 13.124.88.12\n  User ec2-user\nHost *\n  Port 22\n";
        let cfg = SshConfig::parse(&format!("{}\n{user_cfg}", s.to_ssh_config()), Path::new("/h"));
        let chain = cfg.resolve("prod-web", "me").unwrap();
        let got: Vec<_> = chain.iter().map(|h| (h.user.as_str(), h.host.as_str())).collect();
        assert_eq!(got, [("ec2-user", "13.124.88.12"), ("ubuntu", "10.0.1.42")]);
        assert_eq!(cfg.entry("prod-web").forwards, [ForwardSpec::parse('L', "8080:127.0.0.1:80").unwrap()]);
        assert_eq!(cfg.aliases(), ["prod-web", "bastion"]);
    }

    #[test]
    fn slugs_and_labels() {
        assert_eq!(slugify("jumphost 서버"), "jumphost");
        assert_eq!(slugify("mac-server"), "mac-server");
        assert_eq!(slugify("Raspberry Pi 4"), "raspberry-pi-4");
        assert_eq!(slugify("라즈베리파이"), "");
        let named = HostDef { name: Some("테스트 서버".into()), group: Some("hanwha".into()), hostname: Some("10.0.0.7".into()), ..def("test") };
        assert!(named.validate().is_ok());
        assert_eq!(named.display(), "테스트 서버");
        assert!(HostDef { name: Some("a\nb".into()), ..def("x") }.validate().is_err());
        // Name/group-only overlays (for ~/.ssh/config hosts) are kept.
        let mut s = HostStore::default();
        s.upsert(None, HostDef { group: Some("Local".into()), ..def("pi") });
        assert_eq!(s.hosts.len(), 1);
    }

    #[test]
    fn rename_replaces_old_alias() {
        let mut s = HostStore::default();
        s.upsert(None, HostDef { hostname: Some("1.1.1.1".into()), ..def("old") });
        s.upsert(Some("old"), HostDef { hostname: Some("1.1.1.1".into()), ..def("new") });
        assert_eq!(s.hosts.iter().map(|h| h.alias.as_str()).collect::<Vec<_>>(), ["new"]);
    }
}
