//! User authentication for one hop.
//!
//! First the configured method runs; then, while the server answers
//! "failure" with keyboard-interactive / password still allowed, the
//! [`Prompter`] is asked (2FA after publickey = partial success, OTP-only
//! bastions, password prompts). Rounds are bounded.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use russh::MethodKind;
use russh::client::{AuthResult, KeyboardInteractiveAuthResponse};
use russh::keys::agent::client::AgentClient;
use russh::keys::{PrivateKey, PrivateKeyWithHashAlg, load_secret_key};
use zeroize::Zeroizing;

use crate::prompt::{InteractivePrompt, InteractiveQuery, Prompter};
use crate::{Auth, Handle, HostSpec, SshError};

/// Prompted auth rounds before giving up (OpenSSH NumberOfPasswordPrompts).
const MAX_PROMPT_ROUNDS: usize = 3;
/// Info-request exchanges inside one keyboard-interactive round.
const MAX_INFO_REQUESTS: usize = 8;
/// Default identities tried by `Auth::Auto` when none is configured.
const DEFAULT_IDENTITIES: [&str; 3] = ["id_ed25519", "id_ecdsa", "id_rsa"];

enum Outcome {
    Success,
    Failure { remaining: Vec<MethodKind>, partial: bool },
}

impl From<AuthResult> for Outcome {
    fn from(r: AuthResult) -> Self {
        match r {
            AuthResult::Success => Outcome::Success,
            AuthResult::Failure { remaining_methods, partial_success } => {
                Outcome::Failure { remaining: remaining_methods.to_vec(), partial: partial_success }
            }
        }
    }
}

struct Ctx<'a> {
    hop: &'a HostSpec,
    index: usize,
    label: String,
    prompter: Option<&'a Arc<dyn Prompter>>,
}

impl Ctx<'_> {
    fn ssh(&self) -> impl Fn(russh::Error) -> SshError + '_ {
        |source| SshError::Ssh { hop: self.label.clone(), source }
    }

    async fn ask(
        &self,
        name: &str,
        instructions: &str,
        prompts: Vec<InteractivePrompt>,
        password: bool,
    ) -> Result<Vec<Zeroizing<String>>, SshError> {
        let p = self.prompter.ok_or_else(|| SshError::AuthFailed(self.label.clone()))?;
        let n = prompts.len();
        let q = InteractiveQuery {
            hop: self.index,
            label: self.label.clone(),
            name: name.into(),
            instructions: instructions.into(),
            prompts,
            password,
        };
        match p.interactive(q).await {
            Some(a) if a.len() == n => Ok(a),
            Some(_) => Err(SshError::AuthFailed(self.label.clone())),
            None => Err(SshError::Cancelled(self.label.clone())),
        }
    }
}

pub(crate) async fn authenticate(
    handle: &mut Handle,
    hop: &HostSpec,
    index: usize,
    prompter: Option<&Arc<dyn Prompter>>,
) -> Result<(), SshError> {
    let cx = Ctx { hop, index, label: hop.label(), prompter };
    let user = hop.user.clone();
    let mut state: Outcome = match &hop.auth {
        Auth::Password(pw) => handle.authenticate_password(&user, pw.as_str()).await.map_err(cx.ssh())?.into(),
        Auth::KeyFile { path, passphrase } => {
            let key = load_key(&cx, path, passphrase.clone()).await?;
            publickey(handle, &cx, key).await?
        }
        Auth::Agent => agent(handle, &cx).await?.unwrap_or(Outcome::Failure { remaining: vec![], partial: false }),
        Auth::Interactive => handle.authenticate_none(&user).await.map_err(cx.ssh())?.into(),
        Auth::Auto { identity, .. } => auto(handle, &cx, identity.as_deref()).await?,
    };

    let may_prompt = matches!(hop.auth, Auth::Interactive | Auth::Auto { .. });
    // Saved password: used at most once, then the user is asked as usual.
    let mut saved = match &hop.auth {
        Auth::Auto { password, .. } => password.clone(),
        _ => None,
    };
    let rounds = MAX_PROMPT_ROUNDS + usize::from(saved.is_some());
    // Once a first factor succeeded, a failed second factor comes back with
    // partial=false; keep retrying it (bounded) like OpenSSH does.
    let mut second_factor = false;
    for _ in 0..rounds {
        let Outcome::Failure { remaining, partial } = &state else { return Ok(()) };
        second_factor |= *partial;
        if remaining.contains(&MethodKind::Password)
            && let Some(pw) = saved.take()
        {
            state = handle.authenticate_password(&user, pw.as_str()).await.map_err(cx.ssh())?.into();
            continue;
        }
        if (cx.prompter.is_none() && saved.is_none()) || !(second_factor || may_prompt) {
            break;
        }
        state = if remaining.contains(&MethodKind::KeyboardInteractive) {
            keyboard_interactive(handle, &cx, &mut saved).await?
        } else if remaining.contains(&MethodKind::Password) {
            let prompt = InteractivePrompt {
                text: if crate::english() { format!("Password for {}", cx.label) } else { format!("{} 비밀번호", cx.label) },
                echo: false,
            };
            let pw = cx.ask("", "", vec![prompt], true).await?;
            handle.authenticate_password(&user, pw[0].as_str()).await.map_err(cx.ssh())?.into()
        } else {
            break;
        };
    }
    match state {
        Outcome::Success => Ok(()),
        Outcome::Failure { .. } => Err(SshError::AuthFailed(cx.label)),
    }
}

/// Agent → configured/default key files → (prompts handled by caller).
async fn auto(handle: &mut Handle, cx: &Ctx<'_>, identity: Option<&Path>) -> Result<Outcome, SshError> {
    if std::env::var_os("SSH_AUTH_SOCK").is_some()
        && let Ok(Some(Outcome::Success)) = agent(handle, cx).await
    {
        return Ok(Outcome::Success);
    }
    let candidates: Vec<PathBuf> = match identity {
        Some(p) => vec![p.to_path_buf()],
        None => std::env::var_os("HOME")
            .map(|h| DEFAULT_IDENTITIES.iter().map(|n| PathBuf::from(&h).join(".ssh").join(n)).filter(|p| p.exists()).collect())
            .unwrap_or_default(),
    };
    for path in candidates {
        if let Ok(key) = load_key(cx, &path, None).await
            && let Outcome::Success = publickey(handle, cx, key).await?
        {
            return Ok(Outcome::Success);
        }
    }
    Ok(handle.authenticate_none(&cx.hop.user).await.map_err(cx.ssh())?.into())
}

/// Load a key; ask for the passphrase if it is encrypted and none was given.
async fn load_key(cx: &Ctx<'_>, path: &Path, passphrase: Option<Zeroizing<String>>) -> Result<PrivateKey, SshError> {
    let err = |e: russh::keys::Error| SshError::KeyFile { hop: cx.label.clone(), reason: e.to_string() };
    match load_secret_key(path, passphrase.as_ref().map(|p| p.as_str())) {
        Ok(k) => Ok(k),
        Err(russh::keys::Error::KeyIsEncrypted) if passphrase.is_none() && cx.prompter.is_some() => {
            let text = if crate::english() {
                format!("Passphrase for key {}", path.display())
            } else {
                format!("{} 키 암호", path.display())
            };
            let pw = cx.ask("", "", vec![InteractivePrompt { text, echo: false }], false).await?;
            load_secret_key(path, Some(pw[0].as_str())).map_err(err)
        }
        Err(e) => Err(err(e)),
    }
}

async fn publickey(handle: &mut Handle, cx: &Ctx<'_>, key: PrivateKey) -> Result<Outcome, SshError> {
    let hash = handle.best_supported_rsa_hash().await.map_err(cx.ssh())?.flatten();
    Ok(handle
        .authenticate_publickey(&cx.hop.user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
        .await
        .map_err(cx.ssh())?
        .into())
}

/// Try every agent identity. `None` when the agent had no usable identity.
async fn agent(handle: &mut Handle, cx: &Ctx<'_>) -> Result<Option<Outcome>, SshError> {
    let agent_err = |e: russh::keys::Error| SshError::Agent { hop: cx.label.clone(), reason: e.to_string() };
    let mut agent = AgentClient::connect_env().await.map_err(agent_err)?;
    let identities = agent.request_identities().await.map_err(agent_err)?;
    let rsa_hash = handle.best_supported_rsa_hash().await.map_err(cx.ssh())?.flatten();
    let mut last = None;
    for id in identities {
        let key = id.public_key().into_owned();
        let hash = if key.algorithm().is_rsa() { rsa_hash } else { None };
        match handle.authenticate_publickey_with(&cx.hop.user, key, hash, &mut agent).await {
            Ok(r) => {
                let o: Outcome = r.into();
                if matches!(o, Outcome::Success) {
                    return Ok(Some(o));
                }
                last = Some(o);
            }
            Err(_) => continue,
        }
    }
    Ok(last)
}

/// A lone hidden prompt asking for the login password (OpenSSH/PAM:
/// "Password:", "(user@host) Password:"), not an OTP.
fn is_password_prompt(prompts: &[InteractivePrompt]) -> bool {
    let [p] = prompts else { return false };
    let t = p.text.to_lowercase();
    !p.echo && (t.contains("password") || t.contains("비밀번호")) && !t.contains("one-time") && !t.contains("otp")
}

/// `saved` answers the first password prompt (once) instead of the user.
async fn keyboard_interactive(
    handle: &mut Handle,
    cx: &Ctx<'_>,
    saved: &mut Option<Zeroizing<String>>,
) -> Result<Outcome, SshError> {
    let mut resp = handle.authenticate_keyboard_interactive_start(&cx.hop.user, None).await.map_err(cx.ssh())?;
    for _ in 0..MAX_INFO_REQUESTS {
        match resp {
            KeyboardInteractiveAuthResponse::Success => return Ok(Outcome::Success),
            KeyboardInteractiveAuthResponse::Failure { remaining_methods, partial_success } => {
                return Ok(Outcome::Failure { remaining: remaining_methods.to_vec(), partial: partial_success });
            }
            KeyboardInteractiveAuthResponse::InfoRequest { name, instructions, prompts } => {
                let ps: Vec<InteractivePrompt> =
                    prompts.into_iter().map(|p| InteractivePrompt { text: p.prompt, echo: p.echo }).collect();
                let password = is_password_prompt(&ps);
                let answers = if ps.is_empty() {
                    Vec::new()
                } else if password && let Some(pw) = saved.take() {
                    vec![pw.as_str().to_string()]
                } else {
                    cx.ask(&name, &instructions, ps, password).await?.iter().map(|z| z.as_str().to_string()).collect()
                };
                resp = handle.authenticate_keyboard_interactive_respond(answers).await.map_err(cx.ssh())?;
            }
        }
    }
    Err(SshError::AuthFailed(cx.label.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hidden(text: &str) -> Vec<InteractivePrompt> {
        vec![InteractivePrompt { text: text.into(), echo: false }]
    }

    #[test]
    fn password_prompt_detection() {
        assert!(is_password_prompt(&hidden("Password: ")));
        assert!(is_password_prompt(&hidden("(alice@10.0.0.5) Password:")));
        assert!(is_password_prompt(&hidden("비밀번호: ")));
        assert!(!is_password_prompt(&hidden("Verification code: ")));
        assert!(!is_password_prompt(&hidden("One-time password (OATH) for `alice':")));
        assert!(!is_password_prompt(&hidden("OTP password:")));
        assert!(!is_password_prompt(&[InteractivePrompt { text: "Password: ".into(), echo: true }]));
        let two = [hidden("Password: "), hidden("Verification code: ")].concat();
        assert!(!is_password_prompt(&two));
    }
}
