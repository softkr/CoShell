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

    /// Start `cmd` in a fresh non-interactive session on the target and hand back what it
    /// prints as it prints it. The command runs until it ends or the stream is dropped
    /// (which closes its channel: the server takes its output away and it stops).
    pub async fn exec_stream(&self, cmd: &str) -> Result<ExecStream, SshError> {
        let label = self.labels().last().cloned().unwrap_or_default();
        let ssh = |source| SshError::Ssh { hop: label.clone(), source };
        let channel = self.target().channel_open_session().await.map_err(ssh)?;
        channel.exec(true, cmd).await.map_err(ssh)?;
        Ok(ExecStream { channel })
    }
}

/// The output of a running command (see [`Connection::exec_stream`]).
pub struct ExecStream {
    channel: russh::Channel<russh::client::Msg>,
}

impl ExecStream {
    /// The next piece of output (stdout only); `None` once the command has ended or the
    /// connection is gone.
    pub async fn next(&mut self) -> Option<Vec<u8>> {
        loop {
            match self.channel.wait().await? {
                ChannelMsg::Data { data } => return Some(data.to_vec()),
                ChannelMsg::Failure | ChannelMsg::Close | ChannelMsg::ExitStatus { .. } => return None,
                _ => {}
            }
        }
    }
}
