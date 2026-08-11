//! Retina-backed RTSP/TCP source adapter.

use std::{error::Error, fmt, num::NonZeroUsize, sync::Arc, time::Duration};

use futures_util::StreamExt;
use retina::{
    client::{
        Credentials, Demuxed, PlayOptions, Session, SessionGroup, SessionOptions, SetupOptions,
        TcpTransportOptions, TeardownPolicy, Transport,
    },
    codec::{
        h26x::Framing, CodecItem, FrameFormat, ParameterSetInsertion, ParametersRef, VideoFrame,
    },
};
use surveillance_media::packet::{
    AccessUnitError, H264AccessUnit, H264AccessUnitAssembler, H264NalError, H264NalUnit,
    MediaTimestamp, TimestampError,
};
use url::Url;

use crate::connection::{RetryableCameraFailure, TerminalCameraFailure};

/// A validated RTSP URL that never contains credentials or query secrets.
#[derive(Clone, Eq, PartialEq)]
pub struct RtspEndpoint(Url);

impl RtspEndpoint {
    /// Parses and validates an RTSP source endpoint.
    ///
    /// # Errors
    ///
    /// Returns [`RtspEndpointError`] for malformed URLs, non-RTSP schemes,
    /// missing hosts, embedded credentials, queries, or fragments.
    pub fn parse(value: &str) -> Result<Self, RtspEndpointError> {
        let url = Url::parse(value).map_err(|_| RtspEndpointError::Malformed)?;
        if url.scheme() != "rtsp" {
            return Err(RtspEndpointError::UnsupportedScheme);
        }
        if url.host_str().is_none() {
            return Err(RtspEndpointError::MissingHost);
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(RtspEndpointError::EmbeddedCredentials);
        }
        if url.query().is_some() {
            return Err(RtspEndpointError::QueryNotAllowed);
        }
        if url.fragment().is_some() {
            return Err(RtspEndpointError::FragmentNotAllowed);
        }
        Ok(Self(url))
    }

    fn as_url(&self) -> &Url {
        &self.0
    }
}

impl fmt::Display for RtspEndpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0.as_str())
    }
}

impl fmt::Debug for RtspEndpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("RtspEndpoint")
            .field(&self.0.as_str())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RtspEndpointError {
    Malformed,
    UnsupportedScheme,
    MissingHost,
    EmbeddedCredentials,
    QueryNotAllowed,
    FragmentNotAllowed,
}

impl fmt::Display for RtspEndpointError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed => formatter.write_str("RTSP endpoint is malformed"),
            Self::UnsupportedScheme => formatter.write_str("RTSP endpoint scheme must be rtsp"),
            Self::MissingHost => formatter.write_str("RTSP endpoint must contain a host"),
            Self::EmbeddedCredentials => {
                formatter.write_str("RTSP credentials must not be embedded in the URL")
            }
            Self::QueryNotAllowed => formatter
                .write_str("RTSP endpoint queries are disallowed because they may leak secrets"),
            Self::FragmentNotAllowed => {
                formatter.write_str("RTSP endpoint fragments are not supported")
            }
        }
    }
}

impl Error for RtspEndpointError {}

/// Separately owned RTSP credentials with redacted diagnostics.
pub struct RtspCredentials {
    username: String,
    password: String,
}

impl RtspCredentials {
    /// Validates nonempty username/password credentials.
    ///
    /// # Errors
    ///
    /// Returns [`RtspCredentialsError`] when either field is empty.
    pub fn new(username: String, password: String) -> Result<Self, RtspCredentialsError> {
        if username.is_empty() {
            return Err(RtspCredentialsError::EmptyUsername);
        }
        if password.is_empty() {
            return Err(RtspCredentialsError::EmptyPassword);
        }
        Ok(Self { username, password })
    }

    fn to_retina(&self) -> Credentials {
        Credentials {
            username: self.username.clone(),
            password: self.password.clone(),
        }
    }
}

impl fmt::Debug for RtspCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RtspCredentials")
            .field("username", &"[REDACTED]")
            .field("password", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RtspCredentialsError {
    EmptyUsername,
    EmptyPassword,
}

impl fmt::Display for RtspCredentialsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyUsername => formatter.write_str("RTSP username must not be empty"),
            Self::EmptyPassword => formatter.write_str("RTSP password must not be empty"),
        }
    }
}

impl Error for RtspCredentialsError {}

/// Explicit allocation bounds for one depacketized access unit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RtspFrameLimits {
    max_access_unit_bytes: NonZeroUsize,
    max_nals_per_access_unit: NonZeroUsize,
}

impl RtspFrameLimits {
    /// Validates nonzero access-unit parsing limits.
    ///
    /// # Errors
    ///
    /// Returns [`RtspFrameLimitsError`] when either limit is zero.
    pub fn new(
        max_access_unit_bytes: usize,
        max_nals_per_access_unit: usize,
    ) -> Result<Self, RtspFrameLimitsError> {
        let Some(max_access_unit_bytes) = NonZeroUsize::new(max_access_unit_bytes) else {
            return Err(RtspFrameLimitsError::ZeroMaxAccessUnitBytes);
        };
        let Some(max_nals_per_access_unit) = NonZeroUsize::new(max_nals_per_access_unit) else {
            return Err(RtspFrameLimitsError::ZeroMaxNals);
        };
        Ok(Self {
            max_access_unit_bytes,
            max_nals_per_access_unit,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RtspFrameLimitsError {
    ZeroMaxAccessUnitBytes,
    ZeroMaxNals,
}

impl fmt::Display for RtspFrameLimitsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroMaxAccessUnitBytes => {
                formatter.write_str("maximum RTSP access-unit size must be nonzero")
            }
            Self::ZeroMaxNals => {
                formatter.write_str("maximum NAL count per access unit must be nonzero")
            }
        }
    }
}

impl Error for RtspFrameLimitsError {}

/// Validated configuration for one H.264 RTSP/TCP session.
pub struct RtspSourceConfig {
    endpoint: RtspEndpoint,
    credentials: Option<RtspCredentials>,
    connect_timeout: Duration,
    frame_limits: RtspFrameLimits,
    expected_dimensions: Option<(u32, u32)>,
}

impl RtspSourceConfig {
    /// Creates a source configuration with an explicit nonzero timeout.
    ///
    /// # Errors
    ///
    /// Returns [`RtspSourceConfigError::ZeroConnectTimeout`] for a zero timeout.
    pub fn new(
        endpoint: RtspEndpoint,
        credentials: Option<RtspCredentials>,
        connect_timeout: Duration,
        frame_limits: RtspFrameLimits,
    ) -> Result<Self, RtspSourceConfigError> {
        if connect_timeout.is_zero() {
            return Err(RtspSourceConfigError::ZeroConnectTimeout);
        }
        Ok(Self {
            endpoint,
            credentials,
            connect_timeout,
            frame_limits,
            expected_dimensions: None,
        })
    }

    /// Requires negotiated video dimensions to match the recorder contract.
    ///
    /// # Errors
    ///
    /// Rejects zero dimensions.
    pub fn require_dimensions(
        mut self,
        width: u32,
        height: u32,
    ) -> Result<Self, RtspSourceConfigError> {
        if width == 0 || height == 0 {
            return Err(RtspSourceConfigError::ZeroExpectedDimension);
        }
        self.expected_dimensions = Some((width, height));
        Ok(self)
    }

    #[must_use]
    pub fn endpoint(&self) -> &RtspEndpoint {
        &self.endpoint
    }
}

impl fmt::Debug for RtspSourceConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RtspSourceConfig")
            .field("endpoint", &self.endpoint)
            .field("has_credentials", &self.credentials.is_some())
            .field("connect_timeout", &self.connect_timeout)
            .field("frame_limits", &self.frame_limits)
            .field("expected_dimensions", &self.expected_dimensions)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RtspSourceConfigError {
    ZeroConnectTimeout,
    ZeroExpectedDimension,
}

impl fmt::Display for RtspSourceConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroConnectTimeout => {
                formatter.write_str("RTSP connect timeout must be greater than zero")
            }
            Self::ZeroExpectedDimension => {
                formatter.write_str("expected RTSP dimensions must be nonzero")
            }
        }
    }
}

impl Error for RtspSourceConfigError {}

/// Connects, selects exactly one H.264 video stream, requests interleaved TCP,
/// and starts playback.
///
/// # Errors
///
/// Returns [`RtspConnectError`] with an explicit retryable or terminal domain
/// classification.
pub async fn connect_h264(
    config: &RtspSourceConfig,
) -> Result<RetinaH264Session, RtspConnectError> {
    match tokio::time::timeout(config.connect_timeout, connect_h264_inner(config)).await {
        Ok(result) => result,
        Err(_) => Err(RtspConnectError::Timeout),
    }
}

async fn connect_h264_inner(
    config: &RtspSourceConfig,
) -> Result<RetinaH264Session, RtspConnectError> {
    let session_group = Arc::new(SessionGroup::default().named("surveillance-camera".to_owned()));
    let options = SessionOptions::default()
        .creds(config.credentials.as_ref().map(RtspCredentials::to_retina))
        .session_group(Arc::clone(&session_group))
        .teardown(TeardownPolicy::Always)
        .user_agent("surveillance-platform/0.1".to_owned());
    let mut described = Session::describe(config.endpoint.as_url().clone(), options)
        .await
        .map_err(classify_retina_connect_error)?;

    let Some(stream_index) = described
        .streams()
        .iter()
        .position(|stream| stream.media() == "video" && stream.encoding_name() == "h264")
    else {
        return Err(RtspConnectError::Terminal {
            failure: TerminalCameraFailure::UnsupportedMedia,
            source: None,
        });
    };

    let frame_format = FrameFormat {
        h26x_framing: Framing::FourByteLength,
        parameter_set_insertion: ParameterSetInsertion::EachKeyFrame,
        ..FrameFormat::MP4
    };
    described
        .setup(
            stream_index,
            SetupOptions::default()
                .transport(Transport::Tcp(TcpTransportOptions::default()))
                .frame_format(frame_format),
        )
        .await
        .map_err(classify_retina_connect_error)?;
    let playing = described
        .play(PlayOptions::default())
        .await
        .map_err(classify_retina_connect_error)?;
    let demuxed = playing.demuxed().map_err(classify_retina_connect_error)?;

    let session = RetinaH264Session {
        demuxed,
        stream_index,
        session_group,
        frame_limits: config.frame_limits,
    };
    if let Some(expected) = config.expected_dimensions {
        let Some(parameters) = session.video_parameters() else {
            return Err(RtspConnectError::Terminal {
                failure: TerminalCameraFailure::UnsupportedMedia,
                source: None,
            });
        };
        if (parameters.width, parameters.height) != expected {
            tracing::error!(
                expected_width = expected.0,
                expected_height = expected.1,
                actual_width = parameters.width,
                actual_height = parameters.height,
                codec = %parameters.rfc6381_codec,
                "RTSP dimensions do not match recorder configuration"
            );
            return Err(RtspConnectError::Terminal {
                failure: TerminalCameraFailure::UnsupportedMedia,
                source: None,
            });
        }
    }
    Ok(session)
}

/// A connected H.264 playing session. Dropping it initiates Retina teardown;
/// [`Self::shutdown`] additionally waits for a bounded teardown attempt.
pub struct RetinaH264Session {
    demuxed: Demuxed,
    stream_index: usize,
    session_group: Arc<SessionGroup>,
    frame_limits: RtspFrameLimits,
}

impl RetinaH264Session {
    /// Waits for the next H.264 access unit, packet-loss discontinuity, or EOF.
    ///
    /// # Errors
    ///
    /// Returns [`RtspReadError`] for Retina stream errors or malformed/bounded
    /// frame conversion.
    pub async fn next(&mut self) -> Result<RtspReadOutcome, RtspReadError> {
        loop {
            let Some(item) = self.demuxed.next().await else {
                return Ok(RtspReadOutcome::EndOfStream);
            };
            let item = item.map_err(RtspReadError::Retina)?;
            match item {
                CodecItem::VideoFrame(frame) if frame.stream_id() == self.stream_index => {
                    if frame.loss() != 0 {
                        return Ok(RtspReadOutcome::PacketLoss {
                            lost_packets: frame.loss(),
                        });
                    }
                    return self.convert_frame(&frame).map(RtspReadOutcome::AccessUnit);
                }
                CodecItem::Rtcp(_) => {}
                _ => return Err(RtspReadError::UnexpectedCodecItem),
            }
        }
    }

    /// Returns current probed video parameters when Retina has them.
    #[must_use]
    pub fn video_parameters(&self) -> Option<RtspVideoParameters> {
        let stream = self.demuxed.streams().get(self.stream_index)?;
        let ParametersRef::Video(parameters) = stream.parameters()? else {
            return None;
        };
        let (width, height) = parameters.pixel_dimensions();
        Some(RtspVideoParameters {
            width,
            height,
            rfc6381_codec: parameters.rfc6381_codec().to_owned(),
        })
    }

    /// Drops playback and waits for Retina's teardown attempt.
    ///
    /// # Errors
    ///
    /// Returns [`RtspShutdownError`] if the bounded wait expires or Retina
    /// reports teardown failure.
    pub async fn shutdown(self, timeout: Duration) -> Result<(), RtspShutdownError> {
        let Self {
            demuxed,
            session_group,
            ..
        } = self;
        drop(demuxed);
        match tokio::time::timeout(timeout, session_group.await_teardown()).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(RtspShutdownError::Retina(error)),
            Err(_) => Err(RtspShutdownError::Timeout),
        }
    }

    fn convert_frame(&self, frame: &VideoFrame) -> Result<ReceivedAccessUnit, RtspReadError> {
        let data = frame.data();
        if data.len() > self.frame_limits.max_access_unit_bytes.get() {
            return Err(RtspReadError::AccessUnitTooLarge {
                size: data.len(),
                limit: self.frame_limits.max_access_unit_bytes.get(),
            });
        }
        let retina_timestamp = frame.timestamp();
        let timestamp = MediaTimestamp::new(
            retina_timestamp.timestamp(),
            retina_timestamp.clock_rate().get(),
        )
        .map_err(RtspReadError::Timestamp)?;
        let mut assembler = H264AccessUnitAssembler::new();
        let mut cursor = 0_usize;
        let mut nal_count = 0_usize;
        let mut completed = None;
        while cursor < data.len() {
            let length_bytes = data
                .get(cursor..cursor.saturating_add(4))
                .ok_or(RtspReadError::TruncatedNalLength)?;
            let nal_length = u32::from_be_bytes([
                length_bytes[0],
                length_bytes[1],
                length_bytes[2],
                length_bytes[3],
            ]) as usize;
            cursor += 4;
            if nal_length == 0 {
                return Err(RtspReadError::ZeroLengthNal);
            }
            nal_count = nal_count.saturating_add(1);
            if nal_count > self.frame_limits.max_nals_per_access_unit.get() {
                return Err(RtspReadError::TooManyNals {
                    count: nal_count,
                    limit: self.frame_limits.max_nals_per_access_unit.get(),
                });
            }
            let end = cursor
                .checked_add(nal_length)
                .ok_or(RtspReadError::TruncatedNal)?;
            let bytes = data.get(cursor..end).ok_or(RtspReadError::TruncatedNal)?;
            let nal = H264NalUnit::from_ebsp(Arc::from(bytes)).map_err(RtspReadError::Nal)?;
            let ends_access_unit = end == data.len();
            completed = assembler
                .push(timestamp, nal, ends_access_unit)
                .map_err(RtspReadError::AccessUnit)?;
            cursor = end;
        }
        let access_unit = completed.ok_or(RtspReadError::MissingCompletedAccessUnit)?;
        if access_unit.is_keyframe() != frame.is_random_access_point() {
            return Err(RtspReadError::KeyframeClassificationMismatch);
        }
        Ok(ReceivedAccessUnit {
            access_unit: Arc::new(access_unit),
            parameters_changed: frame.has_new_parameters(),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RtspVideoParameters {
    pub width: u32,
    pub height: u32,
    pub rfc6381_codec: String,
}

#[derive(Clone, Debug)]
pub struct ReceivedAccessUnit {
    pub access_unit: Arc<H264AccessUnit>,
    pub parameters_changed: bool,
}

#[derive(Clone, Debug)]
pub enum RtspReadOutcome {
    AccessUnit(ReceivedAccessUnit),
    PacketLoss { lost_packets: u16 },
    EndOfStream,
}

#[derive(Debug)]
pub enum RtspConnectError {
    Timeout,
    Retryable {
        failure: RetryableCameraFailure,
        source: retina::Error,
    },
    Terminal {
        failure: TerminalCameraFailure,
        source: Option<retina::Error>,
    },
}

impl fmt::Display for RtspConnectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout => formatter.write_str("RTSP connection attempt timed out"),
            Self::Retryable { failure, .. } => {
                write!(formatter, "retryable RTSP connection failure: {failure:?}")
            }
            Self::Terminal { failure, .. } => {
                write!(formatter, "terminal RTSP connection failure: {failure:?}")
            }
        }
    }
}

impl Error for RtspConnectError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Timeout => None,
            Self::Retryable { source, .. } => Some(source),
            Self::Terminal { source, .. } => source.as_ref().map(|error| error as &dyn Error),
        }
    }
}

fn classify_retina_connect_error(error: retina::Error) -> RtspConnectError {
    match error.status_code() {
        Some(401 | 403) => RtspConnectError::Terminal {
            failure: TerminalCameraFailure::AuthenticationRejected,
            source: Some(error),
        },
        Some(404) => RtspConnectError::Retryable {
            // MediaMTX and some camera proxies use 404 while a configured
            // source is temporarily offline. Status alone cannot distinguish
            // that condition from a misspelled path.
            failure: RetryableCameraFailure::ServerUnavailable,
            source: error,
        },
        Some(415) => RtspConnectError::Terminal {
            failure: TerminalCameraFailure::UnsupportedMedia,
            source: Some(error),
        },
        _ => RtspConnectError::Retryable {
            failure: RetryableCameraFailure::TransportProtocol,
            source: error,
        },
    }
}

#[derive(Debug)]
pub enum RtspReadError {
    Retina(retina::Error),
    UnexpectedCodecItem,
    AccessUnitTooLarge { size: usize, limit: usize },
    TruncatedNalLength,
    ZeroLengthNal,
    TruncatedNal,
    TooManyNals { count: usize, limit: usize },
    Nal(H264NalError),
    Timestamp(TimestampError),
    AccessUnit(AccessUnitError),
    MissingCompletedAccessUnit,
    KeyframeClassificationMismatch,
}

impl fmt::Display for RtspReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Retina(_) => formatter.write_str("RTSP media stream failed"),
            Self::UnexpectedCodecItem => {
                formatter.write_str("RTSP session returned an unexpected codec item")
            }
            Self::AccessUnitTooLarge { size, limit } => write!(
                formatter,
                "RTSP access unit size {size} exceeds limit {limit}"
            ),
            Self::TruncatedNalLength => {
                formatter.write_str("RTSP H.264 frame has a truncated NAL length")
            }
            Self::ZeroLengthNal => formatter.write_str("RTSP H.264 frame contains an empty NAL"),
            Self::TruncatedNal => formatter.write_str("RTSP H.264 frame contains a truncated NAL"),
            Self::TooManyNals { count, limit } => write!(
                formatter,
                "RTSP access unit NAL count {count} exceeds limit {limit}"
            ),
            Self::Nal(error) => write!(formatter, "invalid depacketized H.264 NAL: {error}"),
            Self::Timestamp(error) => write!(formatter, "invalid RTSP media timestamp: {error}"),
            Self::AccessUnit(error) => write!(formatter, "invalid RTSP H.264 access unit: {error}"),
            Self::MissingCompletedAccessUnit => {
                formatter.write_str("RTSP H.264 frame did not complete an access unit")
            }
            Self::KeyframeClassificationMismatch => formatter
                .write_str("Retina and media contracts disagree on H.264 keyframe classification"),
        }
    }
}

impl Error for RtspReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Retina(error) => Some(error),
            Self::Nal(error) => Some(error),
            Self::Timestamp(error) => Some(error),
            Self::AccessUnit(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub enum RtspShutdownError {
    Timeout,
    Retina(retina::Error),
}

impl fmt::Display for RtspShutdownError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout => formatter.write_str("RTSP teardown attempt timed out"),
            Self::Retina(_) => formatter.write_str("RTSP teardown attempt failed"),
        }
    }
}

impl Error for RtspShutdownError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Timeout => None,
            Self::Retina(error) => Some(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        RtspCredentials, RtspEndpoint, RtspEndpointError, RtspFrameLimits, RtspFrameLimitsError,
        RtspSourceConfig,
    };
    use std::time::Duration;

    #[test]
    fn endpoint_rejects_embedded_or_query_credentials() {
        assert_eq!(
            RtspEndpoint::parse("rtsp://user:secret@camera.local/live"),
            Err(RtspEndpointError::EmbeddedCredentials)
        );
        assert_eq!(
            RtspEndpoint::parse("rtsp://camera.local/live?token=secret"),
            Err(RtspEndpointError::QueryNotAllowed)
        );
        assert!(RtspEndpoint::parse("rtsp://camera.local:8554/live").is_ok());
    }

    #[test]
    fn credentials_and_source_debug_are_redacted() {
        let credentials = RtspCredentials::new("operator".to_owned(), "secret-value".to_owned())
            .unwrap_or_else(|error| panic!("valid credentials: {error}"));
        let debug = format!("{credentials:?}");
        assert!(!debug.contains("operator"));
        assert!(!debug.contains("secret-value"));

        let endpoint = RtspEndpoint::parse("rtsp://camera.local/live")
            .unwrap_or_else(|error| panic!("valid endpoint: {error}"));
        let limits = RtspFrameLimits::new(1_048_576, 64)
            .unwrap_or_else(|error| panic!("valid limits: {error}"));
        let source =
            RtspSourceConfig::new(endpoint, Some(credentials), Duration::from_secs(5), limits)
                .unwrap_or_else(|error| panic!("valid source: {error}"));
        let debug = format!("{source:?}");
        assert!(!debug.contains("operator"));
        assert!(!debug.contains("secret-value"));
    }

    #[test]
    fn frame_limits_are_explicit_and_nonzero() {
        assert_eq!(
            RtspFrameLimits::new(0, 1),
            Err(RtspFrameLimitsError::ZeroMaxAccessUnitBytes)
        );
        assert_eq!(
            RtspFrameLimits::new(1, 0),
            Err(RtspFrameLimitsError::ZeroMaxNals)
        );
    }

    #[test]
    fn expected_dimensions_are_explicit_and_nonzero() {
        let endpoint = RtspEndpoint::parse("rtsp://127.0.0.1/camera")
            .unwrap_or_else(|error| panic!("valid endpoint: {error}"));
        let limits =
            RtspFrameLimits::new(1_024, 8).unwrap_or_else(|error| panic!("valid limits: {error}"));
        let source = RtspSourceConfig::new(endpoint, None, Duration::from_secs(1), limits)
            .unwrap_or_else(|error| panic!("valid source: {error}"));
        assert!(source.require_dimensions(1920, 1080).is_ok());

        let endpoint = RtspEndpoint::parse("rtsp://127.0.0.1/camera")
            .unwrap_or_else(|error| panic!("valid endpoint: {error}"));
        let limits =
            RtspFrameLimits::new(1_024, 8).unwrap_or_else(|error| panic!("valid limits: {error}"));
        let source = RtspSourceConfig::new(endpoint, None, Duration::from_secs(1), limits)
            .unwrap_or_else(|error| panic!("valid source: {error}"));
        assert!(source.require_dimensions(0, 1080).is_err());
    }
}
