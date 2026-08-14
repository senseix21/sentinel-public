//! Application-level ownership and shutdown policy.

pub mod authorize;
pub mod camera_supervisor;
pub mod health_http;
pub mod metadata_writer;
pub mod one_camera;
pub mod phase4_metrics;
pub mod pipeline;
pub mod playback;
pub mod reconciliation_owner;
pub mod retention_owner;
pub mod runtime_config;
pub mod settings_service;
