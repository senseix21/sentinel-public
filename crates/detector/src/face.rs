//! Checksum-verified `YuNet` face inference. No recognition or coasted boxes.
use crate::{
    backend::InferenceBackendConfig,
    face_output::{decode_yunet_observed, FaceLevel, YUNET_INPUT_SIZE},
    inference::{
        DetectionInference, DetectionMode, InferenceError, InferenceFailureKind, InferenceResults,
    },
    letterbox::Letterbox,
    onnx::{build_session, ModelArtifact, OnnxBackendError},
};
use surveillance_events::event::{ModelName, ModelVersion};
use surveillance_media::frame::DecodedFrame;

pub struct OnnxFaceInference {
    session: ort::session::Session,
    geometry: Letterbox,
    name: ModelName,
    version: ModelVersion,
    scratch: Vec<f32>,
    score: u8,
    nms: u8,
    output_capacity_dropped: u64,
}
impl OnnxFaceInference {
    /// # Errors
    /// Refuses absent/unpinned artifacts, unsupported geometry, runtime or tensor failures.
    pub fn load(config: &InferenceBackendConfig) -> Result<Self, OnnxBackendError> {
        if config.input_width != YUNET_INPUT_SIZE
            || config.input_height != YUNET_INPUT_SIZE
            || config.model_sha256.is_none()
        {
            return Err(OnnxBackendError::Session(
                "YuNet requires pinned 640x640 input".to_owned(),
            ));
        }
        let path = config
            .model_path
            .as_ref()
            .ok_or(OnnxBackendError::Identity)?;
        let artifact = ModelArtifact::load(path, config.model_sha256.as_deref())?;
        let session = build_session(config, &artifact).map_err(OnnxBackendError::Session)?;
        // Reject incompatible graphs before running them; every declared output
        // is bounded by this artifact's geometry, including unused landmarks.
        let shape_matches = |outlet: &ort::value::Outlet, expected: &[i64]| matches!(outlet.dtype(), ort::value::ValueType::Tensor {ty:ort::value::TensorElementType::Float32,shape,..} if shape.as_ref()==expected);
        if session.inputs().len() != 1
            || session.inputs()[0].name() != "input"
            || !shape_matches(&session.inputs()[0], &[1, 3, 640, 640])
            || session.outputs().len() != 12
        {
            return Err(OnnxBackendError::Session(
                "unsupported YuNet input/output contract".to_owned(),
            ));
        }
        for stride in [8_i64, 16, 32] {
            for (kind, channels) in [("cls", 1), ("obj", 1), ("bbox", 4), ("kps", 10)] {
                let name = format!("{kind}_{stride}");
                let expected = [1, (640 / stride) * (640 / stride), channels];
                if !session
                    .outputs()
                    .iter()
                    .any(|outlet| outlet.name() == name && shape_matches(outlet, &expected))
                {
                    return Err(OnnxBackendError::Session(
                        "unsupported YuNet output shape".to_owned(),
                    ));
                }
            }
        }
        let geometry = Letterbox::new(
            config.source_width,
            config.source_height,
            640,
            640,
            config.letterbox_alignment,
        )
        .map_err(|_| OnnxBackendError::Identity)?;
        let mut backend = Self {
            session,
            geometry,
            name: ModelName::new("yunet-face").map_err(|_| OnnxBackendError::Identity)?,
            version: ModelVersion::new(&artifact.sha256()[..16])
                .map_err(|_| OnnxBackendError::Identity)?,
            scratch: vec![0.0; 3 * 640 * 640],
            score: config.score_threshold_percent,
            nms: config.nms_iou_percent,
            output_capacity_dropped: 0,
        };
        backend
            .run()
            .map_err(|e| OnnxBackendError::WarmUp(e.to_string()))?;
        Ok(backend)
    }
    fn run(&mut self) -> Result<InferenceResults, InferenceError> {
        let malformed = || InferenceError::new(InferenceFailureKind::MalformedOutput);
        let tensor = ort::value::TensorRef::from_array_view((
            [1_usize, 3, 640, 640],
            self.scratch.as_slice(),
        ))
        .map_err(|_| malformed())?;
        let output = self
            .session
            .run(ort::inputs!["input"=>tensor])
            .map_err(|_| InferenceError::new(InferenceFailureKind::Retryable))?;
        let mut levels = Vec::with_capacity(3);
        for stride in [8_u16, 16, 32] {
            let extract = |kind: &str, channels: usize| {
                let name = format!("{kind}_{stride}");
                let value = output.get(&name).ok_or_else(malformed)?;
                let (shape, values) = value.try_extract_tensor::<f32>().map_err(|_| malformed())?;
                let locations = i64::from((640 / stride) * (640 / stride));
                if shape.as_ref()
                    != [
                        1,
                        locations,
                        i64::try_from(channels).map_err(|_| malformed())?,
                    ]
                {
                    return Err(malformed());
                }
                Ok(values)
            };
            levels.push(FaceLevel {
                stride,
                classes: extract("cls", 1)?,
                objects: extract("obj", 1)?,
                boxes: extract("bbox", 4)?,
            });
        }
        let (set, dropped) = decode_yunet_observed(&levels, self.geometry, self.score, self.nms)?;
        self.output_capacity_dropped = dropped;
        Ok(InferenceResults::Faces(set))
    }
}
impl DetectionInference for OnnxFaceInference {
    fn output_capacity_dropped(&self) -> u64 {
        self.output_capacity_dropped
    }
    fn model_name(&self) -> &ModelName {
        &self.name
    }
    fn model_version(&self) -> &ModelVersion {
        &self.version
    }
    fn mode(&self) -> DetectionMode {
        DetectionMode::Face
    }
    fn is_production_model(&self) -> bool {
        true
    }
    fn infer_detections(
        &mut self,
        frame: &DecodedFrame,
    ) -> Result<InferenceResults, InferenceError> {
        if frame.width() != 640 || frame.height() != 640 {
            return Err(InferenceError::new(InferenceFailureKind::MalformedOutput));
        }
        let plane = 640 * 640;
        for y in 0..640 {
            for x in 0..640 {
                for c in 0..3 {
                    self.scratch[c * plane + y * 640 + x] =
                        f32::from(frame.pixels()[y * frame.stride() + x * 3 + 2 - c]);
                }
            }
        }
        self.run()
    }
}
