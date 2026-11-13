//! Updates: look for a newer GitHub release every few minutes and, on request,
//! download its DMG, check it against the SHA-256 in the release notes,
//! swap it in for the running app bundle and relaunch.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::i18n::{tf, tr};

/// Where releases are published.
const LATEST: &str = "https://api.github.com/repos/softkr/CoShell/releases/latest";
/// How often to look.
pub const CHECK_EVERY: Duration = Duration::from_secs(10 * 60);
const TIMEOUT: Duration = Duration::from_secs(20);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// A release newer than this build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    /// Release page (shown when it cannot be installed in place).
    pub page: String,
    /// The `.dmg` asset.
    pub dmg: Option<String>,
    /// `SHA-256 (`….dmg`): `…`` line of the notes.
    pub sha256: Option<String>,
}

fn parts(v: &str) -> Option<(u64, u64, u64)> {
    let mut it = v.trim().trim_start_matches('v').split('-').next()?.split('.').map(|p| p.parse::<u64>().ok());
    Some((it.next()??, it.next().flatten().unwrap_or(0), it.next().flatten().unwrap_or(0)))
}

/// `candidate` is a later version than `current` (`v` prefix and pre-release suffix ignored).
pub fn is_newer(current: &str, candidate: &str) -> bool {
    matches!((parts(current), parts(candidate)), (Some(a), Some(b)) if b > a)
}

/// The hash the notes give for `file`.
fn notes_sha256(notes: &str, file: &str) -> Option<String> {
    notes.lines().filter(|l| l.contains("SHA-256") && l.contains(file)).find_map(|l| {
        l.split('`').map(str::trim).find(|w| w.len() == 64 && w.bytes().all(|b| b.is_ascii_hexdigit())).map(str::to_ascii_lowercase)
    })
}

/// A GitHub "latest release" response, if it is newer than `current`.
pub fn parse_latest(v: &Value, current: &str) -> Option<Release> {
    if v["draft"].as_bool() == Some(true) || v["prerelease"].as_bool() == Some(true) {
        return None;
    }
    let version = v["tag_name"].as_str()?.trim_start_matches('v').to_string();
    if !is_newer(current, &version) {
        return None;
    }
    let asset = v["assets"].as_array().into_iter().flatten().find(|a| a["name"].as_str().is_some_and(|n| n.ends_with(".dmg")));
    let name = asset.and_then(|a| a["name"].as_str()).unwrap_or_default();
    Some(Release {
        page: v["html_url"].as_str().unwrap_or_default().to_string(),
        dmg: asset.and_then(|a| a["browser_download_url"].as_str()).map(String::from),
        sha256: notes_sha256(v["body"].as_str().unwrap_or_default(), name),
        version,
    })
}

fn client(timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(concat!("CoShell/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .timeout(timeout)
        .build()
        .map_err(|e| e.to_string())
}

/// Ask GitHub for the latest release. `Ok(None)` = up to date.
pub async fn check() -> Result<Option<Release>, String> {
    let resp = client(TIMEOUT)?
        .get(LATEST)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| e.without_url().to_string())?;
    match resp.status().as_u16() {
        200 => {}
        404 => return Err(tr("릴리스를 찾을 수 없습니다 (저장소가 비공개이거나 릴리스가 없음)").into()),
        s => return Err(tf("업데이트 서버 응답 {0}", &[&s])),
    }
    let v: Value = resp.json().await.map_err(|e| e.to_string())?;
    Ok(parse_latest(&v, env!("CARGO_PKG_VERSION")))
}

/// The `.app` bundle this process runs from (`None` for `cargo run` builds).
pub fn running_bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let app = exe.parent()?.parent()?.parent()?; // …/CoShell.app/Contents/MacOS/coshell
    (app.extension().and_then(|e| e.to_str()) == Some("app")).then(|| app.to_path_buf())
}

/// Lower-case hex SHA-256.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Download, verify and put the new bundle in place of `target`. Returns the bundle to relaunch.
pub async fn install(release: Release, target: PathBuf) -> Result<PathBuf, String> {
    let url = release.dmg.ok_or_else(|| tr("이 릴리스에는 설치 파일(.dmg)이 없습니다").to_string())?;
    let want = release.sha256.ok_or_else(|| tr("릴리스 노트에 SHA-256이 없어 설치하지 않았습니다").to_string())?;
    let bytes = client(DOWNLOAD_TIMEOUT)?
        .get(&url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| e.without_url().to_string())?
        .bytes()
        .await
        .map_err(|e| e.to_string())?;
    if sha256_hex(&bytes) != want {
        return Err(tr("내려받은 파일의 SHA-256이 릴리스 노트와 다릅니다. 설치하지 않았습니다.").into());
    }
    let work = std::env::temp_dir().join(format!("coshell-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let dmg = work.join("CoShell.dmg");
    std::fs::write(&dmg, &bytes).map_err(|e| e.to_string())?;
    let result = swap_in(&dmg, &work.join("mnt"), &target);
    let _ = std::fs::remove_dir_all(&work);
    result.map(|()| target)
}

fn run(cmd: &str, args: &[&std::ffi::OsStr]) -> Result<(), String> {
    let out = std::process::Command::new(cmd).args(args).output().map_err(|e| format!("{cmd}: {e}"))?;
    if out.status.success() { Ok(()) } else { Err(format!("{cmd}: {}", String::from_utf8_lossy(&out.stderr).trim())) }
}

/// Mount the DMG, copy its app next to `target`, then exchange them.
fn swap_in(dmg: &Path, mount: &Path, target: &Path) -> Result<(), String> {
    let os = |p: &Path| p.as_os_str().to_owned();
    run("hdiutil", &[ "attach".as_ref(), "-nobrowse".as_ref(), "-readonly".as_ref(), "-mountpoint".as_ref(), &os(mount), &os(dmg)])?;
    let parent = target.parent().ok_or("no parent folder")?;
    let staged = parent.join(".CoShell-update.app");
    let old = parent.join(".CoShell-old.app");
    let _ = std::fs::remove_dir_all(&staged);
    let copied = run("ditto", &[&os(&mount.join("CoShell.app")), &os(&staged)]); // ditto keeps the signature
    let _ = run("hdiutil", &["detach".as_ref(), "-quiet".as_ref(), &os(mount)]);
    copied?;
    let _ = std::fs::remove_dir_all(&old);
    std::fs::rename(target, &old).map_err(|e| tf("앱을 바꾸지 못했습니다 ({0}): {1}", &[&target.display(), &e]))?;
    if let Err(e) = std::fs::rename(&staged, target) {
        let _ = std::fs::rename(&old, target); // put the running version back
        return Err(tf("앱을 바꾸지 못했습니다 ({0}): {1}", &[&target.display(), &e]));
    }
    let _ = std::fs::remove_dir_all(&old);
    Ok(())
}

/// Start the freshly installed bundle (the caller then exits).
pub fn relaunch(bundle: &Path) -> Result<(), String> {
    std::process::Command::new("open").arg("-n").arg(bundle).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("0.3.0", "0.4.0"));
        assert!(is_newer("0.3.0", "v0.10.0"));
        assert!(is_newer("0.9.9", "1.0"));
        assert!(!is_newer("0.3.0", "0.3.0"));
        assert!(!is_newer("0.3.0", "0.2.9"));
        assert!(!is_newer("0.3.0", "nightly"));
    }

    #[test]
    fn latest_release_with_dmg_and_hash() {
        let hash = "394429f6f45ccf2b51a83c4456b15418dd602c7932019d1dfc5ef3aacea6835f";
        let v = serde_json::json!({
            "tag_name": "v0.4.0",
            "html_url": "https://github.com/softkr/CoShell/releases/tag/v0.4.0",
            "draft": false,
            "prerelease": false,
            "body": format!("## 설치\n…\nSHA-256 (`CoShell-0.4.0.dmg`): `{hash}`"),
            "assets": [{"name": "CoShell-0.4.0.dmg", "browser_download_url": "https://example.com/CoShell-0.4.0.dmg"}],
        });
        let r = parse_latest(&v, "0.3.0").expect("newer");
        assert_eq!(r.version, "0.4.0");
        assert_eq!(r.dmg.as_deref(), Some("https://example.com/CoShell-0.4.0.dmg"));
        assert_eq!(r.sha256.as_deref(), Some(hash));
        assert_eq!(parse_latest(&v, "0.4.0"), None, "same version");
        let mut pre = v.clone();
        pre["prerelease"] = true.into();
        assert_eq!(parse_latest(&pre, "0.3.0"), None, "pre-releases are not offered");
    }

    /// Real DMG, throwaway target: `COSHELL_TEST_DMG=… cargo test -p coshell-app swaps -- --ignored`.
    #[test]
    #[ignore = "mounts a DMG"]
    fn swaps_the_bundle_from_a_dmg() {
        let Ok(dmg) = std::env::var("COSHELL_TEST_DMG") else { return };
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("CoShell.app");
        std::fs::create_dir_all(target.join("Contents")).unwrap();
        std::fs::write(target.join("Contents/old-marker"), "").unwrap();
        swap_in(Path::new(&dmg), &dir.path().join("mnt"), &target).unwrap();
        assert!(target.join("Contents/MacOS/coshell").exists(), "new bundle in place");
        assert!(!target.join("Contents/old-marker").exists(), "old one gone");
        assert!(!dir.path().join(".CoShell-old.app").exists() && !dir.path().join(".CoShell-update.app").exists());
        let sig = std::process::Command::new("codesign").arg("-v").arg(&target).status().unwrap();
        assert!(sig.success(), "signature intact after the copy");
    }

    #[test]
    fn digest() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
