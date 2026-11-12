//! Bounded segment-aligned fragmented-MP4 stream-copy playback.

use std::{
    error::Error,
    fmt,
    num::NonZeroU32,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use surveillance_storage::metadata::CompletedRecording;
use surveillance_storage::recordings::PlaybackPlan;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{Child, ChildStdout, Command},
};

const PREFETCH_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug)]
pub struct PlaybackAssembler {
    root: PathBuf,
    ffmpeg: PathBuf,
    startup_timeout: Duration,
}

/// The requested extent of one completed recording.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordingVideoExtent {
    Full,
    Leading(ClipDuration),
}

/// A bounded, positive leading-clip duration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClipDuration(NonZeroU32);

impl ClipDuration {
    pub const MAX_SECONDS: u32 = 60;

    /// Validates a duration accepted by the public recording-video API.
    ///
    /// # Errors
    /// Returns an error for zero or values above the operational bound.
    pub fn new(seconds: u32) -> Result<Self, ClipDurationError> {
        let seconds = NonZeroU32::new(seconds).ok_or(ClipDurationError)?;
        if seconds.get() > Self::MAX_SECONDS {
            return Err(ClipDurationError);
        }
        Ok(Self(seconds))
    }

    #[must_use]
    pub const fn seconds(self) -> u32 {
        self.0.get()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClipDurationError;

impl PlaybackAssembler {
    #[must_use]
    pub const fn new(root: PathBuf, ffmpeg: PathBuf, startup_timeout: Duration) -> Self {
        Self {
            root,
            ffmpeg,
            startup_timeout,
        }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Starts one owned stream-copy child after validating every planned file.
    /// The emitted interval is segment-aligned, not frame-accurate.
    ///
    /// # Errors
    /// Returns a classified path, process, or startup failure.
    pub async fn start(&self, plan: &PlaybackPlan) -> Result<PlaybackStream, PlaybackError> {
        let canonical_root = tokio::fs::canonicalize(&self.root)
            .await
            .map_err(PlaybackError::Io)?;
        let mut manifest = String::from("ffconcat version 1.0\n");
        for segment in plan.segments() {
            let path = self.root.join(segment.locator().as_path());
            let metadata = tokio::fs::symlink_metadata(&path)
                .await
                .map_err(PlaybackError::Io)?;
            if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
                return Err(PlaybackError::UnsafePath);
            }
            let canonical = tokio::fs::canonicalize(&path)
                .await
                .map_err(PlaybackError::Io)?;
            if !canonical.starts_with(&canonical_root) {
                return Err(PlaybackError::UnsafePath);
            }
            let Some(path) = canonical.to_str() else {
                return Err(PlaybackError::UnsafePath);
            };
            manifest.push_str("file 'file:");
            manifest.push_str(&path.replace('\'', "'\\''"));
            manifest.push_str("'\n");
        }
        let mut child = Command::new(&self.ffmpeg)
            .args([
                "-v",
                "error",
                "-f",
                "concat",
                "-safe",
                "0",
                "-protocol_whitelist",
                "file,pipe",
                "-i",
                "pipe:0",
                "-map",
                "0:v:0",
                "-c",
                "copy",
                "-movflags",
                "+frag_keyframe+empty_moov+default_base_moof",
                "-f",
                "mp4",
                "pipe:1",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(PlaybackError::Io)?;
        let mut stdin = child.stdin.take().ok_or(PlaybackError::MissingPipe)?;
        tokio::time::timeout(self.startup_timeout, stdin.write_all(manifest.as_bytes()))
            .await
            .map_err(|_| PlaybackError::StartupTimeout)?
            .map_err(PlaybackError::Io)?;
        drop(stdin);
        let stdout = child.stdout.take().ok_or(PlaybackError::MissingPipe)?;
        Self::with_media_ready(child, stdout, self.startup_timeout).await
    }

    /// Starts one owned stream-copy child for a completed recording.
    ///
    /// A leading duration is packet/keyframe bounded rather than frame-exact;
    /// no decode or re-encode is introduced.
    ///
    /// # Errors
    /// Returns a classified path, process, or startup failure.
    pub async fn start_recording(
        &self,
        recording: &CompletedRecording,
        extent: RecordingVideoExtent,
    ) -> Result<PlaybackStream, PlaybackError> {
        let input = self
            .validated_recording_path(recording.locator().as_path())
            .await?;
        let mut command = Command::new(&self.ffmpeg);
        command.args(["-v", "error", "-i"]).arg(input);
        if let RecordingVideoExtent::Leading(duration) = extent {
            command.args(["-t", &duration.seconds().to_string()]);
        }
        let mut child = command
            .args([
                "-map",
                "0:v:0",
                "-c",
                "copy",
                "-movflags",
                "+frag_keyframe+empty_moov+default_base_moof",
                "-f",
                "mp4",
                "pipe:1",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(PlaybackError::Io)?;
        let stdout = child.stdout.take().ok_or(PlaybackError::MissingPipe)?;
        Self::with_media_ready(child, stdout, self.startup_timeout).await
    }

    /// Returns a stream only once the child has actually produced media.
    ///
    /// A successful spawn is not readiness. Without this, an unreadable or
    /// corrupt recording was discovered only after `200 OK` and the response
    /// headers had been written, so the handler could do nothing but truncate
    /// the body -- while `docs/API.md` and the plan both document a
    /// `409 recording_media_unavailable` for exactly that case. Reading the
    /// first chunk under the startup timeout makes that status reachable.
    ///
    /// The bytes are kept and replayed, so prefetching costs nothing.
    async fn with_media_ready(
        mut child: Child,
        mut stdout: ChildStdout,
        startup_timeout: Duration,
    ) -> Result<PlaybackStream, PlaybackError> {
        let mut prefetched = vec![0_u8; PREFETCH_BYTES];
        let outcome = tokio::time::timeout(startup_timeout, stdout.read(&mut prefetched)).await;
        let read = match outcome {
            Ok(Ok(read)) => read,
            Ok(Err(error)) => {
                let _ = child.kill().await;
                return Err(PlaybackError::Io(error));
            }
            Err(_) => {
                let _ = child.kill().await;
                return Err(PlaybackError::StartupTimeout);
            }
        };
        if read == 0 {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(PlaybackError::ProcessFailed);
        }
        prefetched.truncate(read);
        Ok(PlaybackStream {
            child,
            stdout,
            prefetched,
        })
    }

    async fn validated_recording_path(&self, relative: &Path) -> Result<PathBuf, PlaybackError> {
        let canonical_root = tokio::fs::canonicalize(&self.root)
            .await
            .map_err(PlaybackError::Io)?;
        let path = self.root.join(relative);
        let metadata = tokio::fs::symlink_metadata(&path)
            .await
            .map_err(PlaybackError::Io)?;
        if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
            return Err(PlaybackError::UnsafePath);
        }
        let canonical = tokio::fs::canonicalize(path)
            .await
            .map_err(PlaybackError::Io)?;
        if !canonical.starts_with(canonical_root) {
            return Err(PlaybackError::UnsafePath);
        }
        Ok(canonical)
    }
}

pub struct PlaybackStream {
    child: Child,
    stdout: ChildStdout,
    prefetched: Vec<u8>,
}

impl PlaybackStream {
    /// Reads assembled media, replaying the readiness prefetch first.
    ///
    /// This replaces a raw accessor for the child's stdout: a caller holding
    /// the pipe directly would skip the prefetched bytes and silently drop the
    /// start of every response, including the fragmented-MP4 header.
    ///
    /// # Errors
    /// Returns the underlying pipe failure.
    pub async fn read(&mut self, buffer: &mut [u8]) -> Result<usize, std::io::Error> {
        if self.prefetched.is_empty() {
            return self.stdout.read(buffer).await;
        }
        let taken = buffer.len().min(self.prefetched.len());
        buffer[..taken].copy_from_slice(&self.prefetched[..taken]);
        self.prefetched.drain(..taken);
        Ok(taken)
    }

    /// Waits for the owned assembler child after its output is drained.
    ///
    /// # Errors
    /// Returns a process I/O or nonzero-exit failure.
    pub async fn finish(mut self) -> Result<(), PlaybackError> {
        let status = self.child.wait().await.map_err(PlaybackError::Io)?;
        if status.success() {
            Ok(())
        } else {
            Err(PlaybackError::ProcessFailed)
        }
    }

    pub async fn cancel(mut self) {
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
    }
}

#[derive(Debug)]
pub enum PlaybackError {
    Io(std::io::Error),
    UnsafePath,
    MissingPipe,
    StartupTimeout,
    ProcessFailed,
}

impl fmt::Display for PlaybackError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("segment-aligned playback assembly failed")
    }
}
impl Error for PlaybackError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}
