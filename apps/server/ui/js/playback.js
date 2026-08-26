export function createPlaybackState() {
  return { camera: "", date: "", from: "00:00:00", to: "01:00:00", result: null, error: null, busy: false, selected: null };
}

export function searchRange(state) {
  return {
    start: `${state.date}T${state.from}Z`,
    end: `${state.date}T${state.to}Z`,
  };
}

export function recordingSearchPath(cameraId, range) {
  return `/api/cameras/${encodeURIComponent(cameraId)}/recordings`
    + `?start=${encodeURIComponent(range.start)}&end=${encodeURIComponent(range.end)}`;
}

export function playbackPath(cameraId, start, end) {
  return `/api/cameras/${encodeURIComponent(cameraId)}/recordings/playback`
    + `?start=${encodeURIComponent(start)}&end=${encodeURIComponent(end)}`;
}

export function startPlayback(video, url) {
  if (!video) return Promise.resolve();
  video.pause();
  video.setAttribute("src", url);
  // Assigning `src` queues resource selection, but explicitly loading here
  // makes the button deterministic when it replaces a stopped or failed
  // stream whose element has already been reset with `load()`.
  video.load();
  return video.play();
}

// Pausing keeps the current media resource so playback can resume. Stopping is
// intentionally stronger: clearing the resource closes the HTTP response and
// lets the server cancel its bounded FFmpeg assembly instead of continuing to
// produce footage nobody is watching.
export function togglePlayback(video) {
  if (!video) return Promise.resolve();
  if (video.paused || video.ended) return video.play();
  video.pause();
  return Promise.resolve();
}

export function stopPlayback(video) {
  if (!video) return;
  video.pause();
  video.removeAttribute("src");
  video.load();
}

export function contiguousRuns(recordings) {
  const sorted = [...recordings].sort((a, b) => (a.started_at < b.started_at ? -1 : 1));
  const runs = [];
  let current = null;
  for (const recording of sorted) {
    if (current && current.ended_at === recording.started_at) {
      current.ended_at = recording.ended_at;
      current.count += 1;
      current.bytes += Number(recording.size_bytes || 0);
      continue;
    }
    current = {
      started_at: recording.started_at,
      ended_at: recording.ended_at,
      count: 1,
      bytes: Number(recording.size_bytes || 0),
    };
    runs.push(current);
  }
  return runs;
}

export function coverageBuckets(recordings, bucketCount = 96) {
  if (!recordings.length) return null;
  const first = Math.min(...recordings.map((recording) => Date.parse(recording.started_at)));
  const last = Math.max(...recordings.map((recording) => Date.parse(recording.ended_at)));
  const span = last - first || 1;
  const filled = new Array(bucketCount).fill(false);
  for (const recording of recordings) {
    const from = Math.floor(((Date.parse(recording.started_at) - first) / span) * bucketCount);
    const to = Math.ceil(((Date.parse(recording.ended_at) - first) / span) * bucketCount);
    for (let index = Math.max(0, from); index < Math.min(bucketCount, Math.max(to, from + 1)); index += 1) {
      filled[index] = true;
    }
  }
  return { first, last, filled };
}
