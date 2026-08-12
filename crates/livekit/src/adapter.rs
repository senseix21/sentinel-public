use std::{error::Error, fmt, future::Future, path::PathBuf, pin::Pin, process::Stdio};

use tokio::{
    io::AsyncWriteExt,
    net::{UnixListener, UnixStream},
    process::{Child, Command},
};

use crate::config::LiveKitConfig;

pub type AdapterFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, AdapterError>> + Send + 'a>>;

pub trait PublicationAdapter: Send + 'static {
    fn connect(&mut self) -> AdapterFuture<'_, ()>;
    fn send_access_unit<'a>(&'a mut self, annex_b: &'a [u8]) -> AdapterFuture<'a, ()>;
    fn disconnect(&mut self) -> AdapterFuture<'_, ()>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdapterErrorKind {
    Unavailable,
    VersionMismatch,
    Authentication,
    Connection,
    Transport,
    ChildExited,
    Shutdown,
}

#[derive(Debug)]
pub struct AdapterError {
    kind: AdapterErrorKind,
}

impl AdapterError {
    #[must_use]
    pub const fn new(kind: AdapterErrorKind) -> Self {
        Self { kind }
    }
    #[must_use]
    pub const fn kind(&self) -> AdapterErrorKind {
        self.kind
    }
    #[must_use]
    pub const fn retryable(&self) -> bool {
        matches!(
            self.kind,
            AdapterErrorKind::Connection
                | AdapterErrorKind::Transport
                | AdapterErrorKind::ChildExited
        )
    }
}

impl fmt::Display for AdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "LiveKit adapter failure: {:?}", self.kind)
    }
}
impl Error for AdapterError {}

pub struct CliPublicationAdapter {
    config: LiveKitConfig,
    child: Option<Child>,
    stream: Option<UnixStream>,
    socket_path: Option<PathBuf>,
}

impl CliPublicationAdapter {
    #[must_use]
    pub fn new(config: LiveKitConfig) -> Self {
        Self {
            config,
            child: None,
            stream: None,
            socket_path: None,
        }
    }

    fn fresh_socket_path() -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        // Unix socket paths are commonly limited to roughly 100 bytes. Keep the
        // IPC address independent of long platform temp-directory expansions;
        // logical identity remains in the LiveKit participant/track contract.
        PathBuf::from("/tmp")
            .join(format!("slk-{nonce}"))
            .join("video.h264")
    }
}

impl PublicationAdapter for CliPublicationAdapter {
    fn connect(&mut self) -> AdapterFuture<'_, ()> {
        Box::pin(async move {
            self.disconnect().await?;
            let version = Command::new(&self.config.cli_path)
                .arg("--version")
                .output()
                .await
                .map_err(|_| AdapterError::new(AdapterErrorKind::Unavailable))?;
            let version = String::from_utf8_lossy(&version.stdout);
            if !version.contains(&self.config.cli_version) {
                return Err(AdapterError::new(AdapterErrorKind::VersionMismatch));
            }
            let socket_path = Self::fresh_socket_path();
            let socket_directory = socket_path
                .parent()
                .ok_or_else(|| AdapterError::new(AdapterErrorKind::Connection))?;
            tokio::fs::create_dir(socket_directory)
                .await
                .map_err(|_| AdapterError::new(AdapterErrorKind::Connection))?;
            let listener = UnixListener::bind(&socket_path)
                .map_err(|_| AdapterError::new(AdapterErrorKind::Connection))?;
            let publish = format!("h264://{}", socket_path.display());
            let mut command = Command::new(&self.config.cli_path);
            command
                .arg("room")
                .arg("join")
                .arg("--identity")
                .arg(self.config.identity.participant.as_str())
                .arg("--publish")
                .arg(publish)
                .arg(self.config.identity.room.as_str())
                .env("LIVEKIT_URL", self.config.endpoint.as_str())
                .env("LIVEKIT_API_KEY", self.config.credentials.api_key())
                .env("LIVEKIT_API_SECRET", self.config.credentials.api_secret())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true);
            let child = command
                .spawn()
                .map_err(|_| AdapterError::new(AdapterErrorKind::Unavailable))?;
            self.child = Some(child);
            self.socket_path = Some(socket_path);
            let accepted = tokio::time::timeout(
                self.config.ipc_write_timeout.saturating_mul(20),
                listener.accept(),
            )
            .await
            .map_err(|_| AdapterError::new(AdapterErrorKind::Connection))?
            .map_err(|_| AdapterError::new(AdapterErrorKind::Connection))?;
            self.stream = Some(accepted.0);
            Ok(())
        })
    }

    fn send_access_unit<'a>(&'a mut self, annex_b: &'a [u8]) -> AdapterFuture<'a, ()> {
        Box::pin(async move {
            if self
                .child
                .as_mut()
                .is_some_and(|child| child.try_wait().ok().flatten().is_some())
            {
                return Err(AdapterError::new(AdapterErrorKind::ChildExited));
            }
            let stream = self
                .stream
                .as_mut()
                .ok_or_else(|| AdapterError::new(AdapterErrorKind::Connection))?;
            tokio::time::timeout(self.config.ipc_write_timeout, stream.write_all(annex_b))
                .await
                .map_err(|_| AdapterError::new(AdapterErrorKind::Transport))?
                .map_err(|_| AdapterError::new(AdapterErrorKind::Transport))
        })
    }

    fn disconnect(&mut self) -> AdapterFuture<'_, ()> {
        Box::pin(async move {
            self.stream.take();
            if let Some(mut child) = self.child.take() {
                let _ = child.start_kill();
                let waited = tokio::time::timeout(self.config.shutdown_timeout, child.wait()).await;
                if waited.is_err() {
                    return Err(AdapterError::new(AdapterErrorKind::Shutdown));
                }
            }
            if let Some(path) = self.socket_path.take() {
                let parent = path.parent().map(std::path::Path::to_path_buf);
                match tokio::fs::remove_file(path).await {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => return Err(AdapterError::new(AdapterErrorKind::Shutdown)),
                }
                if let Some(parent) = parent {
                    let _ = tokio::fs::remove_dir(parent).await;
                }
            }
            Ok(())
        })
    }
}

impl Drop for CliPublicationAdapter {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.start_kill();
        }
        if let Some(path) = self.socket_path.take() {
            let parent = path.parent().map(std::path::Path::to_path_buf);
            let _ = std::fs::remove_file(path);
            if let Some(parent) = parent {
                let _ = std::fs::remove_dir(parent);
            }
        }
    }
}

pub struct MemoryAdapter {
    pub fail_connect: Option<AdapterErrorKind>,
    pub fail_send_after: Option<usize>,
    pub connected: bool,
    pub access_units: Vec<Vec<u8>>,
}

impl MemoryAdapter {
    #[must_use]
    pub fn healthy() -> Self {
        Self {
            fail_connect: None,
            fail_send_after: None,
            connected: false,
            access_units: Vec::new(),
        }
    }
}

impl PublicationAdapter for MemoryAdapter {
    fn connect(&mut self) -> AdapterFuture<'_, ()> {
        Box::pin(async move {
            if let Some(kind) = self.fail_connect {
                Err(AdapterError::new(kind))
            } else {
                self.connected = true;
                Ok(())
            }
        })
    }
    fn send_access_unit<'a>(&'a mut self, annex_b: &'a [u8]) -> AdapterFuture<'a, ()> {
        Box::pin(async move {
            if self
                .fail_send_after
                .is_some_and(|limit| self.access_units.len() >= limit)
            {
                Err(AdapterError::new(AdapterErrorKind::Transport))
            } else {
                self.access_units.push(annex_b.to_vec());
                Ok(())
            }
        })
    }
    fn disconnect(&mut self) -> AdapterFuture<'_, ()> {
        Box::pin(async move {
            self.connected = false;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn memory_adapter_has_explicit_connect_send_disconnect_lifetime() {
        let mut adapter = MemoryAdapter::healthy();
        adapter
            .connect()
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        adapter
            .send_access_unit(&[0, 0, 0, 1, 0x65])
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        adapter
            .disconnect()
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(adapter.access_units.len(), 1);
        assert!(!adapter.connected);
    }
}
