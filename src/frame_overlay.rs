use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RectangleConfig {
    pub x: u32,                 // Top-left corner X coordinate
    pub y: u32,                 // Top-left corner Y coordinate
    pub width: u32,             // Rectangle width
    pub height: u32,            // Rectangle height
    pub color: String,          // GStreamer color format, e.g., "red"
    pub line_width: u32,
    pub enabled: bool,
}

impl Default for RectangleConfig {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
            color: "red".to_string(),
            line_width: 3,
            enabled: false,
        }
    }
}

impl RectangleConfig {
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
            ..Default::default()
        }
    }

    pub fn at_position(x: u32, y: u32) -> Self {
        Self {
            x,
            y,
            ..Default::default()
        }
    }

    pub fn with_size(mut self, width: u32, height: u32) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    pub fn with_color(mut self, color: &str) -> Self {
        self.color = color.to_string();
        self
    }

    pub fn with_line_width(mut self, width: u32) -> Self {
        self.line_width = width;
        self
    }

    pub fn enable(mut self) -> Self {
        self.enabled = true;
        self
    }

    pub fn disable(mut self) -> Self {
        self.enabled = false;
        self
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Build FFmpeg drawbox filter arguments using proper syntax
    pub fn build_drawbox_args(&self, _video_width: Option<u32>, _video_height: Option<u32>) -> Vec<String> {
        if !self.enabled {
            return Vec::new();
        }

        // Format: drawbox=x=100:y=100:w=200:h=150:color=red@0.8:t=5
        let filter = format!("drawbox=x={}:y={}:w={}:h={}:color={}:t={}",
            self.x, self.y, self.width, self.height, self.color, self.line_width);

        vec![filter]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameOverlayConfig {
    pub rectangle: RectangleConfig,
    // Future overlay features can be added here
}

impl Default for FrameOverlayConfig {
    fn default() -> Self {
        Self {
            rectangle: RectangleConfig::default(),
        }
    }
}

impl FrameOverlayConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_rectangle(mut self, x: u32, y: u32, width: u32, height: u32) -> Self {
        self.rectangle = RectangleConfig::new(x, y, width, height).enable();
        self
    }

    pub fn with_centered_rectangle(mut self, width: u32, height: u32) -> Self {
        let video_width = 640; // Default width for centering calculation
        let video_height = 480; // Default height for centering calculation
        let x = (video_width - width) / 2;
        let y = (video_height - height) / 2;
        self.rectangle = RectangleConfig::new(x, y, width, height).enable();
        self
    }

    pub fn with_positioned_rectangle(mut self, x: u32, y: u32, width: u32, height: u32) -> Self {
        self.rectangle = RectangleConfig::new(x, y, width, height).enable();
        self
    }

    pub fn is_enabled(&self) -> bool {
        self.rectangle.is_enabled()
    }

    /// Build the complete video filter chain for overlay
    pub fn build_video_filter(&self, video_width: Option<u32>, video_height: Option<u32>) -> Option<String> {
        if !self.is_enabled() {
            return None;
        }

        let mut filters = Vec::new();

        // Add rectangle drawbox filter
        let rectangle_filters = self.rectangle.build_drawbox_args(video_width, video_height);
        filters.extend(rectangle_filters);

        if filters.is_empty() {
            None
        } else {
            Some(filters.join(","))
        }
    }

    /// Build FFmpeg arguments for applying overlay
    pub fn build_ffmpeg_args(&self, video_width: Option<u32>, video_height: Option<u32>) -> Vec<String> {
        let mut args = Vec::new();

        if let Some(filter) = self.build_video_filter(video_width, video_height) {
            args.push("-vf".to_string());
            args.push(filter);
        }

        args
    }
}

/// Helper function to create a centered 100x100 red rectangle overlay
pub fn create_centered_red_rectangle() -> FrameOverlayConfig {
    FrameOverlayConfig::new().with_centered_rectangle(100, 100)
}

/// Helper function to create a 100x100 red rectangle at specific position
pub fn create_red_rectangle_at_position(x: u32, y: u32) -> FrameOverlayConfig {
    FrameOverlayConfig::new().with_rectangle(x, y, 100, 100)
}

/// Helper function to create a custom overlay
pub fn create_custom_overlay(
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    color: &str,
    line_width: u32
) -> FrameOverlayConfig {
    let mut config = FrameOverlayConfig::new();

    let rectangle = RectangleConfig::new(x, y, width, height)
        .with_color(color)
        .with_line_width(line_width)
        .enable();

    config.rectangle = rectangle;
    config
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rectangle_config_creation() {
        let rect = RectangleConfig::new(50, 75, 100, 100);

        assert_eq!(rect.x, 50);
        assert_eq!(rect.y, 75);
        assert_eq!(rect.width, 100);
        assert_eq!(rect.height, 100);
        assert!(!rect.enabled);
        assert_eq!(rect.color, "red");
        assert_eq!(rect.line_width, 3);
    }

    #[test]
    fn test_rectangle_at_position() {
        let rect = RectangleConfig::at_position(50, 75);

        assert_eq!(rect.x, 50);
        assert_eq!(rect.y, 75);
        assert_eq!(rect.width, 100);
        assert_eq!(rect.height, 100);
    }

    #[test]
    fn test_rectangle_with_size() {
        let rect = RectangleConfig::at_position(50, 75).with_size(200, 150);

        assert_eq!(rect.x, 50);
        assert_eq!(rect.y, 75);
        assert_eq!(rect.width, 200);
        assert_eq!(rect.height, 150);
    }

    #[test]
    fn test_rectangle_color_and_line_width() {
        let rect = RectangleConfig::new(50, 75, 200, 150)
            .with_color("blue")
            .with_line_width(5);

        assert_eq!(rect.color, "blue");
        assert_eq!(rect.line_width, 5);
    }

    #[test]
    fn test_rectangle_enable_disable() {
        let rect = RectangleConfig::new(50, 75, 100, 100)
            .enable();

        assert!(rect.enabled);

        let disabled_rect = rect.disable();
        assert!(!disabled_rect.enabled);
    }

    #[test]
    fn test_frame_overlay_config() {
        let overlay = FrameOverlayConfig::new()
            .with_centered_rectangle(150, 150);

        assert!(overlay.is_enabled());
        assert_eq!(overlay.rectangle.width, 150);
        assert_eq!(overlay.rectangle.height, 150);
    }

    #[test]
    fn test_frame_overlay_with_position() {
        let overlay = FrameOverlayConfig::new()
            .with_rectangle(100, 200, 300, 250);

        assert!(overlay.is_enabled());
        assert_eq!(overlay.rectangle.x, 100);
        assert_eq!(overlay.rectangle.y, 200);
        assert_eq!(overlay.rectangle.width, 300);
        assert_eq!(overlay.rectangle.height, 250);
    }

    #[test]
    fn test_video_filter_building() {
        let overlay = FrameOverlayConfig::new().with_centered_rectangle(100, 100);
        let filter = overlay.build_video_filter(Some(800), Some(600));

        assert!(filter.is_some());
        let filter_str = filter.unwrap();
        assert!(filter_str.contains("drawbox"));
        assert!(filter_str.contains("x=270:y=190:w=100:h=100:t=3")); // Centered in 640x480 default
    }

    #[test]
    fn test_video_filter_building_disabled() {
        let overlay = FrameOverlayConfig::new();
        let filter = overlay.build_video_filter(Some(800), Some(600));

        assert!(filter.is_none());
    }

    #[test]
    fn test_ffmpeg_args() {
        let overlay = FrameOverlayConfig::new().with_rectangle(50, 75, 200, 150);
        let args = overlay.build_ffmpeg_args(Some(800), Some(600));

        assert_eq!(args.len(), 2);
        assert_eq!(args[0], "-vf");
        assert!(args[1].contains("drawbox"));
        assert!(args[1].contains("x=50:y=75:w=200:h=150:t=3"));
    }

    #[test]
    fn test_helper_functions() {
        // Test centered rectangle helper
        let overlay = create_centered_red_rectangle();
        assert!(overlay.is_enabled());
        assert_eq!(overlay.rectangle.width, 100);
        assert_eq!(overlay.rectangle.height, 100);
        assert_eq!(overlay.rectangle.color, "red");

        // Test positioned rectangle helper
        let overlay2 = create_red_rectangle_at_position(50, 75);
        assert!(overlay2.is_enabled());
        assert_eq!(overlay2.rectangle.x, 50);
        assert_eq!(overlay2.rectangle.y, 75);

        // Test custom overlay helper
        let overlay3 = create_custom_overlay(100, 200, 300, 250, "blue", 5);
        assert!(overlay3.is_enabled());
        assert_eq!(overlay3.rectangle.x, 100);
        assert_eq!(overlay3.rectangle.y, 200);
        assert_eq!(overlay3.rectangle.width, 300);
        assert_eq!(overlay3.rectangle.height, 250);
        assert_eq!(overlay3.rectangle.color, "blue");
        assert_eq!(overlay3.rectangle.line_width, 5);
    }
}