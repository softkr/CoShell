//! AI provider profiles (`~/.config/coshell/ai.toml`) and API-key storage.
//! Keys never touch the TOML file: they live in the OS keychain, with
//! `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` as a read-only fallback.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// Keychain service name for API keys.
pub const KEYCHAIN_SERVICE: &str = "CoShell AI";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Anthropic,
    #[serde(rename = "openai")]
    OpenAi,
}

impl Format {
    pub fn label(self) -> &'static str {
        match self {
            Format::Anthropic => "Anthropic Messages",
            Format::OpenAi => "OpenAI Chat Completions",
        }
    }
    pub fn default_base(self) -> &'static str {
        match self {
            Format::Anthropic => "https://api.anthropic.com",
            Format::OpenAi => "https://api.openai.com",
        }
    }
    fn env_key(self) -> &'static str {
        match self {
            Format::Anthropic => "ANTHROPIC_API_KEY",
            Format::OpenAi => "OPENAI_API_KEY",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub format: Format,
    pub base_url: String,
    pub model: String,
}

impl Profile {
    /// Local endpoints (Ollama …) usually need no key.
    pub fn is_local(&self) -> bool {
        let b = self.base_url.to_ascii_lowercase();
        b.contains("://localhost") || b.contains("://127.0.0.1") || b.contains("://[::1]")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileStore {
    pub active: String,
    pub profiles: Vec<Profile>,
}

impl Default for ProfileStore {
    fn default() -> Self {
        let p = |id: &str, name: &str, format, base: &str, model: &str| Profile {
            id: id.into(),
            name: name.into(),
            format,
            base_url: base.into(),
            model: model.into(),
        };
        Self {
            active: "claude".into(),
            profiles: vec![
                p("claude", "Claude", Format::Anthropic, "https://api.anthropic.com", "claude-sonnet-5-5"),
                p("openai", "OpenAI", Format::OpenAi, "https://api.openai.com", "gpt-5"),
                p("ollama", if crate::english() { "Ollama (local)" } else { "Ollama (로컬)" }, Format::OpenAi, "http://localhost:11434", "llama3.1"),
            ],
        }
    }
}

impl ProfileStore {
    /// `$XDG_CONFIG_HOME/coshell/ai.toml` or `~/.config/coshell/ai.toml`.
    pub fn default_path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("coshell").join("ai.toml"))
    }

    /// Load, or defaults when the file does not exist.
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let s: Self = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
                if s.profiles.is_empty() { Ok(Self::default()) } else { Ok(s) }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())
    }

    pub fn active(&self) -> &Profile {
        self.profiles.iter().find(|p| p.id == self.active).unwrap_or(&self.profiles[0])
    }
}

/// Where API keys are kept.
pub trait SecretStore: Send + Sync {
    fn get(&self, id: &str) -> Option<Zeroizing<String>>;
    fn set(&self, id: &str, secret: &str) -> Result<(), String>;
    fn delete(&self, id: &str) -> Result<(), String>;
}

/// OS keychain (macOS Keychain, Windows Credential Manager, Secret Service),
/// items filed under one service name.
pub struct Keychain(pub &'static str);

impl Keychain {
    /// AI API keys.
    pub const AI: Keychain = Keychain(KEYCHAIN_SERVICE);
}

impl SecretStore for Keychain {
    fn get(&self, id: &str) -> Option<Zeroizing<String>> {
        keyring::Entry::new(self.0, id).ok()?.get_password().ok().map(Zeroizing::new)
    }
    fn set(&self, id: &str, secret: &str) -> Result<(), String> {
        keyring::Entry::new(self.0, id).and_then(|e| e.set_password(secret)).map_err(|e| e.to_string())
    }
    fn delete(&self, id: &str) -> Result<(), String> {
        match keyring::Entry::new(self.0, id).and_then(|e| e.delete_credential()) {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeySource {
    Keychain,
    Env,
    None,
}

/// Key for `profile`: keychain first, then the provider's env var.
pub fn resolve_key(profile: &Profile, store: &dyn SecretStore) -> (Option<Zeroizing<String>>, KeySource) {
    if let Some(k) = store.get(&profile.id) {
        return (Some(k), KeySource::Keychain);
    }
    match std::env::var(profile.format.env_key()) {
        Ok(k) if !k.is_empty() => (Some(Zeroizing::new(k)), KeySource::Env),
        _ => (None, KeySource::None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("ai.toml");
        assert_eq!(ProfileStore::load(&path).unwrap(), ProfileStore::default());
        let s = ProfileStore { active: "ollama".into(), ..ProfileStore::default() };
        s.save(&path).unwrap();
        let back = ProfileStore::load(&path).unwrap();
        assert_eq!(back, s);
        assert_eq!(back.active().format, Format::OpenAi);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("format = \"openai\"") && text.contains("format = \"anthropic\""));
        assert!(!text.to_lowercase().contains("key"), "no secrets in the file");
    }

    #[test]
    fn local_detection() {
        let s = ProfileStore::default();
        assert!(s.profiles[2].is_local());
        assert!(!s.profiles[0].is_local());
    }
}
