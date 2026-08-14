//! Validated decoded-frame representation with bounded owned storage.

use crate::packet::MediaTimestamp;
use std::{error::Error, fmt, sync::Arc, time::SystemTime};
use surveillance_common::types::CameraId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PixelFormat {
    Rgb24,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameLimits {
    pub max_width: u32,
    pub max_height: u32,
    pub max_bytes: usize,
}
impl FrameLimits {
    /// # Errors
    /// Returns an error when any bound is zero.
    pub fn new(max_width: u32, max_height: u32, max_bytes: usize) -> Result<Self, FrameError> {
        if max_width == 0 || max_height == 0 || max_bytes == 0 {
            Err(FrameError::InvalidLimits)
        } else {
            Ok(Self {
                max_width,
                max_height,
                max_bytes,
            })
        }
    }
}

#[derive(Clone, Debug)]
pub struct DecodedFrame {
    camera_id: CameraId,
    connection_epoch: u64,
    media_timestamp: MediaTimestamp,
    observed_at: SystemTime,
    width: u32,
    height: u32,
    stride: usize,
    format: PixelFormat,
    pixels: Arc<[u8]>,
}
impl DecodedFrame {
    /// # Errors
    /// Returns an error when identity, dimensions, layout, or storage violate bounds.
    #[allow(
        clippy::too_many_arguments,
        reason = "all frame invariants are validated together"
    )]
    pub fn new(
        camera_id: CameraId,
        connection_epoch: u64,
        media_timestamp: MediaTimestamp,
        observed_at: SystemTime,
        width: u32,
        height: u32,
        stride: usize,
        format: PixelFormat,
        pixels: Arc<[u8]>,
        limits: FrameLimits,
    ) -> Result<Self, FrameError> {
        if connection_epoch == 0 {
            return Err(FrameError::ZeroEpoch);
        }
        if width == 0 || height == 0 || width > limits.max_width || height > limits.max_height {
            return Err(FrameError::Dimensions);
        }
        let bytes_per_pixel = match format {
            PixelFormat::Rgb24 => 3_usize,
        };
        let minimum_stride = usize::try_from(width)
            .ok()
            .and_then(|w| w.checked_mul(bytes_per_pixel))
            .ok_or(FrameError::ByteLength)?;
        if stride != minimum_stride {
            return Err(FrameError::Stride);
        }
        let expected = stride
            .checked_mul(usize::try_from(height).map_err(|_| FrameError::ByteLength)?)
            .ok_or(FrameError::ByteLength)?;
        if expected != pixels.len() || expected > limits.max_bytes {
            return Err(FrameError::ByteLength);
        }
        Ok(Self {
            camera_id,
            connection_epoch,
            media_timestamp,
            observed_at,
            width,
            height,
            stride,
            format,
            pixels,
        })
    }
    #[must_use]
    pub fn camera_id(&self) -> &CameraId {
        &self.camera_id
    }
    #[must_use]
    pub const fn connection_epoch(&self) -> u64 {
        self.connection_epoch
    }
    #[must_use]
    pub const fn media_timestamp(&self) -> MediaTimestamp {
        self.media_timestamp
    }
    #[must_use]
    pub const fn observed_at(&self) -> SystemTime {
        self.observed_at
    }
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }
    #[must_use]
    pub const fn stride(&self) -> usize {
        self.stride
    }
    #[must_use]
    pub const fn pixel_format(&self) -> PixelFormat {
        self.format
    }
    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameError {
    InvalidLimits,
    ZeroEpoch,
    Dimensions,
    Stride,
    ByteLength,
}
impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid decoded frame: {self:?}")
    }
}
impl Error for FrameError {}

#[cfg(test)]
mod tests {
    use super::{DecodedFrame, FrameError, FrameLimits, PixelFormat};
    use crate::packet::MediaTimestamp;
    use std::{sync::Arc, time::SystemTime};
    use surveillance_common::types::CameraId;
    fn limits() -> FrameLimits {
        FrameLimits::new(1920, 1080, 1920 * 1080 * 3)
            .unwrap_or_else(|error| panic!("limits: {error}"))
    }
    #[test]
    fn accepts_exact_bounded_rgb_frame() {
        let frame = DecodedFrame::new(
            CameraId::new("camera-01").unwrap_or_else(|error| panic!("camera: {error}")),
            1,
            MediaTimestamp::new(90_000, 90_000)
                .unwrap_or_else(|error| panic!("timestamp: {error}")),
            SystemTime::UNIX_EPOCH,
            2,
            2,
            6,
            PixelFormat::Rgb24,
            Arc::from([0_u8; 12]),
            limits(),
        )
        .unwrap_or_else(|error| panic!("frame: {error}"));
        assert_eq!(frame.pixels().len(), 12);
    }
    #[test]
    fn rejects_oversize_stride_and_length() {
        let camera = CameraId::new("camera-01").unwrap_or_else(|error| panic!("camera: {error}"));
        let timestamp =
            MediaTimestamp::new(1, 1).unwrap_or_else(|error| panic!("timestamp: {error}"));
        assert!(matches!(
            DecodedFrame::new(
                camera.clone(),
                1,
                timestamp,
                SystemTime::UNIX_EPOCH,
                1921,
                1,
                5763,
                PixelFormat::Rgb24,
                Arc::from([]),
                limits()
            ),
            Err(FrameError::Dimensions)
        ));
        assert!(matches!(
            DecodedFrame::new(
                camera,
                1,
                timestamp,
                SystemTime::UNIX_EPOCH,
                2,
                2,
                7,
                PixelFormat::Rgb24,
                Arc::from([0_u8; 14]),
                limits()
            ),
            Err(FrameError::Stride)
        ));
    }
}
