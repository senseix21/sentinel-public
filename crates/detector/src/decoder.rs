//! Owned `FFmpeg` H.264 decode-process adapter.

use crate::detector::{DecodeError, H264Decoder};
use crate::letterbox::LetterboxAlignment;
use std::{
    io::{Read, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{mpsc, Arc},
    thread::JoinHandle,
    time::{Duration, SystemTime},
};
use surveillance_common::types::CameraId;
use surveillance_media::{
    frame::{DecodedFrame, FrameLimits, PixelFormat},
    packet::H264AccessUnit,
};

#[derive(Clone, Debug)]
pub struct FfmpegDecoderConfig {
    pub executable: PathBuf,
    /// The geometry frames are produced at: the model's input size, not the
    /// camera's.
    ///
    /// Decoding at source resolution to feed a 416x416 model cost 6.2 MB per
    /// 1080p frame to produce roughly 0.5 MB of tensor, and put the resize on
    /// the hot path in Rust. Scaling here hands it to `swscale`, which is
    /// vectorised and can be hardware-backed, and makes the letterbox factors
    /// constants derived from configuration rather than per-frame work.
    pub width: u32,
    pub height: u32,
    /// Passed to `FFmpeg` as `-hwaccel` when set. Software decode by default, so
    /// a development host behaves exactly as it did.
    pub hwaccel: Option<String>,
    /// Where to put the padding. The inverse that reads boxes back is built
    /// from the same value, so this is not a free choice.
    pub letterbox_alignment: LetterboxAlignment,
    pub frame_limits: FrameLimits,
    pub response_timeout: Duration,
    pub max_access_unit_bytes: usize,
}

impl FfmpegDecoderConfig {
    /// # Errors
    /// Returns an error for an empty executable, zero timeout/input bound, or
    /// dimensions that cannot fit the configured decoded-frame bound.
    pub fn validate(self) -> Result<Self, DecodeError> {
        if self.executable.as_os_str().is_empty()
            || self.response_timeout.is_zero()
            || self.max_access_unit_bytes == 0
            || self.width == 0
            || self.height == 0
        {
            return Err(DecodeError::Incompatible);
        }
        let bytes = frame_bytes(self.width, self.height)?;
        if self.width > self.frame_limits.max_width
            || self.height > self.frame_limits.max_height
            || bytes > self.frame_limits.max_bytes
        {
            return Err(DecodeError::Oversized);
        }
        Ok(self)
    }
}

enum DecoderCommand {
    Decode {
        unit: Arc<H264AccessUnit>,
        reply: mpsc::SyncSender<Result<Option<DecodedOutput>, DecodeError>>,
    },
    Reset,
    Stop,
}

/// Synchronous proxy whose foreign/process work is owned by one dedicated
/// thread. `DetectionOwner` is responsible for calling it outside Tokio workers.
pub struct FfmpegH264Decoder {
    config: FfmpegDecoderConfig,
    commands: mpsc::SyncSender<DecoderCommand>,
    worker: Option<JoinHandle<()>>,
}

impl FfmpegH264Decoder {
    /// Starts one bounded decoder worker without starting `FFmpeg` until the first
    /// random-access unit.
    ///
    /// # Errors
    /// Returns a typed failure for invalid bounds or worker-start failure.
    pub fn start(config: FfmpegDecoderConfig) -> Result<Self, DecodeError> {
        let config = config.validate()?;
        let (commands, receiver) = mpsc::sync_channel(2);
        let worker_config = config.clone();
        let worker = std::thread::Builder::new()
            .name("h264-detection-decoder".to_owned())
            .spawn(move || run_worker(worker_config, receiver))
            .map_err(|_| DecodeError::Unavailable)?;
        Ok(Self {
            config,
            commands,
            worker: Some(worker),
        })
    }
}

impl H264Decoder for FfmpegH264Decoder {
    fn reset(&mut self) {
        let _ = self.commands.try_send(DecoderCommand::Reset);
    }

    fn decode(
        &mut self,
        camera_id: &CameraId,
        unit: &H264AccessUnit,
        observed_at: SystemTime,
    ) -> Result<Option<DecodedFrame>, DecodeError> {
        let (reply, response) = mpsc::sync_channel(1);
        self.commands
            .send(DecoderCommand::Decode {
                unit: Arc::new(unit.clone()),
                reply,
            })
            .map_err(|_| DecodeError::Unavailable)?;
        let output = response
            .recv_timeout(self.config.response_timeout)
            .map_err(|_| DecodeError::Unavailable)??;
        let Some(output) = output else {
            return Ok(None);
        };
        let stride = usize::try_from(self.config.width)
            .ok()
            .and_then(|width| width.checked_mul(3))
            .ok_or(DecodeError::Oversized)?;
        DecodedFrame::new(
            camera_id.clone(),
            output.unit.connection_epoch(),
            output.unit.timestamp(),
            observed_at,
            self.config.width,
            self.config.height,
            stride,
            PixelFormat::Rgb24,
            output.pixels,
            self.config.frame_limits,
        )
        .map(Some)
        .map_err(|_| DecodeError::Oversized)
    }
}

impl Drop for FfmpegH264Decoder {
    fn drop(&mut self) {
        let _ = self.commands.send(DecoderCommand::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct ActiveDecoder {
    child: Child,
    input: ChildStdin,
    frames: mpsc::Receiver<Result<Arc<[u8]>, DecodeError>>,
    reader: Option<JoinHandle<()>>,
    epoch: u64,
    pending: std::collections::VecDeque<Arc<H264AccessUnit>>,
}

struct DecodedOutput {
    pixels: Arc<[u8]>,
    unit: Arc<H264AccessUnit>,
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "the dedicated thread takes sole ownership of configuration and command receiver"
)]
fn run_worker(config: FfmpegDecoderConfig, commands: mpsc::Receiver<DecoderCommand>) {
    let mut active: Option<ActiveDecoder> = None;
    while let Ok(command) = commands.recv() {
        match command {
            DecoderCommand::Stop => break,
            DecoderCommand::Reset => stop_decoder(&mut active),
            DecoderCommand::Decode { unit, reply } => {
                let result = decode_one(&config, &mut active, &unit);
                let _ = reply.send(result);
            }
        }
    }
    stop_decoder(&mut active);
}

fn decode_one(
    config: &FfmpegDecoderConfig,
    active: &mut Option<ActiveDecoder>,
    unit: &H264AccessUnit,
) -> Result<Option<DecodedOutput>, DecodeError> {
    if unit.connection_epoch() == 0 {
        return Err(DecodeError::Malformed);
    }
    if active
        .as_ref()
        .is_some_and(|decoder| decoder.epoch != unit.connection_epoch())
    {
        stop_decoder(active);
    }
    if active.is_none() {
        if !unit.is_keyframe() || !unit.has_parameter_sets() {
            return Ok(None);
        }
        *active = Some(start_process(config, unit.connection_epoch())?);
    }
    let encoded = annex_b(unit, config.max_access_unit_bytes)?;
    let decoder = active.as_mut().ok_or(DecodeError::Unavailable)?;
    if decoder.input.write_all(&encoded).is_err() || decoder.input.flush().is_err() {
        stop_decoder(active);
        return Err(DecodeError::Unavailable);
    }
    decoder.pending.push_back(Arc::new(unit.clone()));
    if let Ok(frame) = decoder
        .frames
        .recv_timeout(Duration::from_millis(50).min(config.response_timeout))
    {
        let pixels = frame?;
        let source = decoder.pending.pop_front().ok_or(DecodeError::Malformed)?;
        Ok(Some(DecodedOutput {
            pixels,
            unit: source,
        }))
    } else {
        Ok(None)
    }
}

fn start_process(config: &FfmpegDecoderConfig, epoch: u64) -> Result<ActiveDecoder, DecodeError> {
    // Aspect-preserving scale with centred padding, filled with the 114 grey
    // the `YOLOX` family letterboxes with. The decoder emits exactly the model's
    // input geometry, so the adapter copies rather than resizes.
    let scale_filter = format!(
        "scale={width}:{height}:force_original_aspect_ratio=decrease,\
         pad={width}:{height}:{placement}:color=0x727272",
        width = config.width,
        height = config.height,
        placement = config.letterbox_alignment.pad_expression()
    );
    let mut child = Command::new(&config.executable)
        .args(["-hide_banner", "-loglevel", "error"])
        .args(
            config
                .hwaccel
                .as_deref()
                .map_or_else(Vec::new, |accelerator| vec!["-hwaccel", accelerator]),
        )
        .args([
            "-probesize",
            "32",
            "-analyzeduration",
            "0",
            "-f",
            "h264",
            "-i",
            "pipe:0",
            "-an",
            "-vf",
            &scale_filter,
            "-pix_fmt",
            "rgb24",
            "-f",
            "rawvideo",
            "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| DecodeError::Unavailable)?;
    let input = child.stdin.take().ok_or(DecodeError::Unavailable)?;
    let mut output = child.stdout.take().ok_or(DecodeError::Unavailable)?;
    let frame_len = frame_bytes(config.width, config.height)?;
    let (frames_tx, frames) = mpsc::sync_channel(1);
    let reader = std::thread::Builder::new()
        .name("h264-detection-frames".to_owned())
        .spawn(move || loop {
            let mut bytes = vec![0_u8; frame_len];
            if output.read_exact(&mut bytes).is_ok() {
                if frames_tx.send(Ok(bytes.into())).is_err() {
                    return;
                }
            } else {
                let _ = frames_tx.send(Err(DecodeError::Malformed));
                return;
            }
        })
        .map_err(|_| DecodeError::Unavailable)?;
    Ok(ActiveDecoder {
        child,
        input,
        frames,
        reader: Some(reader),
        epoch,
        pending: std::collections::VecDeque::new(),
    })
}

fn stop_decoder(active: &mut Option<ActiveDecoder>) {
    let Some(mut decoder) = active.take() else {
        return;
    };
    drop(decoder.input);
    let _ = decoder.child.kill();
    let _ = decoder.child.wait();
    if let Some(reader) = decoder.reader.take() {
        let _ = reader.join();
    }
}

fn annex_b(unit: &H264AccessUnit, maximum: usize) -> Result<Vec<u8>, DecodeError> {
    let required = unit
        .nal_units()
        .iter()
        .try_fold(0_usize, |total, nal| {
            total.checked_add(4)?.checked_add(nal.bytes().len())
        })
        .and_then(|bytes| bytes.checked_add(6))
        .ok_or(DecodeError::Oversized)?;
    if required > maximum {
        return Err(DecodeError::Oversized);
    }
    let mut output = Vec::with_capacity(required);
    for nal in unit.nal_units() {
        output.extend_from_slice(&[0, 0, 0, 1]);
        output.extend_from_slice(nal.bytes());
    }
    // A following access-unit delimiter lets the streaming parser complete the
    // current picture without waiting for bytes from the next camera frame.
    output.extend_from_slice(&[0, 0, 0, 1, 0x09, 0xf0]);
    Ok(output)
}

fn frame_bytes(width: u32, height: u32) -> Result<usize, DecodeError> {
    usize::try_from(width)
        .ok()
        .and_then(|width| width.checked_mul(height as usize))
        .and_then(|pixels| pixels.checked_mul(3))
        .ok_or(DecodeError::Oversized)
}
