export function createPlaybackState() {
  return {
    camera: "", date: "", from: "00:00:00", to: "01:00:00", result: null,
    error: null, busy: false, selected: null, timezone: "UTC",
    clipDuration: "full", customClipDuration: "15",
  };
}

export function searchRange(state) {
  return {
    start: wallTimeToUtc(state.date, state.from, state.timezone || "UTC"),
    end: wallTimeToUtc(state.date, state.to, state.timezone || "UTC"),
  };
}

export const PLAYBACK_TIMEZONES = [
  { value: "UTC", label: "UTC" },
  { value: "America/New_York", label: "US Eastern" },
  { value: "Asia/Dhaka", label: "Bangladesh" },
];

function wallText(parts) {
  return `${parts.year}-${parts.month}-${parts.day}T${parts.hour}:${parts.minute}:${parts.second}`;
}

function wallParts(instant, timezone) {
  const parts = new Intl.DateTimeFormat("en-CA", {
    timeZone: timezone, hourCycle: "h23", year: "numeric", month: "2-digit",
    day: "2-digit", hour: "2-digit", minute: "2-digit", second: "2-digit",
  }).formatToParts(instant);
  return Object.fromEntries(parts.filter((part) => part.type !== "literal").map((part) => [part.type, part.value]));
}

export function wallTimeToUtc(date, time, timezone) {
  const match = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})$/.exec(`${date}T${time}`);
  if (!match) throw new RangeError("Date and time must be complete.");
  const values = match.slice(1).map(Number);
  const desiredWall = Date.UTC(values[0], values[1] - 1, ...values.slice(2));
  let instant = desiredWall;
  // Re-evaluate because the first estimate can cross a daylight-saving boundary.
  for (let attempt = 0; attempt < 2; attempt += 1) {
    const parts = wallParts(new Date(instant), timezone);
    const observedWall = Date.UTC(
      Number(parts.year), Number(parts.month) - 1, Number(parts.day),
      Number(parts.hour), Number(parts.minute), Number(parts.second),
    );
    instant += desiredWall - observedWall;
  }
  const resolvedText = wallText(wallParts(new Date(instant), timezone));
  if (resolvedText !== `${date}T${time}`) {
    throw new RangeError("That local time does not exist because of a daylight-saving change.");
  }
  // A fall-back wall time happens twice. The fixpoint above converges on one of
  // the two instants, and silently picking it searches an hour of footage the
  // operator did not ask for, with nothing on screen to say so. Offsets shift
  // by 60 minutes almost everywhere and by 30 in Lord Howe, so both deltas are
  // probed in each direction.
  const ambiguous = [-60, -30, 30, 60].some(
    (minutes) => wallText(wallParts(new Date(instant + minutes * 60_000), timezone)) === resolvedText,
  );
  if (ambiguous) {
    throw new RangeError(
      "That local time happens twice because of a daylight-saving change. Search in UTC to choose which one.",
    );
  }
  return new Date(instant).toISOString().replace(".000Z", "Z");
}

export function playbackTime(ms, timezone = "UTC") {
  if (ms === null || ms === undefined || Number.isNaN(Number(ms))) return "—";
  const parts = wallParts(new Date(Number(ms)), timezone);
  return `${parts.year}-${parts.month}-${parts.day} ${parts.hour}:${parts.minute}:${parts.second}`;
}

export function recordingSearchPath(cameraId, range) {
  return `/api/cameras/${encodeURIComponent(cameraId)}/recordings`
    + `?start=${encodeURIComponent(range.start)}&end=${encodeURIComponent(range.end)}`;
}

export function playbackPath(cameraId, start, end) {
  return `/api/cameras/${encodeURIComponent(cameraId)}/recordings/playback`
    + `?start=${encodeURIComponent(start)}&end=${encodeURIComponent(end)}`;
}

export function recordingVideoPath(cameraId, recordingId, durationSeconds = null) {
  const base = `/api/cameras/${encodeURIComponent(cameraId)}/recordings/${encodeURIComponent(recordingId)}/video`;
  if (durationSeconds === null || durationSeconds === "" || durationSeconds === "full") return base;
  const seconds = Number(durationSeconds);
  if (!Number.isInteger(seconds) || seconds < 1 || seconds > 60) {
    throw new RangeError("Duration must be a whole number from 1 to 60 seconds.");
  }
  return `${base}?duration_seconds=${seconds}`;
}

/// Plain operator language for a media failure.
///
/// The server's stable error codes are not reachable from here: the response is
/// consumed by the media element, and re-requesting the URL to read its JSON
/// body would issue a second export -- another audit row for footage nobody
/// watched. So this reports what the element knows, which is still far better
/// than the silence it replaced.
export function mediaErrorMessage(video) {
  const code = video && video.error ? video.error.code : 0;
  if (code === 1) return "Playback was stopped before it finished.";
  if (code === 2) return "The connection dropped while streaming this footage.";
  if (code === 3) return "This footage could not be decoded.";
  if (code === 4) {
    return "The server would not stream this footage. A clip longer than the recording is the usual cause.";
  }
  return "This footage could not be played.";
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
