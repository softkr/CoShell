//! Run one command on the target without a terminal and collect its output
//! (used by the AI assistant to read results; never touches the user's shell).

use russh::ChannelMsg;

use crate::{Connection, SshError};

/// What a command printed (stdout and stderr, in arrival order).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecOutput {
    pub output: Vec<u8>,
    /// `None` when the server did not report one (e.g. killed by a signal).
    pub exit: Option<u32>,
    /// More than the limit was printed; the rest was dropped.
    pub truncated: bool,
}

impl Connection {
    /// Run `cmd` in a fresh non-interactive session on the target; keep at most `limit` bytes.
    /// The caller bounds the time (dropping the future closes the channel).
    pub async fn exec(&self, cmd: &str, limit: usize) -> Result<ExecOutput, SshError> {
        let label = self.labels().last().cloned().unwrap_or_default();
        let ssh = |source| SshError::Ssh { hop: label.clone(), source };
        let mut channel = self.target().channel_open_session().await.map_err(ssh)?;
        channel.exec(true, cmd).await.map_err(ssh)?;
        let mut out = ExecOutput { output: Vec::new(), exit: None, truncated: false };
        while let Some(msg) = channel.wait().await {
            match msg {
                ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => {
                    let room = limit.saturating_sub(out.output.len());
                    out.truncated |= data.len() > room;
                    out.output.extend_from_slice(&data[..data.len().min(room)]);
                }
                ChannelMsg::ExitStatus { exit_status } => out.exit = Some(exit_status),
                ChannelMsg::Failure => {
                    out.output.extend_from_slice(b"[CoShell] the server refused to run the command");
                    break;
                }
                ChannelMsg::Close => break,
                _ => {}
            }
        }
        Ok(out)
    }
}
