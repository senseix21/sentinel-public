//! Streaming H.264 → I420 decode for the live-viewing path.
//!
//! This exists because encoded pass-through does not run on any platform
//! `LiveKit` ships; see `docs/adr/012-viewer-gated-transcode-publication.md`.
//! Frames decoded here are handed to libwebrtc as raw pictures and encoded by
//! libwebrtc, whose *encoders* work correctly — only its pass-through path does
//! not.
//!
//! It is deliberately not `crates/detector/src/decoder.rs`. That one is
//! request/response, one frame at a time, under a short timeout, in RGB24, for
//! *sampled* detection. This one streams at the camera's full frame rate in
//! I420. One component serving both would serve both badly (ADR 012
//! Decision 6).

use std::{
    fmt,
    process::Stdio,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    task::JoinHandle,
    time::timeout,
};

/// The frame geometry a decode session is pinned to.
///
/// The size is forced on `FFmpeg`'s output rather than discovered from it, which
/// is what lets the reader consume fixed-length frames and makes a mid-stream
/// resolution change a scaling event instead of a desynchronised byte stream
/// (ADR 012 Decision 4).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameSize {
    width: u32,
    height: u32,
}

impl FrameSize {
    /// Largest dimension either axis may take.
    ///
    /// Not a policy about cameras — a guard against an implausible sequence
    /// parameter set turning into a multi-gigabyte frame buffer.
    const MAX_DIMENSION: u32 = 8192;

    /// Builds a frame size, rounding each axis down to an even number.
    ///
    /// I420 subsamples chroma by two on both axes, so an odd dimension has no
    /// exact representation. Rounding down keeps `frame_bytes` exact rather
    /// than approximately right.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError::Geometry`] when either axis is zero after
    /// rounding, or implausibly large.
    pub const fn new(width: u32, height: u32) -> Result<Self, DecodeError> {
        let width = width & !1;
        let height = height & !1;
        if width == 0 || height == 0 || width > Self::MAX_DIMENSION || height > Self::MAX_DIMENSION
        {
            return Err(DecodeError::Geometry);
        }
        Ok(Self { width, height })
    }

    /// Scales down to fit a maximum height, preserving aspect ratio.
    ///
    /// Returns `self` when already within the limit: this never scales *up*,
    /// because inventing detail costs the same CPU as carrying real detail and
    /// delivers less.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError::Geometry`] when the scaled result has no valid
    /// even representation.
    pub const fn capped_to_height(self, max_height: u32) -> Result<Self, DecodeError> {
        if max_height == 0 || self.height <= max_height {
            return Ok(self);
        }
        // Integer arithmetic, deliberately: a float ratio here would make the
        // published resolution depend on rounding mode.
        //
        // The product cannot overflow. This line is only reached when
        // `max_height < self.height`, and `self.height` is at most
        // `MAX_DIMENSION`, so both factors are bounded by 8192 and the product
        // by 2^26.
        let width = self.width * max_height / self.height;
        Self::new(width, max_height)
    }

    #[must_use]
    pub const fn width(self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(self) -> u32 {
        self.height
    }

    /// Bytes in one planar I420 frame of this size.
    ///
    /// Exact because both axes are even: a full-size luma plane plus two
    /// half-by-half chroma planes.
    #[must_use]
    pub const fn frame_bytes(self) -> usize {
        let luma = self.width as usize * self.height as usize;
        luma + luma / 2
    }
}

/// Receives decoded frames, in planar I420, from the reader task.
///
/// A trait rather than a channel so the decoder can be tested without a
/// `LiveKit` room, and so the frame goes straight from the pipe to libwebrtc
/// with no queue of our own in between. libwebrtc's encoder queue is the bound;
/// interposing another would add latency to the one consumer that is supposed
/// to prefer freshness.
pub trait DecodedFrameSink: Send + 'static {
    /// One decoded frame, exactly [`FrameSize::frame_bytes`] long, planar I420.
    fn accept(&mut self, yuv420p: &[u8], size: FrameSize);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecodeError {
    /// No decoder binary. Not retryable: it will still be absent next time.
    Unavailable,
    /// The frame geometry cannot be represented.
    Geometry,
    /// The decoder died, or its pipes did.
    Failed,
    /// A write did not complete within its budget. The session is poisoned:
    /// a timeout can leave a truncated NAL in the pipe, and trusting the
    /// parser to resynchronise past it is a guess. The caller restarts.
    Overloaded,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unavailable => "no H.264 decoder binary is available",
            Self::Geometry => "frame geometry cannot be represented in I420",
            Self::Failed => "the decode process failed",
            Self::Overloaded => "the decode process did not accept input in time",
        })
    }
}

impl std::error::Error for DecodeError {}

/// Counters a decode session exposes for its own overload behaviour.
#[derive(Debug, Default)]
pub struct DecodeCounters {
    pub frames_decoded: AtomicU64,
}

/// An owned `FFmpeg` process decoding Annex-B H.264 into I420 frames.
///
/// Owns its child and two tasks — a frame reader and a stderr drain — and joins
/// both on [`Self::shutdown`]. The child is `kill_on_drop`, so an abandoned
/// session cannot outlive the process the way an orphaned stack child does.
pub struct H264ToI420 {
    child: Child,
    stdin: Option<ChildStdin>,
    reader: JoinHandle<()>,
    stderr: JoinHandle<()>,
    size: FrameSize,
    counters: Arc<DecodeCounters>,
    /// Whether a write to this session has ever completed.
    ///
    /// Distinguishes a decoder that has not started reading yet from one that
    /// has stopped keeping up. See [`Self::SPAWN_WRITE_TIMEOUT`].
    accepted_a_write: bool,
}

impl H264ToI420 {
    /// How long a write to the decoder may take before the session is poisoned.
    ///
    /// This bounds the only place the publication path can block on a slow
    /// consumer. It is generous against a scheduling hiccup and far short of
    /// any interval a viewer would tolerate.
    pub const WRITE_TIMEOUT: Duration = Duration::from_millis(250);

    /// How long the *first* write of a session may take.
    ///
    /// A cold `FFmpeg` has not reached its input loop yet: it is still loading
    /// its own libraries and initialising a decoder. Until it does, nothing
    /// drains the stdin pipe, and an access unit larger than the kernel pipe
    /// buffer blocks — the first picture of a stream is the largest one, so this
    /// is the normal case rather than a rare one.
    ///
    /// Measured here on 2026-08-17: with a single 250 ms budget every watch
    /// began with exactly one `Overloaded` restart at startup and then ran for
    /// 45 s with none, so the operator paid a keyframe interval of black on
    /// every subscribe for a stall that was never backpressure.
    ///
    /// Two budgets rather than one generous one: raising [`Self::WRITE_TIMEOUT`]
    /// to cover process startup would blind the steady-state overload detector
    /// for the entire session, which is the one thing it exists to catch.
    pub const SPAWN_WRITE_TIMEOUT: Duration = Duration::from_secs(2);

    /// How long a closing decoder may take to flush and exit before it is
    /// killed.
    ///
    /// Teardown happens when the last viewer leaves, so this is not on a
    /// latency path; it is long enough for a decoder to emit what it already
    /// holds and short enough that a wedged one cannot delay a shutdown.
    pub const SHUTDOWN_GRACE: Duration = Duration::from_secs(1);

    /// Starts a decode session producing frames of exactly `size`.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError::Unavailable`] when `program` cannot be executed,
    /// and [`DecodeError::Failed`] when its pipes cannot be taken.
    pub fn spawn(
        program: &str,
        size: FrameSize,
        mut sink: impl DecodedFrameSink,
        counters: Arc<DecodeCounters>,
    ) -> Result<Self, DecodeError> {
        let mut child = Command::new(program)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                // Never let the decoder try to read the terminal; it shares
                // stdin with nothing and must not consume ours.
                "-nostdin",
                // Deliberately absent: `-fflags nobuffer` and `-flags
                // low_delay`, the two options this would otherwise reach for.
                //
                // `-fflags nobuffer` on a raw H.264 pipe produces *no output at
                // all* here. Measured on ffmpeg 8.0: the same command with and
                // without it writes 0 and 1152000 bytes respectively, exit 0
                // and silent either way, which is exactly the failure that is
                // impossible to diagnose from a live stream.
                //
                // `-flags low_delay` tells the H.264 decoder to emit pictures
                // without waiting for reordering. That is only correct for a
                // stream with no B-frames, and this code does not get to assume
                // what a camera sends. Where there are none it costs nothing to
                // omit; where there are some, omitting it is the difference
                // between right and scrambled.
                "-f",
                "h264",
                "-i",
                "pipe:0",
                "-an",
                // Passthrough rather than a constant frame rate. The default
                // would duplicate or drop pictures to reach a guessed rate,
                // which is fabricated media.
                "-fps_mode",
                "passthrough",
                "-pix_fmt",
                "yuv420p",
                "-s",
                &format!("{}x{}", size.width(), size.height()),
                "-f",
                "rawvideo",
                // Without this the output sits in ffmpeg's AVIO buffer until it
                // fills, so the reader sees frames in bursts and the tail of a
                // stalled stream never arrives at all. This is the low-latency
                // knob that works, as distinct from the two above.
                "-flush_packets",
                "1",
                "pipe:1",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // An abandoned session must not leave a decoder running. This is
            // the same class of defect as the orphaned stack children that
            // cost a debugging session on 2026-08-16.
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| DecodeError::Unavailable)?;

        let stdin = child.stdin.take().ok_or(DecodeError::Failed)?;
        let mut stdout = child.stdout.take().ok_or(DecodeError::Failed)?;
        let stderr_pipe = child.stderr.take().ok_or(DecodeError::Failed)?;

        let frame_bytes = size.frame_bytes();
        let reader_counters = Arc::clone(&counters);
        let reader = tokio::spawn(async move {
            let mut frame = vec![0_u8; frame_bytes];
            // Ends on EOF or error, which is how the decoder reports that it
            // is gone. There is no other exit, and no unbounded growth: the
            // buffer is allocated once and reused.
            while stdout.read_exact(&mut frame).await.is_ok() {
                reader_counters
                    .frames_decoded
                    .fetch_add(1, Ordering::Relaxed);
                sink.accept(&frame, size);
            }
        });

        let stderr = tokio::spawn(async move {
            // The child-process publisher this design replaces discarded its
            // child's stderr, so a failing decoder was invisible. Surface it.
            let mut lines = BufReader::new(stderr_pipe).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::warn!(decoder_stderr = %line, "live decode reported an error");
            }
        });

        Ok(Self {
            child,
            stdin: Some(stdin),
            reader,
            stderr,
            size,
            counters,
            accepted_a_write: false,
        })
    }

    #[must_use]
    pub const fn size(&self) -> FrameSize {
        self.size
    }

    #[must_use]
    pub fn frames_decoded(&self) -> u64 {
        self.counters.frames_decoded.load(Ordering::Relaxed)
    }

    /// Offers one Annex-B access unit to the decoder.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError::Overloaded`] when the write does not complete in
    /// [`Self::WRITE_TIMEOUT`], and [`DecodeError::Failed`] when the pipe is
    /// gone. Both poison the session; the caller restarts at the next
    /// decodable run rather than feeding a decoder whose input may be
    /// truncated mid-NAL.
    pub async fn feed(&mut self, annex_b: &[u8]) -> Result<(), DecodeError> {
        let budget = if self.accepted_a_write {
            Self::WRITE_TIMEOUT
        } else {
            Self::SPAWN_WRITE_TIMEOUT
        };
        let pipe = self.stdin.as_mut().ok_or(DecodeError::Failed)?;
        match timeout(budget, pipe.write_all(annex_b)).await {
            Ok(Ok(())) => {
                // One completed write proves the decoder reached its input
                // loop. Everything after this is measured against the tight
                // budget, because a stall then is the consumer falling behind.
                self.accepted_a_write = true;
                Ok(())
            }
            Ok(Err(_)) => Err(DecodeError::Failed),
            // `write_all` is not cancellation-safe, so a timeout here may have
            // left part of an access unit in the pipe. The caller must discard
            // the session rather than continue mid-NAL.
            Err(_) => Err(DecodeError::Overloaded),
        }
    }

    /// Closes the decoder and joins both of its tasks.
    ///
    /// Dropping stdin gives `FFmpeg` an ordinary end of input, so it flushes
    /// the pictures it has already decoded and exits on its own. Killing first
    /// instead of waiting discards that flush, which on a short session means
    /// discarding every frame it produced.
    ///
    /// The kill is the backstop for a decoder that does not exit within
    /// [`Self::SHUTDOWN_GRACE`], so this call is bounded whatever the child
    /// does. Neither task can outlive it.
    pub async fn shutdown(mut self) {
        drop(self.stdin.take());
        if timeout(Self::SHUTDOWN_GRACE, self.child.wait())
            .await
            .is_err()
        {
            let _ = self.child.kill().await;
        }
        let _ = self.reader.await;
        let _ = self.stderr.await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Clone, Default)]
    struct CollectingSink(Arc<Mutex<Vec<usize>>>);

    impl DecodedFrameSink for CollectingSink {
        fn accept(&mut self, yuv420p: &[u8], _size: FrameSize) {
            if let Ok(mut frames) = self.0.lock() {
                frames.push(yuv420p.len());
            }
        }
    }

    #[test]
    fn frame_size_rounds_to_even_and_computes_exact_i420_length() {
        let size = FrameSize::new(1921, 1081).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!((size.width(), size.height()), (1920, 1080));
        // Luma plus two quarter-size chroma planes, with no rounding slack.
        assert_eq!(size.frame_bytes(), 1920 * 1080 * 3 / 2);
    }

    #[test]
    fn frame_size_rejects_degenerate_and_implausible_geometry() {
        assert_eq!(FrameSize::new(0, 720), Err(DecodeError::Geometry));
        assert_eq!(FrameSize::new(1, 720), Err(DecodeError::Geometry));
        assert_eq!(FrameSize::new(1280, 0), Err(DecodeError::Geometry));
        assert_eq!(FrameSize::new(16384, 720), Err(DecodeError::Geometry));
    }

    #[test]
    fn capping_preserves_aspect_ratio_and_never_scales_up() {
        let uhd = FrameSize::new(3840, 2160).unwrap_or_else(|error| panic!("{error}"));
        let capped = uhd
            .capped_to_height(720)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!((capped.width(), capped.height()), (1280, 720));

        let vga = FrameSize::new(640, 480).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            vga.capped_to_height(1080)
                .unwrap_or_else(|error| panic!("{error}")),
            vga,
            "a stream smaller than the cap must be published untouched"
        );
        assert_eq!(
            vga.capped_to_height(0)
                .unwrap_or_else(|error| panic!("{error}")),
            vga,
            "a zero cap means no cap, not a zero-sized frame"
        );
    }

    #[test]
    fn capping_an_extreme_aspect_ratio_stays_representable() {
        // A 32:9 panorama capped hard: the derived width must still round to a
        // legal even value rather than collapsing to zero.
        let wide = FrameSize::new(5120, 1440).unwrap_or_else(|error| panic!("{error}"));
        let capped = wide
            .capped_to_height(144)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!((capped.width(), capped.height()), (512, 144));
    }

    /// Costs [`H264ToI420::SPAWN_WRITE_TIMEOUT`] of real time, deliberately.
    ///
    /// A paused clock would make it instant but needs tokio's `test-util`
    /// feature, which unifies across the workspace and would change how every
    /// other crate's runtime is built. Two seconds is the cheaper price.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_cold_decoder_gets_the_startup_budget_for_its_first_write() {
        // The failure this pins down: with one 250 ms budget, every subscribe
        // began with an `Overloaded` restart while FFmpeg was still starting,
        // costing the viewer a keyframe interval of black for a stall that was
        // never backpressure.
        assert!(
            H264ToI420::WRITE_TIMEOUT < H264ToI420::SPAWN_WRITE_TIMEOUT,
            "a cold decoder must be given more time than a warm one"
        );

        // `yes` never reads its stdin and ignores the arguments it is handed, so
        // the pipe fills and stays full. That is precisely a decoder that has
        // not reached its input loop.
        let mut decoder = H264ToI420::spawn(
            "yes",
            FrameSize::new(320, 240).unwrap_or_else(|error| panic!("{error}")),
            CollectingSink(Arc::new(Mutex::new(Vec::new()))),
            Arc::new(DecodeCounters::default()),
        )
        .unwrap_or_else(|error| panic!("{error}"));

        // Comfortably past any platform's pipe capacity, so the write cannot
        // complete into the buffer alone.
        let oversized = vec![0_u8; 4 << 20];
        let started = std::time::Instant::now();
        let outcome = decoder.feed(&oversized).await;
        let waited = started.elapsed();

        assert!(
            matches!(outcome, Err(DecodeError::Overloaded)),
            "a decoder that never reads is an overload, not a broken pipe"
        );
        assert!(
            waited >= H264ToI420::SPAWN_WRITE_TIMEOUT,
            "the first write was cut off after {waited:?}, which is the warm \
             budget rather than the startup budget"
        );
        decoder.shutdown().await;
    }

    #[tokio::test]
    async fn a_missing_decoder_binary_is_unavailable_rather_than_a_failure() {
        // Not retryable: the binary will still be absent on the next attempt,
        // and a reconnect loop over a missing dependency is noise.
        let error = H264ToI420::spawn(
            "surveillance-nonexistent-decoder",
            FrameSize::new(320, 240).unwrap_or_else(|error| panic!("{error}")),
            CollectingSink::default(),
            Arc::new(DecodeCounters::default()),
        )
        .err()
        .unwrap_or_else(|| panic!("spawning a nonexistent binary must fail"));
        assert_eq!(error, DecodeError::Unavailable);
    }

    #[tokio::test]
    #[ignore = "needs ffmpeg with an H.264 decoder"]
    async fn decoding_a_synthetic_stream_yields_fixed_length_i420_frames() {
        let size = FrameSize::new(320, 240).unwrap_or_else(|error| panic!("{error}"));
        let sink = CollectingSink::default();
        let frames = Arc::clone(&sink.0);

        // Produce a short real H.264 elementary stream with the same tool that
        // will decode it, so the test exercises actual media rather than a
        // handcrafted buffer.
        let encoded = std::process::Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=320x240:rate=10:duration=1",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-bsf:v",
                "h264_mp4toannexb",
                "-f",
                "h264",
                "pipe:1",
            ])
            .output()
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(!encoded.stdout.is_empty(), "fixture produced no media");

        let mut decoder =
            H264ToI420::spawn("ffmpeg", size, sink, Arc::new(DecodeCounters::default()))
                .unwrap_or_else(|error| panic!("{error}"));
        decoder
            .feed(&encoded.stdout)
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        decoder.shutdown().await;

        let observed = frames.lock().unwrap_or_else(|error| panic!("{error}"));
        assert!(!observed.is_empty(), "no frames were decoded");
        assert!(
            observed.iter().all(|length| *length == size.frame_bytes()),
            "every frame must be exactly one I420 picture: {observed:?}"
        );
    }
}
