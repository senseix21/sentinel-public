//! Aspect-preserving letterbox geometry and its exact inverse.
//!
//! The detector's decoder scales each frame into the model's input rectangle
//! with the aspect ratio preserved and the remainder padded, so every box the
//! model reports is in letterboxed pixel space and has to be mapped back before
//! it means anything about the camera.
//!
//! That inverse is the highest-risk arithmetic in detection: when it is wrong
//! the boxes stay plausible and become consistently displaced, which no type
//! catches and no fixture without ground truth reveals. It lives here, away
//! from any model runtime, so it can be tested as the pure function it is.

/// The mapping between one source frame and the model's input rectangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Letterbox {
    source_width: f32,
    source_height: f32,
    target_width: u32,
    target_height: u32,
    scale: f32,
    pad_x: f32,
    pad_y: f32,
}

/// Why a letterbox could not be constructed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LetterboxError {
    /// A source or target side was zero, which would divide by zero later.
    DegenerateDimension,
}

impl core::fmt::Display for LetterboxError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("letterbox dimensions must all be non-zero")
    }
}

impl core::error::Error for LetterboxError {}

impl Letterbox {
    /// Builds the mapping for one source frame and model input size.
    ///
    /// # Errors
    /// Returns an error when any dimension is zero. Rejecting here is what
    /// keeps every later operation free of a division guard.
    pub fn new(
        source_width: u32,
        source_height: u32,
        target_width: u32,
        target_height: u32,
    ) -> Result<Self, LetterboxError> {
        if source_width == 0 || source_height == 0 || target_width == 0 || target_height == 0 {
            return Err(LetterboxError::DegenerateDimension);
        }
        let source_width_f = f32::from(u16::try_from(source_width).unwrap_or(u16::MAX));
        let source_height_f = f32::from(u16::try_from(source_height).unwrap_or(u16::MAX));
        let target_width_f = f32::from(u16::try_from(target_width).unwrap_or(u16::MAX));
        let target_height_f = f32::from(u16::try_from(target_height).unwrap_or(u16::MAX));
        let scale = (target_width_f / source_width_f).min(target_height_f / source_height_f);
        let pad_x = (target_width_f - source_width_f * scale) / 2.0;
        let pad_y = (target_height_f - source_height_f * scale) / 2.0;
        Ok(Self {
            source_width: source_width_f,
            source_height: source_height_f,
            target_width,
            target_height,
            scale,
            pad_x,
            pad_y,
        })
    }

    #[must_use]
    pub const fn scale(self) -> f32 {
        self.scale
    }

    #[must_use]
    pub const fn pad(self) -> (f32, f32) {
        (self.pad_x, self.pad_y)
    }

    #[must_use]
    pub const fn target(self) -> (u32, u32) {
        (self.target_width, self.target_height)
    }

    /// Maps a source pixel into model input space.
    #[must_use]
    pub fn forward(self, x: f32, y: f32) -> (f32, f32) {
        (x * self.scale + self.pad_x, y * self.scale + self.pad_y)
    }

    /// Maps a model input pixel back to source pixel space.
    #[must_use]
    pub fn inverse(self, x: f32, y: f32) -> (f32, f32) {
        ((x - self.pad_x) / self.scale, (y - self.pad_y) / self.scale)
    }

    /// Maps a box in model input space to normalized source coordinates.
    ///
    /// Returns `None` when the box is non-finite, or when clamping it to the
    /// frame leaves no area. A detection entirely inside the padding describes
    /// nothing the camera saw, and the event types reject empty geometry, so
    /// discarding it here is the only truthful option.
    #[must_use]
    pub fn to_normalized_source(
        self,
        left: f32,
        top: f32,
        width: f32,
        height: f32,
    ) -> Option<Rect> {
        if ![left, top, width, height].iter().all(|v| v.is_finite()) {
            return None;
        }
        let (x0, y0) = self.inverse(left, top);
        let (x1, y1) = self.inverse(left + width, top + height);
        let left_fraction = (x0 / self.source_width).clamp(0.0, 1.0);
        let top_fraction = (y0 / self.source_height).clamp(0.0, 1.0);
        let right_fraction = (x1 / self.source_width).clamp(0.0, 1.0);
        let bottom_fraction = (y1 / self.source_height).clamp(0.0, 1.0);
        let width = right_fraction - left_fraction;
        let height = bottom_fraction - top_fraction;
        if width <= 0.0 || height <= 0.0 {
            return None;
        }
        Some(Rect {
            left: left_fraction,
            top: top_fraction,
            width,
            height,
        })
    }
}

/// A normalized rectangle, before it becomes event geometry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
}

#[cfg(test)]
mod tests {
    use super::{Letterbox, LetterboxError};

    #[test]
    fn rejects_a_degenerate_dimension_rather_than_dividing_by_zero() {
        for (sw, sh, tw, th) in [
            (0, 720, 416, 416),
            (1280, 0, 416, 416),
            (1280, 720, 0, 416),
            (1280, 720, 416, 0),
        ] {
            assert_eq!(
                Letterbox::new(sw, sh, tw, th),
                Err(LetterboxError::DegenerateDimension),
                "{sw}x{sh} -> {tw}x{th} must be refused"
            );
        }
    }

    #[test]
    fn matches_the_geometry_measured_against_the_real_model() {
        // 1280x720 into 416x416, the configuration the slice 0 probe used.
        let letterbox = Letterbox::new(1280, 720, 416, 416)
            .unwrap_or_else(|error| panic!("valid letterbox: {error}"));
        // f32 tolerances, not exact equality: pad_y computes to 91.00001 here.
        assert!((letterbox.scale() - 0.325).abs() < 1e-6);
        let (pad_x, pad_y) = letterbox.pad();
        assert!(pad_x.abs() < 1e-4);
        assert!((pad_y - 91.0).abs() < 1e-3, "pad_y was {pad_y}");
    }

    #[test]
    fn forward_then_inverse_returns_the_original_point() {
        let cases = [
            (1280, 720, 416, 416),
            (720, 1280, 416, 416),
            (1920, 1080, 640, 640),
            (2560, 1440, 512, 512),
            (640, 640, 416, 416),
            (1, 1, 416, 416),
            (4096, 17, 320, 320),
        ];
        for (sw, sh, tw, th) in cases {
            let letterbox = Letterbox::new(sw, sh, tw, th)
                .unwrap_or_else(|error| panic!("valid letterbox: {error}"));
            for step in 0_u8..=20 {
                let fraction = f32::from(step) / 20.0;
                let x = fraction * f32::from(u16::try_from(sw).unwrap_or(u16::MAX));
                let y = fraction * f32::from(u16::try_from(sh).unwrap_or(u16::MAX));
                let (tx, ty) = letterbox.forward(x, y);
                let (rx, ry) = letterbox.inverse(tx, ty);
                let tolerance = 1e-2 * x.abs().max(y.abs()).max(1.0);
                assert!(
                    (rx - x).abs() <= tolerance && (ry - y).abs() <= tolerance,
                    "{sw}x{sh}->{tw}x{th}: ({x},{y}) became ({rx},{ry})"
                );
            }
        }
    }

    #[test]
    fn normalized_output_stays_inside_the_frame() {
        let letterbox = Letterbox::new(1280, 720, 416, 416)
            .unwrap_or_else(|error| panic!("valid letterbox: {error}"));
        // A box overhanging every edge of the model input.
        let rect = letterbox
            .to_normalized_source(-50.0, -50.0, 600.0, 600.0)
            .unwrap_or_else(|| panic!("an overhanging box still covers the frame"));
        assert!(rect.left >= 0.0 && rect.top >= 0.0);
        assert!(rect.left + rect.width <= 1.0 + 1e-6);
        assert!(rect.top + rect.height <= 1.0 + 1e-6);
    }

    #[test]
    fn a_box_entirely_in_the_padding_is_discarded() {
        let letterbox = Letterbox::new(1280, 720, 416, 416)
            .unwrap_or_else(|error| panic!("valid letterbox: {error}"));
        // pad_y is 91, so this sits above the image entirely.
        assert!(letterbox
            .to_normalized_source(10.0, 0.0, 20.0, 40.0)
            .is_none());
    }

    #[test]
    fn non_finite_geometry_is_discarded_rather_than_propagated() {
        let letterbox = Letterbox::new(1280, 720, 416, 416)
            .unwrap_or_else(|error| panic!("valid letterbox: {error}"));
        assert!(letterbox
            .to_normalized_source(f32::NAN, 10.0, 20.0, 20.0)
            .is_none());
        assert!(letterbox
            .to_normalized_source(10.0, 10.0, f32::INFINITY, 20.0)
            .is_none());
    }

    #[test]
    fn a_full_frame_box_round_trips_to_the_whole_frame() {
        let letterbox = Letterbox::new(1280, 720, 416, 416)
            .unwrap_or_else(|error| panic!("valid letterbox: {error}"));
        let (left, top) = letterbox.forward(0.0, 0.0);
        let (right, bottom) = letterbox.forward(1280.0, 720.0);
        let rect = letterbox
            .to_normalized_source(left, top, right - left, bottom - top)
            .unwrap_or_else(|| panic!("the whole frame is a valid box"));
        assert!(rect.left.abs() < 1e-3 && rect.top.abs() < 1e-3);
        assert!((rect.width - 1.0).abs() < 1e-3 && (rect.height - 1.0).abs() < 1e-3);
    }
}
