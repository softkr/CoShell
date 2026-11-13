//! What a private key file is: algorithm, public half, passphrase or not.
//! Used to list keys and to check a key before it is saved.

use std::path::Path;

use russh::keys::{Algorithm, PrivateKey, decode_secret_key};

/// Larger files are not private keys.
const MAX_KEY_BYTES: u64 = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyInfo {
    /// Short label: `ED25519`, `RSA`, `ECDSA`, …
    pub algorithm: String,
    /// `ssh-ed25519 AAAA… comment`, for the server's authorized_keys.
    pub public: Option<String>,
    /// Needs a passphrase (asked by ssh-agent; not supported for direct use yet).
    pub encrypted: bool,
}

fn label(a: &Algorithm) -> String {
    match a {
        Algorithm::Ed25519 => "ED25519".into(),
        Algorithm::Rsa { .. } => "RSA".into(),
        Algorithm::Ecdsa { .. } => "ECDSA".into(),
        Algorithm::Dsa => "DSA".into(),
        other => other.as_str().to_uppercase(),
    }
}

fn info(k: &PrivateKey) -> KeyInfo {
    KeyInfo { algorithm: label(&k.algorithm()), public: k.public_key().to_openssh().ok(), encrypted: k.is_encrypted() }
}

/// Inspect private key text (OpenSSH, PEM or PuTTY).
pub fn inspect(text: &str) -> Result<KeyInfo, String> {
    let t = text.trim();
    if !(t.contains("PRIVATE KEY") || t.starts_with("PuTTY-User-Key-File-")) {
        return Err(if crate::english() { "Not a private key".into() } else { "개인 키가 아닙니다".into() });
    }
    // OpenSSH format: readable without the passphrase (the public half is in clear).
    if let Ok(k) = PrivateKey::from_openssh(t) {
        return Ok(info(&k));
    }
    match decode_secret_key(t, None) {
        Ok(k) => Ok(info(&k)),
        Err(russh::keys::Error::KeyIsEncrypted) => Ok(KeyInfo { algorithm: "?".into(), public: None, encrypted: true }),
        Err(e) => Err(if crate::english() { format!("Unreadable key: {e}") } else { format!("읽을 수 없는 키입니다: {e}") }),
    }
}

/// Inspect a key file; `Err` for anything that is not a private key.
pub fn inspect_file(path: &Path) -> Result<KeyInfo, String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.len() > MAX_KEY_BYTES {
        return Err(if crate::english() { "Not a private key".into() } else { "개인 키가 아닙니다".into() });
    }
    inspect(&std::fs::read_to_string(path).map_err(|e| e.to_string())?)
}

/// Whether `passphrase` unlocks the key.
pub fn check_passphrase(text: &str, passphrase: &str) -> Result<(), String> {
    match decode_secret_key(text.trim(), Some(passphrase)) {
        Ok(_) => Ok(()),
        Err(_) => Err(if crate::english() { "Wrong passphrase".into() } else { "암호가 맞지 않습니다".into() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Throwaway keys generated for these tests only.
    const PLAIN: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACCupRzXAGYmowrqEFkAqjdtyvtD0QxKHmqOXVtSzFTGiQAAAIgfyi2uH8ot
rgAAAAtzc2gtZWQyNTUxOQAAACCupRzXAGYmowrqEFkAqjdtyvtD0QxKHmqOXVtSzFTGiQ
AAAEBanwDBbhObOrD2XHhiR/WzwthNSeXIqywbhUIIrD7yb66lHNcAZiajCuoQWQCqN23K
+0PRDEoeao5dW1LMVMaJAAAABHRlc3QB
-----END OPENSSH PRIVATE KEY-----
";
    /// Passphrase `pw`.
    const ENCRYPTED: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAACmFlczI1Ni1jdHIAAAAGYmNyeXB0AAAAGAAAABBVctzBIt
AnwQEs/5bNwgZ1AAAAGAAAAAEAAAAzAAAAC3NzaC1lZDI1NTE5AAAAIHEK6QZ//+nnRihM
7IJZ23aStYHnQ36HAvW7IeF5EaRAAAAAkNN+ucdNAcm3q4Sao1GJUCG/Ifr2QCorNxgLw1
7wx8sVRniKc+tgQITrG6Z6ERr/ANgTGV3ZpCLuKsUg8U85T4vxfzdbhqEYaxAny/YCod7n
J50yCVMeApycgdYG6T+PEQHaQvFzU3YpIFOGAaWLcSp6aSlSEP+0V/uwsoxtwi4Iia4FRo
5e/uW6mCMpqV44Ow==
-----END OPENSSH PRIVATE KEY-----
";

    #[test]
    fn reads_algorithm_and_public_half() {
        let i = inspect(PLAIN).unwrap();
        assert_eq!(i.algorithm, "ED25519");
        assert_eq!(i.public.as_deref().map(|p| p.split(' ').next().unwrap()), Some("ssh-ed25519"));
        assert!(!i.encrypted);
    }

    #[test]
    fn encrypted_key_is_listed_and_passphrase_checked() {
        let i = inspect(ENCRYPTED).unwrap();
        assert!(i.encrypted);
        assert_eq!(i.algorithm, "ED25519", "readable without the passphrase");
        assert!(check_passphrase(ENCRYPTED, "pw").is_ok());
        assert!(check_passphrase(ENCRYPTED, "nope").is_err());
    }

    #[test]
    fn rejects_other_text() {
        assert!(inspect("ssh-ed25519 AAAAC3Nza... me@host").is_err(), "a public key is not a private key");
        assert!(inspect("hello").is_err());
    }
}
