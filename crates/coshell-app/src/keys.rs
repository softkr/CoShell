//! SSH keys the user can pick for a server: private keys found in ~/.ssh,
//! plus keys imported into CoShell's own folder (`…/coshell/keys/<name>`).

use std::path::{Path, PathBuf};

use coshell_ssh::keys::{KeyInfo, inspect_file};

use crate::i18n::tr;

/// Files in ~/.ssh that are never private keys.
const NOT_KEYS: [&str; 5] = ["config", "known_hosts", "known_hosts.old", "authorized_keys", "environment"];
/// Side files next to an imported key.
const PUB_EXT: &str = "pub";
const NOTE_EXT: &str = "note";

#[derive(Clone, Debug)]
pub struct SshKey {
    pub name: String,
    pub path: PathBuf,
    pub info: KeyInfo,
    /// Imported into CoShell (can be deleted from the app).
    pub managed: bool,
    pub note: Option<String>,
}

/// `<key>.pub` / `<key>.note` (appended, so `my.key` keeps its own extension).
fn side(path: &Path, ext: &str) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".");
    s.push(ext);
    PathBuf::from(s)
}

fn private_keys_in(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<(String, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let p = e.path();
            let side = matches!(p.extension().and_then(|x| x.to_str()), Some(PUB_EXT | NOTE_EXT));
            (!name.starts_with('.') && !side && !NOT_KEYS.contains(&name.as_str())).then_some((name, p))
        })
        .collect();
    out.sort();
    out
}

/// Imported keys first, then ~/.ssh. Unreadable files are skipped.
pub fn discover(home: &Path, managed_dir: Option<&Path>) -> Vec<SshKey> {
    let mut keys = Vec::new();
    for (dir, managed) in managed_dir.map(|d| (d.to_path_buf(), true)).into_iter().chain([(home.join(".ssh"), false)]) {
        for (name, path) in private_keys_in(&dir) {
            if let Ok(info) = inspect_file(&path) {
                let note = managed
                    .then(|| std::fs::read_to_string(side(&path, NOTE_EXT)).ok())
                    .flatten()
                    .map(|n| n.trim().to_string())
                    .filter(|n| !n.is_empty());
                keys.push(SshKey { name, path, info, managed, note });
            }
        }
    }
    keys
}

/// A key name doubles as its file name.
pub fn check_name(name: &str) -> Result<(), String> {
    let n = name.trim();
    let bad = n.is_empty()
        || n.starts_with('.')
        || n.chars().any(|c| c == '/' || c == '\\' || c.is_control())
        || n.ends_with(&format!(".{PUB_EXT}"))
        || n.ends_with(&format!(".{NOTE_EXT}"));
    if bad { Err(tr("키 이름에는 / 와 맨 앞 . 을 쓸 수 없습니다").into()) } else { Ok(()) }
}

/// Write an imported key (owner-only), its public half and note. Returns the key path.
pub fn save(dir: &Path, name: &str, private: &str, public: Option<&str>, note: &str) -> Result<PathBuf, String> {
    check_name(name)?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = dir.join(name.trim());
    if path.exists() {
        return Err(tr("같은 이름의 키가 이미 있습니다").into());
    }
    write_private(&path, &format!("{}\n", private.trim()))?;
    if let Some(p) = public {
        std::fs::write(side(&path, PUB_EXT), format!("{p}\n")).map_err(|e| e.to_string())?;
    }
    if !note.trim().is_empty() {
        std::fs::write(side(&path, NOTE_EXT), note.trim()).map_err(|e| e.to_string())?;
    }
    Ok(path)
}

#[cfg(unix)]
fn write_private(path: &Path, text: &str) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path).map_err(|e| e.to_string())?;
    f.write_all(text.as_bytes()).map_err(|e| e.to_string())
}

#[cfg(not(unix))]
fn write_private(path: &Path, text: &str) -> Result<(), String> {
    std::fs::write(path, text).map_err(|e| e.to_string())
}

/// Remove an imported key and its side files.
pub fn delete(path: &Path) -> Result<(), String> {
    std::fs::remove_file(path).map_err(|e| e.to_string())?;
    for ext in [PUB_EXT, NOTE_EXT] {
        let _ = std::fs::remove_file(side(path, ext));
    }
    Ok(())
}

/// `~/…` form for config files when the path is under home.
pub fn display_path(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if !home.as_os_str().is_empty() => format!("~/{}", rest.display()),
        _ => path.display().to_string(),
    }
}

/// `~/x` → `<home>/x`.
pub fn expand(path: &str, home: &Path) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => PathBuf::from(path),
    }
}
