//! Sampled human-detection pipeline.

pub mod backend;
pub mod decoder;
pub mod detector;
pub mod gate;
pub mod inference;
pub mod letterbox;
pub mod model_output;
#[cfg(feature = "onnx-inference")]
pub mod onnx;
pub mod sampler;
pub mod tracker;

#[cfg(feature = "onnx-inference")]
pub mod face;
pub mod face_output;

pub mod combined;
pub mod presence;
