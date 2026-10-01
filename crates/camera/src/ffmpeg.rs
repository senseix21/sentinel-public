//! Managed `FFmpeg` H.264 source for local webcams and deterministic fixtures.

use std::{
    collections::VecDeque,
    error::Error,
    fmt, io,
    num::{NonZeroU32, NonZeroUsize},
    path::PathBuf,
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use surveillance_media::packet::{
    AccessUnitError, H264AccessUnit, H264AccessUnitAssembler, H264NalError, H264NalKind,
    H264NalUnit, MediaTimestamp, TimestampError,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::{Child, ChildStdout, Command},
    task::JoinHandle,
};

use crate::connection::{RetryableCameraFailure, TerminalCameraFailure};

const READ_CHUNK_BYTES: usize = 16 * 1024;
const H264_TIMEBASE_HZ: u32 = 90_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FfmpegSourceKind {
    Webcam,
    Synthetic,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FfmpegCaptureBackend {
    AvFoundation,
    Video4Linux2,
    DirectShow,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WebcamInput {
    backend: FfmpegCaptureBackend,
    device: String,
}

impl WebcamInput {
    /// Validates one operating-system capture device selector.
    ///
    /// # Errors
    /// Returns an error when the selector is empty or contains a NUL byte.
    pub fn new(
        backend: FfmpegCaptureBackend,
        device: String,
    ) -> Result<Self, FfmpegSourceConfigError> {
        if device.trim().is_empty() {
            return Err(FfmpegSourceConfigError::EmptyDevice);
        }
        if device.contains('\0') {
            return Err(FfmpegSourceConfigError::DeviceContainsNul);
        }
        Ok(Self { backend, device })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum FfmpegInput {
    Webcam(WebcamInput),
    Synthetic,
}

/// Validated source settings. `FFmpeg` arguments are constructed by trusted code;
/// callers cannot inject an arbitrary argument vector or shell expression.
#[derive(Clone, Eq, PartialEq)]
pub struct FfmpegSourceConfig {
    executable: PathBuf,
    input: FfmpegInput,
    width: NonZeroU32,
    height: NonZeroU32,
    frames_per_second: NonZeroU32,
    bitrate_kbps: NonZeroU32,
    keyframe_interval: NonZeroU32,
    startup_timeout: Duration,
    read_timeout: Duration,
    max_access_unit_bytes: NonZeroUsize,
    max_nals_per_access_unit: NonZeroUsize,
    max_parser_bytes: NonZeroUsize,
    bounded_memory_bytes: usize,
}

impl FfmpegSourceConfig {
    /// Validates a webcam capture and its complete process and memory bounds.
    ///
    /// # Errors
    /// Returns the specific violated source invariant.
    #[allow(
        clippy::too_many_arguments,
        reason = "constructor validates one bounded source contract"
    )]
    pub fn webcam(
        executable: PathBuf,
        input: WebcamInput,
        width: u32,
        height: u32,
        frames_per_second: u32,
        bitrate_kbps: u32,
        keyframe_interval: u32,
        startup_timeout: Duration,
        read_timeout: Duration,
        max_access_unit_bytes: usize,
        max_nals_per_access_unit: usize,
        max_parser_bytes: usize,
    ) -> Result<Self, FfmpegSourceConfigError> {
        Self::new(
            executable,
            FfmpegInput::Webcam(input),
            width,
            height,
            frames_per_second,
            bitrate_kbps,
            keyframe_interval,
            startup_timeout,
            read_timeout,
            max_access_unit_bytes,
            max_nals_per_access_unit,
            max_parser_bytes,
        )
    }

    #[doc(hidden)]
    #[allow(
        clippy::too_many_arguments,
        reason = "test source mirrors the production bounds"
    )]
    pub fn synthetic(
        executable: PathBuf,
        width: u32,
        height: u32,
        frames_per_second: u32,
        bitrate_kbps: u32,
        keyframe_interval: u32,
        startup_timeout: Duration,
        read_timeout: Duration,
        max_access_unit_bytes: usize,
        max_nals_per_access_unit: usize,
        max_parser_bytes: usize,
    ) -> Result<Self, FfmpegSourceConfigError> {
        Self::new(
            executable,
            FfmpegInput::Synthetic,
            width,
            height,
            frames_per_second,
            bitrate_kbps,
            keyframe_interval,
            startup_timeout,
            read_timeout,
            max_access_unit_bytes,
            max_nals_per_access_unit,
            max_parser_bytes,
        )
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "all coupled resource limits are validated together"
    )]
    fn new(
        executable: PathBuf,
        input: FfmpegInput,
        width: u32,
        height: u32,
        frames_per_second: u32,
        bitrate_kbps: u32,
        keyframe_interval: u32,
        startup_timeout: Duration,
        read_timeout: Duration,
        max_access_unit_bytes: usize,
        max_nals_per_access_unit: usize,
        max_parser_bytes: usize,
    ) -> Result<Self, FfmpegSourceConfigError> {
        if executable.as_os_str().is_empty() {
            return Err(FfmpegSourceConfigError::EmptyExecutable);
        }
        let width = NonZeroU32::new(width).ok_or(FfmpegSourceConfigError::ZeroWidth)?;
        let height = NonZeroU32::new(height).ok_or(FfmpegSourceConfigError::ZeroHeight)?;
        let frames_per_second = NonZeroU32::new(frames_per_second)
            .ok_or(FfmpegSourceConfigError::ZeroFramesPerSecond)?;
        let bitrate_kbps =
            NonZeroU32::new(bitrate_kbps).ok_or(FfmpegSourceConfigError::ZeroBitrate)?;
        let keyframe_interval = NonZeroU32::new(keyframe_interval)
            .ok_or(FfmpegSourceConfigError::ZeroKeyframeInterval)?;
        if startup_timeout.is_zero() {
            return Err(FfmpegSourceConfigError::ZeroStartupTimeout);
        }
        if read_timeout.is_zero() {
            return Err(FfmpegSourceConfigError::ZeroReadTimeout);
        }
        let max_access_unit_bytes = NonZeroUsize::new(max_access_unit_bytes)
            .ok_or(FfmpegSourceConfigError::ZeroMaxAccessUnitBytes)?;
        let max_nals_per_access_unit = NonZeroUsize::new(max_nals_per_access_unit)
            .ok_or(FfmpegSourceConfigError::ZeroMaxNals)?;
        let max_parser_bytes = NonZeroUsize::new(max_parser_bytes)
            .ok_or(FfmpegSourceConfigError::ZeroMaxParserBytes)?;
        let minimum_parser_bytes = max_access_unit_bytes
            .get()
            .checked_add(8)
            .ok_or(FfmpegSourceConfigError::ParserBoundOverflow)?;
        if max_parser_bytes.get() < minimum_parser_bytes {
            return Err(FfmpegSourceConfigError::ParserBoundTooSmall {
                minimum: minimum_parser_bytes,
                configured: max_parser_bytes.get(),
            });
        }
        let bounded_memory_bytes = max_parser_bytes
            .get()
            .checked_add(max_access_unit_bytes.get())
            .and_then(|bytes| bytes.checked_add(4 * 1024))
            .ok_or(FfmpegSourceConfigError::ParserBoundOverflow)?;
        Ok(Self {
            executable,
            input,
            width,
            height,
            frames_per_second,
            bitrate_kbps,
            keyframe_interval,
            startup_timeout,
            read_timeout,
            max_access_unit_bytes,
            max_nals_per_access_unit,
            max_parser_bytes,
            bounded_memory_bytes,
        })
    }

    #[must_use]
    pub const fn kind(&self) -> FfmpegSourceKind {
        match self.input {
            FfmpegInput::Webcam(_) => FfmpegSourceKind::Webcam,
            FfmpegInput::Synthetic => FfmpegSourceKind::Synthetic,
        }
    }

    #[must_use]
    pub const fn parser_capacity_bytes(&self) -> usize {
        self.max_parser_bytes.get()
    }

    #[must_use]
    pub const fn bounded_memory_bytes(&self) -> usize {
        self.bounded_memory_bytes
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.executable);
        command
            .arg("-hide_banner")
            .arg("-nostdin")
            .arg("-loglevel")
            .arg("warning");
        match &self.input {
            FfmpegInput::Synthetic => {
                command
                    .arg("-re")
                    .arg("-f")
                    .arg("lavfi")
                    .arg("-i")
                    .arg(format!(
                        "testsrc2=size={}x{}:rate={}",
                        self.width, self.height, self.frames_per_second
                    ));
            }
            FfmpegInput::Webcam(input) => add_webcam_input(&mut command, input, self),
        }
        let bitrate = format!("{}k", self.bitrate_kbps);
        let keyframe_interval = self.keyframe_interval.to_string();
        command
            .arg("-an")
            .arg("-vf")
            .arg("format=yuv420p")
            .arg("-r")
            .arg(self.frames_per_second.to_string())
            .arg("-c:v")
            .arg("libx264")
            .arg("-profile:v")
            .arg("baseline")
            .arg("-preset")
            .arg("veryfast")
            .arg("-tune")
            .arg("zerolatency")
            .arg("-b:v")
            .arg(&bitrate)
            .arg("-maxrate")
            .arg(&bitrate)
            .arg("-bufsize")
            .arg(&bitrate)
            .arg("-g")
            .arg(&keyframe_interval)
            .arg("-keyint_min")
            .arg(&keyframe_interval)
            .arg("-sc_threshold")
            .arg("0")
            .arg("-bf")
            .arg("0")
            .arg("-x264-params")
            .arg("repeat-headers=1:aud=1")
            .arg("-f")
            .arg("h264")
            .arg("pipe:1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }
}

impl fmt::Debug for FfmpegSourceConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FfmpegSourceConfig")
            .field("kind", &self.kind())
            .field("executable", &self.executable)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("frames_per_second", &self.frames_per_second)
            .field("bitrate_kbps", &self.bitrate_kbps)
            .field("keyframe_interval", &self.keyframe_interval)
            .field("startup_timeout", &self.startup_timeout)
            .field("read_timeout", &self.read_timeout)
            .field("max_access_unit_bytes", &self.max_access_unit_bytes)
            .field("max_nals_per_access_unit", &self.max_nals_per_access_unit)
            .field("max_parser_bytes", &self.max_parser_bytes)
            .field("bounded_memory_bytes", &self.bounded_memory_bytes)
            .finish_non_exhaustive()
    }
}

fn add_webcam_input(command: &mut Command, input: &WebcamInput, config: &FfmpegSourceConfig) {
    match input.backend {
        FfmpegCaptureBackend::AvFoundation => {
            command
                .arg("-f")
                .arg("avfoundation")
                .arg("-framerate")
                .arg(config.frames_per_second.to_string())
                .arg("-video_size")
                .arg(format!("{}x{}", config.width, config.height))
                .arg("-i")
                .arg(format!("{}:none", input.device));
        }
        FfmpegCaptureBackend::Video4Linux2 => {
            command
                .arg("-f")
                .arg("v4l2")
                .arg("-framerate")
                .arg(config.frames_per_second.to_string())
                .arg("-video_size")
                .arg(format!("{}x{}", config.width, config.height))
                .arg("-i")
                .arg(&input.device);
        }
        FfmpegCaptureBackend::DirectShow => {
            command
                .arg("-f")
                .arg("dshow")
                .arg("-framerate")
                .arg(config.frames_per_second.to_string())
                .arg("-video_size")
                .arg(format!("{}x{}", config.width, config.height))
                .arg("-i")
                .arg(format!("video={}", input.device));
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FfmpegSourceConfigError {
    EmptyExecutable,
    EmptyDevice,
    DeviceContainsNul,
    ZeroWidth,
    ZeroHeight,
    ZeroFramesPerSecond,
    ZeroBitrate,
    ZeroKeyframeInterval,
    ZeroStartupTimeout,
    ZeroReadTimeout,
    ZeroMaxAccessUnitBytes,
    ZeroMaxNals,
    ZeroMaxParserBytes,
    ParserBoundOverflow,
    ParserBoundTooSmall { minimum: usize, configured: usize },
}

impl fmt::Display for FfmpegSourceConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid FFmpeg H.264 source configuration: {self:?}"
        )
    }
}

impl Error for FfmpegSourceConfigError {}

pub struct FfmpegH264Session {
    child: Child,
    reader: AnnexBAccessUnitReader<ChildStdout>,
    stderr_drain: JoinHandle<Result<u64, io::Error>>,
    prefetched: Option<Arc<H264AccessUnit>>,
    read_timeout: Duration,
}

impl FfmpegH264Session {
    /// Starts the owned process and waits for a parameterized keyframe.
    ///
    /// # Errors
    /// Returns a typed spawn, readiness, timeout, or media-contract failure.
    pub async fn start(config: &FfmpegSourceConfig) -> Result<Self, FfmpegConnectError> {
        let mut child = config
            .command()
            .spawn()
            .map_err(FfmpegConnectError::Spawn)?;
        let stdout = child
            .stdout
            .take()
            .ok_or(FfmpegConnectError::MissingStdout)?;
        let stderr = child
            .stderr
            .take()
            .ok_or(FfmpegConnectError::MissingStderr)?;
        let stderr_drain = tokio::spawn(drain(stderr));
        let mut reader = AnnexBAccessUnitReader::new(
            stdout,
            config.frames_per_second,
            config.max_access_unit_bytes,
            config.max_nals_per_access_unit,
            config.max_parser_bytes,
        );
        let prefetched = match tokio::time::timeout(config.startup_timeout, reader.next()).await {
            Ok(Ok(Some(access_unit))) => access_unit,
            Ok(Ok(None)) => {
                terminate_child(&mut child).await;
                stderr_drain.abort();
                let _ = stderr_drain.await;
                return Err(FfmpegConnectError::EndedBeforeReadiness);
            }
            Ok(Err(error)) => {
                terminate_child(&mut child).await;
                stderr_drain.abort();
                let _ = stderr_drain.await;
                return Err(FfmpegConnectError::Readiness(error));
            }
            Err(_) => {
                terminate_child(&mut child).await;
                stderr_drain.abort();
                let _ = stderr_drain.await;
                return Err(FfmpegConnectError::StartupTimeout);
            }
        };
        if !prefetched.is_keyframe() || !prefetched.has_parameter_sets() {
            terminate_child(&mut child).await;
            stderr_drain.abort();
            let _ = stderr_drain.await;
            return Err(FfmpegConnectError::MissingDecodableStart);
        }
        Ok(Self {
            child,
            reader,
            stderr_drain,
            prefetched: Some(Arc::new(prefetched)),
            read_timeout: config.read_timeout,
        })
    }

    /// Reads the next complete access unit, or `None` after clean process EOF.
    ///
    /// # Errors
    /// Returns a bounded parser, I/O, timeout, or invalid-media failure.
    pub async fn next(&mut self) -> Result<Option<Arc<H264AccessUnit>>, FfmpegReadError> {
        if let Some(access_unit) = self.prefetched.take() {
            return Ok(Some(access_unit));
        }
        match tokio::time::timeout(self.read_timeout, self.reader.next()).await {
            Ok(result) => result.map(|unit| unit.map(Arc::new)),
            Err(_) => Err(FfmpegReadError::Timeout),
        }
    }

    /// Terminates the child and joins its stderr drain within `timeout`.
    ///
    /// # Errors
    /// Returns an I/O, child-wait, or join-timeout failure.
    pub async fn shutdown(mut self, timeout: Duration) -> Result<(), io::Error> {
        if self.child.try_wait()?.is_none() {
            self.child.start_kill()?;
        }
        match tokio::time::timeout(timeout, self.child.wait()).await {
            Ok(result) => {
                let _ = result?;
            }
            Err(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "FFmpeg wait timed out",
                ))
            }
        }
        match tokio::time::timeout(timeout, &mut self.stderr_drain).await {
            Ok(Ok(Ok(_))) => Ok(()),
            Ok(Ok(Err(error))) => Err(error),
            Ok(Err(error)) => Err(io::Error::other(error)),
            Err(_) => {
                self.stderr_drain.abort();
                Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "FFmpeg stderr drain join timed out",
                ))
            }
        }
    }
}

async fn terminate_child(child: &mut Child) {
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.kill().await;
    }
}

async fn drain(mut reader: impl AsyncRead + Unpin) -> Result<u64, io::Error> {
    let mut total = 0_u64;
    let mut buffer = [0_u8; 4 * 1024];
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            return Ok(total);
        }
        total = total.saturating_add(read as u64);
    }
}

#[derive(Debug)]
pub enum FfmpegConnectError {
    Spawn(io::Error),
    MissingStdout,
    MissingStderr,
    StartupTimeout,
    EndedBeforeReadiness,
    MissingDecodableStart,
    Readiness(FfmpegReadError),
}

impl FfmpegConnectError {
    #[must_use]
    pub fn terminal_failure(&self) -> Option<TerminalCameraFailure> {
        match self {
            Self::Spawn(error) if error.kind() == io::ErrorKind::NotFound => {
                Some(TerminalCameraFailure::InvalidConfiguration)
            }
            Self::MissingStdout
            | Self::MissingStderr
            | Self::MissingDecodableStart
            | Self::Readiness(FfmpegReadError::InvalidMedia(_)) => {
                Some(TerminalCameraFailure::UnsupportedMedia)
            }
            _ => None,
        }
    }

    #[must_use]
    pub const fn retryable_failure(&self) -> RetryableCameraFailure {
        match self {
            Self::StartupTimeout => RetryableCameraFailure::Timeout,
            Self::EndedBeforeReadiness => RetryableCameraFailure::EndOfStream,
            _ => RetryableCameraFailure::TransportProtocol,
        }
    }
}

impl fmt::Display for FfmpegConnectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(_) => formatter.write_str("FFmpeg source process could not be started"),
            Self::MissingStdout | Self::MissingStderr => {
                formatter.write_str("FFmpeg source process pipes were not available")
            }
            Self::StartupTimeout => formatter.write_str("FFmpeg source readiness timed out"),
            Self::EndedBeforeReadiness => {
                formatter.write_str("FFmpeg source ended before producing media")
            }
            Self::MissingDecodableStart => {
                formatter.write_str("FFmpeg source did not begin with parameterized keyframe")
            }
            Self::Readiness(error) => write!(formatter, "FFmpeg source media was invalid: {error}"),
        }
    }
}

impl Error for FfmpegConnectError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Spawn(error) => Some(error),
            Self::Readiness(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub enum FfmpegReadError {
    Io(io::Error),
    Timeout,
    ParserCapacityExceeded { capacity: usize },
    AccessUnitTooLarge { size: usize, limit: usize },
    TooManyNals { count: usize, limit: usize },
    InvalidMedia(FfmpegMediaError),
}

impl fmt::Display for FfmpegReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(_) => formatter.write_str("FFmpeg media pipe read failed"),
            Self::Timeout => formatter.write_str("FFmpeg media pipe stalled"),
            Self::ParserCapacityExceeded { capacity } => {
                write!(formatter, "FFmpeg parser exceeded {capacity} bytes")
            }
            Self::AccessUnitTooLarge { size, limit } => {
                write!(formatter, "FFmpeg access unit size {size} exceeds {limit}")
            }
            Self::TooManyNals { count, limit } => {
                write!(
                    formatter,
                    "FFmpeg access unit NAL count {count} exceeds {limit}"
                )
            }
            Self::InvalidMedia(error) => write!(formatter, "invalid FFmpeg H.264 media: {error}"),
        }
    }
}

impl Error for FfmpegReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::InvalidMedia(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub enum FfmpegMediaError {
    Nal(H264NalError),
    Timestamp(TimestampError),
    AccessUnit(AccessUnitError),
    MissingStartCode,
    TrailingBytes,
}

impl fmt::Display for FfmpegMediaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl Error for FfmpegMediaError {}

struct AnnexBAccessUnitReader<R> {
    reader: R,
    buffer: Vec<u8>,
    pending: VecDeque<H264NalUnit>,
    pending_bytes: usize,
    eof: bool,
    frame_index: i64,
    frames_per_second: NonZeroU32,
    max_access_unit_bytes: NonZeroUsize,
    max_nals_per_access_unit: NonZeroUsize,
    max_parser_bytes: NonZeroUsize,
}

impl<R: AsyncRead + Unpin> AnnexBAccessUnitReader<R> {
    fn new(
        reader: R,
        frames_per_second: NonZeroU32,
        max_access_unit_bytes: NonZeroUsize,
        max_nals_per_access_unit: NonZeroUsize,
        max_parser_bytes: NonZeroUsize,
    ) -> Self {
        Self {
            reader,
            buffer: Vec::with_capacity(READ_CHUNK_BYTES.min(max_parser_bytes.get())),
            pending: VecDeque::new(),
            pending_bytes: 0,
            eof: false,
            frame_index: 0,
            frames_per_second,
            max_access_unit_bytes,
            max_nals_per_access_unit,
            max_parser_bytes,
        }
    }

    async fn next(&mut self) -> Result<Option<H264AccessUnit>, FfmpegReadError> {
        loop {
            let Some(bytes) = self.next_nal().await? else {
                return self.finish_pending();
            };
            let nal = H264NalUnit::from_ebsp(Arc::from(bytes))
                .map_err(|error| FfmpegReadError::InvalidMedia(FfmpegMediaError::Nal(error)))?;
            if nal.kind() == H264NalKind::AccessUnitDelimiter && self.pending_has_vcl() {
                let completed = self.finish_pending()?;
                self.push_pending(nal)?;
                return Ok(completed);
            }
            self.push_pending(nal)?;
        }
    }

    async fn next_nal(&mut self) -> Result<Option<Vec<u8>>, FfmpegReadError> {
        loop {
            if let Some((first_at, first_len)) = find_start_code(&self.buffer, 0) {
                if first_at > 0 {
                    if self.buffer[..first_at].iter().any(|byte| *byte != 0) {
                        return Err(FfmpegReadError::InvalidMedia(
                            FfmpegMediaError::MissingStartCode,
                        ));
                    }
                    self.buffer.drain(..first_at);
                    continue;
                }
                if let Some((next_at, _)) = find_start_code(&self.buffer, first_len) {
                    let nal = self.buffer[first_len..next_at].to_vec();
                    self.buffer.drain(..next_at);
                    if nal.is_empty() {
                        continue;
                    }
                    return Ok(Some(nal));
                }
                if self.eof {
                    let nal = self.buffer[first_len..].to_vec();
                    self.buffer.clear();
                    return if nal.is_empty() {
                        Ok(None)
                    } else {
                        Ok(Some(nal))
                    };
                }
            } else if self.eof {
                return if self.buffer.iter().all(|byte| *byte == 0) {
                    self.buffer.clear();
                    Ok(None)
                } else {
                    Err(FfmpegReadError::InvalidMedia(
                        FfmpegMediaError::TrailingBytes,
                    ))
                };
            }

            if self.buffer.len() >= self.max_parser_bytes.get() {
                return Err(FfmpegReadError::ParserCapacityExceeded {
                    capacity: self.max_parser_bytes.get(),
                });
            }
            let remaining = self.max_parser_bytes.get() - self.buffer.len();
            let read_len = remaining.min(READ_CHUNK_BYTES);
            let start = self.buffer.len();
            self.buffer.resize(start + read_len, 0);
            let read = self
                .reader
                .read(&mut self.buffer[start..])
                .await
                .map_err(FfmpegReadError::Io)?;
            self.buffer.truncate(start + read);
            if read == 0 {
                self.eof = true;
            }
        }
    }

    fn push_pending(&mut self, nal: H264NalUnit) -> Result<(), FfmpegReadError> {
        let count = self.pending.len().saturating_add(1);
        if count > self.max_nals_per_access_unit.get() {
            return Err(FfmpegReadError::TooManyNals {
                count,
                limit: self.max_nals_per_access_unit.get(),
            });
        }
        let size = self.pending_bytes.checked_add(nal.bytes().len()).ok_or(
            FfmpegReadError::AccessUnitTooLarge {
                size: usize::MAX,
                limit: self.max_access_unit_bytes.get(),
            },
        )?;
        if size > self.max_access_unit_bytes.get() {
            return Err(FfmpegReadError::AccessUnitTooLarge {
                size,
                limit: self.max_access_unit_bytes.get(),
            });
        }
        self.pending_bytes = size;
        self.pending.push_back(nal);
        Ok(())
    }

    fn pending_has_vcl(&self) -> bool {
        self.pending.iter().any(H264NalUnit::is_vcl)
    }

    fn finish_pending(&mut self) -> Result<Option<H264AccessUnit>, FfmpegReadError> {
        if self.pending.is_empty() {
            return Ok(None);
        }
        if !self.pending_has_vcl() {
            self.pending.clear();
            self.pending_bytes = 0;
            return if self.eof {
                Err(FfmpegReadError::InvalidMedia(
                    FfmpegMediaError::TrailingBytes,
                ))
            } else {
                Ok(None)
            };
        }
        let ticks = self.frame_index.saturating_mul(i64::from(H264_TIMEBASE_HZ))
            / i64::from(self.frames_per_second.get());
        let timestamp = MediaTimestamp::new(ticks, H264_TIMEBASE_HZ)
            .map_err(|error| FfmpegReadError::InvalidMedia(FfmpegMediaError::Timestamp(error)))?;
        self.frame_index = self.frame_index.saturating_add(1);
        let count = self.pending.len();
        let mut assembler = H264AccessUnitAssembler::new();
        let mut completed = None;
        for (index, nal) in self.pending.drain(..).enumerate() {
            completed = assembler
                .push(timestamp, nal, index.saturating_add(1) == count)
                .map_err(|error| {
                    FfmpegReadError::InvalidMedia(FfmpegMediaError::AccessUnit(error))
                })?;
        }
        self.pending_bytes = 0;
        completed
            .ok_or(FfmpegReadError::InvalidMedia(
                FfmpegMediaError::TrailingBytes,
            ))
            .map(Some)
    }
}

fn find_start_code(bytes: &[u8], from: usize) -> Option<(usize, usize)> {
    let mut index = from;
    while index.saturating_add(3) <= bytes.len() {
        if bytes.get(index..index + 3) == Some(&[0, 0, 1]) {
            return Some((index, 3));
        }
        if index.saturating_add(4) <= bytes.len()
            && bytes.get(index..index + 4) == Some(&[0, 0, 0, 1])
        {
            return Some((index, 4));
        }
        index = index.saturating_add(1);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{
        find_start_code, AnnexBAccessUnitReader, FfmpegCaptureBackend, FfmpegSourceConfig,
        FfmpegSourceConfigError, WebcamInput,
    };
    use std::{
        num::{NonZeroU32, NonZeroUsize},
        path::PathBuf,
        time::Duration,
    };
    use tokio::io::{AsyncWriteExt, DuplexStream};

    fn source() -> FfmpegSourceConfig {
        let max_access_unit_bytes = 2 * 1024 * 1024;
        FfmpegSourceConfig::synthetic(
            PathBuf::from("ffmpeg"),
            640,
            360,
            15,
            500,
            15,
            Duration::from_secs(2),
            Duration::from_secs(2),
            max_access_unit_bytes,
            128,
            max_access_unit_bytes + 8,
        )
        .unwrap_or_else(|error| panic!("valid source: {error}"))
    }

    fn reader(bytes: &[u8]) -> AnnexBAccessUnitReader<DuplexStream> {
        let (mut writer, reader) = tokio::io::duplex(64);
        let owned = bytes.to_vec();
        tokio::spawn(async move {
            for chunk in owned.chunks(2) {
                writer
                    .write_all(chunk)
                    .await
                    .unwrap_or_else(|error| panic!("write fixture: {error}"));
            }
        });
        AnnexBAccessUnitReader::new(
            reader,
            NonZeroU32::new(15).expect("nonzero"),
            NonZeroUsize::new(1024).expect("nonzero"),
            NonZeroUsize::new(16).expect("nonzero"),
            NonZeroUsize::new(2048).expect("nonzero"),
        )
    }

    #[test]
    fn validates_device_and_parser_bounds() {
        assert_eq!(
            WebcamInput::new(FfmpegCaptureBackend::AvFoundation, " ".to_owned()),
            Err(FfmpegSourceConfigError::EmptyDevice)
        );
        let input = WebcamInput::new(FfmpegCaptureBackend::Video4Linux2, "/dev/video0".to_owned())
            .expect("device");
        assert!(matches!(
            FfmpegSourceConfig::webcam(
                PathBuf::from("ffmpeg"),
                input,
                640,
                360,
                15,
                500,
                15,
                Duration::from_secs(1),
                Duration::from_secs(1),
                1024,
                16,
                1024,
            ),
            Err(FfmpegSourceConfigError::ParserBoundTooSmall { .. })
        ));
    }

    #[test]
    fn constructs_arguments_without_a_shell() {
        let config = source();
        let command = config.command();
        let arguments: Vec<_> = command
            .as_std()
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect();
        assert!(arguments.windows(2).any(|pair| pair == ["-f", "lavfi"]));
        assert!(arguments
            .iter()
            .any(|argument| argument == "repeat-headers=1:aud=1"));
        assert_eq!(arguments.last().map(String::as_str), Some("pipe:1"));
    }

    #[test]
    fn recognizes_three_and_four_byte_start_codes() {
        assert_eq!(find_start_code(&[9, 0, 0, 1, 5], 0), Some((1, 3)));
        assert_eq!(find_start_code(&[0, 0, 0, 1, 5], 0), Some((0, 4)));
    }

    #[tokio::test]
    async fn parses_fragmented_annex_b_access_units_with_frame_clock() {
        let bytes = [
            0, 0, 0, 1, 0x09, 0xf0, 0, 0, 1, 0x67, 0x42, 0, 0, 1, 0x68, 0xce, 0, 0, 1, 0x65, 0x88,
            0, 0, 1, 0x09, 0xf0, 0, 0, 1, 0x41, 0x9a,
        ];
        let mut reader = reader(&bytes);
        let first = reader
            .next()
            .await
            .expect("first parse")
            .expect("first access unit");
        assert!(first.is_keyframe());
        assert!(first.has_parameter_sets());
        assert_eq!(first.timestamp().ticks(), 0);
        assert_eq!(first.timestamp().timebase_hz(), 90_000);
        let second = reader
            .next()
            .await
            .expect("second parse")
            .expect("second access unit");
        assert!(!second.is_keyframe());
        assert_eq!(second.timestamp().ticks(), 6_000);
        assert!(reader.next().await.expect("eof").is_none());
    }

    #[tokio::test]
    #[ignore = "requires FFmpeg with the libx264 encoder"]
    async fn managed_synthetic_process_produces_media_and_stops() {
        let config = source();
        let mut session = super::FfmpegH264Session::start(&config)
            .await
            .unwrap_or_else(|error| panic!("start synthetic source: {error}"));
        let first = session
            .next()
            .await
            .unwrap_or_else(|error| panic!("read first unit: {error}"))
            .unwrap_or_else(|| panic!("synthetic source ended"));
        assert!(first.is_keyframe());
        assert!(first.has_parameter_sets());
        let second = session
            .next()
            .await
            .unwrap_or_else(|error| panic!("read second unit: {error}"))
            .unwrap_or_else(|| panic!("synthetic source ended"));
        assert!(second.timestamp().ticks() > first.timestamp().ticks());
        session
            .shutdown(Duration::from_secs(2))
            .await
            .unwrap_or_else(|error| panic!("stop synthetic source: {error}"));
    }
}
