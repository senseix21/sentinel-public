import { esc, icon } from "../formatting.js";

export function createCameraTile({ cameraLabel, pipelinePill, recordingDot, livekitChip }) {
function cameraTile(camera, options) {
  const id = esc(camera.camera_id);
  const canWatch = camera.live_publication_enabled;
  // The track is attached by the LiveKit owner. Muted autoplay is explicit so
  // browsers can begin playback without an operator gesture.
  // `data-live-watchable` is what "Watch all" filters on. Without it that button
  // joins a room for a camera whose own tile offers no watch button, because
  // publication is disabled — the page would contradict itself in one press.
  const watchable = canWatch ? " data-live-watchable" : "";
  const element = options && options.primary
    ? `<video id="live" data-live-tile="${id}"${watchable} muted autoplay playsinline hidden></video>`
    : `<video data-live-tile="${id}"${watchable} muted autoplay playsinline hidden></video>`;
  return `<div class="tile">
      <div class="tile-head">
        <b>${cameraLabel(camera)}</b>
        <span class="sp"></span>
        ${pipelinePill(camera)}
      </div>
      <div class="stage">
        ${element}
        <div class="idle" data-live-idle="${id}" >
          ${canWatch ? "" : `${icon("cam", "lg")}<div>This camera does not publish to LiveKit.</div>`}
        </div>
      </div>
      <div class="tile-foot">
        ${recordingDot(camera)}
        <span class="sp"></span>
        ${livekitChip(camera)}
        ${canWatch ? `<button class="btn icon" data-act="watch-live" data-camera="${id}" title="Watch live">${icon("play")}</button>
        <button class="btn icon" data-act="stop-live" data-camera="${id}" title="Stop">${icon("stop")}</button>` : ""}
        <button class="btn icon" data-act="full" data-camera="${id}" title="Fullscreen">${icon("expand")}</button>
      </div>
      <div class="tile-note" data-live-note="${id}"></div>
    </div>`;
}

// ------------------------------------------------------------------ screens
  return cameraTile;
}
