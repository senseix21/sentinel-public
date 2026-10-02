//! Source-neutral ownership boundary for one active H.264 session.

use std::{error::Error, fmt, time::Duration};

use crate::{
    connection::{RetryableCameraFailure, TerminalCameraFailure},
    ffmpeg::{
        FfmpegConnectError, FfmpegH264Session, FfmpegReadError, FfmpegSourceConfig,
        FfmpegSourceKind,
    },
    rtsp::{
        connect_h264 as connect_rtsp_h264, ReceivedAccessUnit, RetinaH264Session, RtspConnectError,
        RtspReadError, RtspReadOutcome, RtspShutdownError, RtspSourceConfig,
    },
};

/// One fully validated H.264 source. The exhaustive enum prevents source-
/// specific settings from being combined accidentally.
pub enum H264SourceConfig {
    Rtsp(RtspSourceConfig),
    Ffmpeg(FfmpegSourceConfig),
}

impl H264SourceConfig {
    #[must_use]
    pub const fn kind(&self) -> H264SourceKind {
        match self {
            Self::Rtsp(_) => H264SourceKind::Rtsp,
            Self::Ffmpeg(config) => match config.kind() {
                FfmpegSourceKind::Webcam => H264SourceKind::FfmpegWebcam,
                FfmpegSourceKind::Synthetic => H264SourceKind::FfmpegSynthetic,
            },
        }
    }

    #[must_use]
    pub const fn as_rtsp(&self) -> Option<&RtspSourceConfig> {
        match self {
            Self::Rtsp(config) => Some(config),
            Self::Ffmpeg(_) => None,
        }
    }
}

impl From<RtspSourceConfig> for H264SourceConfig {
    fn from(config: RtspSourceConfig) -> Self {
        Self::Rtsp(config)
    }
}

impl From<FfmpegSourceConfig> for H264SourceConfig {
    fn from(config: FfmpegSourceConfig) -> Self {
        Self::Ffmpeg(config)
    }
}

impl fmt::Debug for H264SourceConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rtsp(config) => formatter.debug_tuple("Rtsp").field(config).finish(),
            Self::Ffmpeg(config) => formatter.debug_tuple("Ffmpeg").field(config).finish(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum H264SourceKind {
    Rtsp,
    FfmpegWebcam,
    FfmpegSynthetic,
}

/// A running source session. Enum dispatch keeps the supported ownership set
/// explicit and avoids a broad async trait before an external adapter exists.
pub enum ActiveH264Session {
    Rtsp(RetinaH264Session),
    Ffmpeg(Box<FfmpegH264Session>),
}

impl ActiveH264Session {
    /// Reads the next source-neutral H.264 outcome.
    ///
    /// # Errors
    /// Returns the source-specific read failure without discarding its cause.
    pub async fn next(&mut self) -> Result<H264SourceReadOutcome, H264SourceReadError> {
        match self {
            Self::Rtsp(session) => session
                .next()
                .await
                .map(H264SourceReadOutcome::from)
                .map_err(H264SourceReadError::Rtsp),
            Self::Ffmpeg(session) => session
                .next()
                .await
                .map(|outcome| match outcome {
                    Some(access_unit) => H264SourceReadOutcome::AccessUnit(ReceivedAccessUnit {
                        access_unit,
                        parameters_changed: false,
                    }),
                    None => H264SourceReadOutcome::EndOfStream,
                })
                .map_err(H264SourceReadError::Ffmpeg),
        }
    }

    /// Closes the active source within the caller's teardown bound.
    ///
    /// # Errors
    /// Returns the source-specific shutdown failure.
    pub async fn shutdown(self, timeout: Duration) -> Result<(), H264SourceShutdownError> {
        match self {
            Self::Rtsp(session) => session
                .shutdown(timeout)
                .await
                .map_err(H264SourceShutdownError::Rtsp),
            Self::Ffmpeg(session) => session
                .shutdown(timeout)
                .await
                .map_err(H264SourceShutdownError::Ffmpeg),
        }
    }
}

#[derive(Clone, Debug)]
pub enum H264SourceReadOutcome {
    AccessUnit(ReceivedAccessUnit),
    PacketLoss { lost_packets: u16 },
    EndOfStream,
}

impl From<RtspReadOutcome> for H264SourceReadOutcome {
    fn from(outcome: RtspReadOutcome) -> Self {
        match outcome {
            RtspReadOutcome::AccessUnit(unit) => Self::AccessUnit(unit),
            RtspReadOutcome::PacketLoss { lost_packets } => Self::PacketLoss { lost_packets },
            RtspReadOutcome::EndOfStream => Self::EndOfStream,
        }
    }
}

/// Starts one validated source without changing downstream ownership.
///
/// # Errors
/// Returns the source-specific connection or readiness failure.
pub async fn connect_h264(
    config: &H264SourceConfig,
) -> Result<ActiveH264Session, H264SourceConnectError> {
    match config {
        H264SourceConfig::Rtsp(config) => connect_rtsp_h264(config)
            .await
            .map(ActiveH264Session::Rtsp)
            .map_err(H264SourceConnectError::Rtsp),
        H264SourceConfig::Ffmpeg(config) => FfmpegH264Session::start(config)
            .await
            .map(Box::new)
            .map(ActiveH264Session::Ffmpeg)
            .map_err(H264SourceConnectError::Ffmpeg),
    }
}

#[derive(Debug)]
pub enum H264SourceConnectError {
    Rtsp(RtspConnectError),
    Ffmpeg(FfmpegConnectError),
}

impl H264SourceConnectError {
    #[must_use]
    pub fn terminal_failure(&self) -> Option<TerminalCameraFailure> {
        match self {
            Self::Rtsp(RtspConnectError::Terminal { failure, .. }) => Some(*failure),
            Self::Ffmpeg(error) => error.terminal_failure(),
            Self::Rtsp(_) => None,
        }
    }

    #[must_use]
    pub const fn retryable_failure(&self) -> RetryableCameraFailure {
        match self {
            Self::Rtsp(RtspConnectError::Timeout) => RetryableCameraFailure::Timeout,
            Self::Rtsp(RtspConnectError::Retryable { failure, .. }) => *failure,
            Self::Rtsp(RtspConnectError::Terminal { .. }) => {
                RetryableCameraFailure::TransportProtocol
            }
            Self::Ffmpeg(error) => error.retryable_failure(),
        }
    }
}

impl fmt::Display for H264SourceConnectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rtsp(error) => write!(formatter, "{error}"),
            Self::Ffmpeg(error) => write!(formatter, "{error}"),
        }
    }
}

impl Error for H264SourceConnectError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Rtsp(error) => Some(error),
            Self::Ffmpeg(error) => Some(error),
        }
    }
}

#[derive(Debug)]
pub enum H264SourceReadError {
    Rtsp(RtspReadError),
    Ffmpeg(FfmpegReadError),
}

impl fmt::Display for H264SourceReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rtsp(error) => write!(formatter, "{error}"),
            Self::Ffmpeg(error) => write!(formatter, "{error}"),
        }
    }
}

impl Error for H264SourceReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Rtsp(error) => Some(error),
            Self::Ffmpeg(error) => Some(error),
        }
    }
}

#[derive(Debug)]
pub enum H264SourceShutdownError {
    Rtsp(RtspShutdownError),
    Ffmpeg(std::io::Error),
}

impl fmt::Display for H264SourceShutdownError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rtsp(error) => write!(formatter, "{error}"),
            Self::Ffmpeg(_) => formatter.write_str("FFmpeg source shutdown failed"),
        }
    }
}

impl Error for H264SourceShutdownError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Rtsp(error) => Some(error),
            Self::Ffmpeg(error) => Some(error),
        }
    }
}
