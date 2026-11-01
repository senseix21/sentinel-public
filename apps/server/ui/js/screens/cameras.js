import { banner } from "../components/banner.js";
import { ago, bytes, esc, icon, utc } from "../formatting.js";

export function createCamerasScreen({
  add, api, cameraTile, detectionIsFixture, errorBanner, fleetCounts, fleetProblem,
  livekitChip, pipelinePill, route, render,
}) {
  function screenCameras(args) {
    if (args.length) return cameraDetail(args[0]);
    const counts = fleetCounts();
    return `${fleetProblem()}
      <div class="card">
        <h3>${icon("cam")} Fleet <span class="right">${counts.readable ? `${counts.connected} of ${counts.total} connected` : "fleet not readable"}</span></h3>
        <div class="wrap"><table>
          <thead><tr>
            <th>Camera</th><th>Pipeline</th><th>Connection</th><th class="num">Attempts</th>
            <th class="num">Reconnects</th><th class="num">Segments</th><th>LiveKit</th><th>Last media</th><th></th>
          </tr></thead>
          <tbody>${counts.cameras.map((camera) => `<tr>
            <td><a href="#/cameras/${encodeURIComponent(camera.camera_id)}">${esc(camera.camera_id)}</a></td>
            <td>${pipelinePill(camera)}</td>
            <td>${esc(camera.connection_state || "—")}</td>
            <td class="num">${esc(camera.connection_attempts)}</td>
            <td class="num ${camera.reconnects ? "warn" : ""}">${esc(camera.reconnects)}</td>
            <td class="num">${esc(camera.recorder_completed_segments)}</td>
            <td>${livekitChip(camera)}</td>
            <td class="num muted">${esc(ago(camera.last_media_unix_ms))}</td>
            <td class="row" style="gap:6px">
              <button class="btn icon" data-act="camera-start" data-camera="${esc(camera.camera_id)}" title="Start">${icon("play")}</button>
              <button class="btn icon" data-act="camera-stop" data-camera="${esc(camera.camera_id)}" title="Stop">${icon("stop")}</button>
            </td>
          </tr>`).join("")}</tbody>
        </table></div>
        ${counts.total ? "" : `<div class="muted mt">${counts.readable
          ? "No cameras are configured."
          : "The fleet could not be read, so this table is empty for a reason unrelated to how many cameras exist."}</div>`}
      </div>
  
      <div class="card mt">
        <h3>${icon("plus")} Add a camera</h3>
        <p class="note">Saves a camera on this server, initially stopped. Press Start to connect; running cameras resume after a server restart.</p>
        <div class="filters">
          <div class="field"><label class="f">Camera identifier</label><input class="f" id="add-camera-id" data-add="id" value="${esc(add.id)}" placeholder="camera-02" autocomplete="off"></div>
          <div class="field grow"><label class="f">RTSP endpoint</label><input class="f" id="add-camera-rtsp" data-add="rtsp" value="${esc(add.rtsp)}" placeholder="rtsp://192.168.1.20:554/stream1" autocomplete="off"></div>
          <div class="field"><label class="f">Camera username (optional)</label><input class="f" data-add="username" value="${esc(add.username || "")}" autocomplete="off"></div>
          <div class="field"><label class="f">Camera password (optional)</label><input class="f" type="password" data-add="password" value="${esc(add.password || "")}" autocomplete="new-password"></div>
          <div class="field"><label class="f">Detection</label><select class="f" data-add="detection">
            <option value="off" ${add.detection !== "on" ? "selected" : ""}>Disabled</option>
            <option value="on" ${add.detection === "on" ? "selected" : ""}>Enabled (requires configured backend)</option>
          </select></div>
          <div class="field"><label class="f">Width</label><input class="f" id="add-camera-width" data-add="width" type="number" min="1" value="${esc(add.width)}"></div>
          <div class="field"><label class="f">Height</label><input class="f" id="add-camera-height" data-add="height" type="number" min="1" value="${esc(add.height)}"></div>
        </div>
        <div class="filters">
          <div class="field"><label class="f">Live publication</label>
            <select class="f" id="add-camera-live" data-add="live">
              <option value="default" ${add.live === "default" ? "selected" : ""}>Same as this deployment</option>
              <option value="on" ${add.live === "on" ? "selected" : ""}>Publish to LiveKit</option>
              <option value="off" ${add.live === "off" ? "selected" : ""}>Do not publish</option>
            </select></div>
          <div class="row"><button class="btn primary" data-act="camera-add">${icon("plus")} Add camera</button></div>
        </div>
        <p class="note">Enter credentials separately from the RTSP URL. They are saved in a protected server file and are never returned by the API. Fields clear after a successful add. The dev server must be able to reach the camera's network address.</p>
        <p class="note">Live publication carries no endpoint or credential from this page. The server holds one LiveKit deployment and stamps it onto the camera; leaving this on <em>Same as this deployment</em> publishes when the server publishes. Choosing <em>Publish to LiveKit</em> on a server with none configured is refused rather than quietly ignored.</p>
        <div id="add-camera-out"></div>
      </div>`;
  }
  
  // A camera the server will not describe is reported as absent, in the same
  // words for every reason. The server answers `404` identically for one that was
  // never configured and one this caller may not see, so that a viewer cannot
  // enumerate the estate by reading error messages. Explaining the second case
  // here would rebuild exactly the oracle the server removed.
  function missingCamera(cameraId) {
    return `<div class="card">
        ${banner("warn", "No such camera.", `The server does not describe <code>${esc(cameraId)}</code>.`)}
        <div class="row mt"><a class="btn" href="#/cameras">${icon("left")} Back to the fleet</a></div>
      </div>`;
  }
  
  const detail = { id: null, camera: null, error: null };
  
  function cameraDetail(cameraId) {
    if (detail.id !== cameraId) {
      detail.id = cameraId;
      detail.camera = null;
      detail.error = null;
      loadCamera(cameraId);
      return `<div class="card"><div class="muted">Loading ${esc(cameraId)}…</div></div>`;
    }
    if (detail.error) {
      if (detail.error.status === 404) return missingCamera(cameraId);
      return `<div class="card">${errorBanner(detail.error)}</div>`;
    }
    if (!detail.camera) return `<div class="card"><div class="muted">Loading ${esc(cameraId)}…</div></div>`;
  
    const camera = detail.camera;
    const fixture = detectionIsFixture();
  
    return `<div class="row" style="margin-bottom:14px">
        <a class="btn" href="#/cameras">${icon("left")} Fleet</a>
        <h2 style="margin:0;font-size:17px">${esc(camera.camera_id)}</h2>
        ${pipelinePill(camera)}
      </div>
      <div class="split-wide">
        <div class="stack">
          <div class="card">
            <h3>${icon("live")} Live</h3>
            <div class="tiles" style="grid-template-columns:minmax(0,1fr)">
              ${cameraTile(camera, { primary: true })}
            </div>
          </div>
          <div class="card">
            <h3>${icon("db")} Recording</h3>
            <div class="kv mt">
              <div class="k">Completed segments</div><div class="v">${esc(camera.recorder_completed_segments)}</div>
              <div class="k">Failed segments</div><div class="v ${camera.recorder_failed_segments ? "bad" : ""}">${esc(camera.recorder_failed_segments)}</div>
              <div class="k">Recorder queue</div><div class="v">${camera.recorder_queue_depth === null ? "—" : `${esc(camera.recorder_queue_depth)} / ${esc(camera.recorder_queue_capacity)}`}</div>
              <div class="k">Rejected (queue full)</div><div class="v ${camera.recorder_rejected ? "bad" : ""}">${esc(camera.recorder_rejected)}</div>
            </div>
            <div class="tiny muted mt">The recorder queue rejects when full, which stops ingest rather than losing footage.</div>
            <div class="row mt"><a class="btn" href="#/playback">${icon("clock")} Search recordings</a></div>
          </div>
        </div>
        <div class="stack">
          <div class="card">
            <h3>${icon("net")} Connection</h3>
            <div class="kv mt">
              <div class="k">State</div><div class="v">${esc(camera.connection_state || "—")}</div>
              <div class="k">Attempts</div><div class="v">${esc(camera.connection_attempts)}</div>
              <div class="k">Connected epochs</div><div class="v">${esc(camera.connection_epochs)}</div>
              <div class="k">Reconnects</div><div class="v ${camera.reconnects ? "warn" : ""}">${esc(camera.reconnects)}</div>
              <div class="k">Terminal failures</div><div class="v ${camera.terminal_failures ? "bad" : ""}">${esc(camera.terminal_failures)}</div>
              <div class="k">Last connected</div><div class="v">${esc(utc(camera.last_connected_unix_ms))}</div>
              <div class="k">Last media</div><div class="v">${esc(utc(camera.last_media_unix_ms))}</div>
            </div>
            ${camera.last_actionable_failure ? `<div class="tiny bad mt">${esc(camera.last_actionable_failure)}</div>` : ""}
          </div>
          <div class="card">
            <h3>${icon("live")} LiveKit</h3>
            <div class="kv mt">
              <div class="k">Enabled</div><div class="v">${camera.live_publication_enabled ? "yes" : "no"}</div>
              <div class="k">State</div><div class="v">${esc(camera.live_publication_state || "—")}</div>
              <div class="k">Access units</div><div class="v">${esc(camera.live_published_access_units)} / ${esc(camera.live_offered)} offered</div>
              <div class="k">Bytes</div><div class="v">${esc(bytes(camera.live_published_bytes))}</div>
              <div class="k">Live queue</div><div class="v">${camera.live_queue_depth === null ? "—" : `${esc(camera.live_queue_depth)} / ${esc(camera.live_queue_capacity)}`}</div>
              <div class="k">Dropped (oldest)</div><div class="v ${camera.live_dropped ? "warn" : ""}">${esc(camera.live_dropped)}</div>
              <div class="k">Declined by transport</div><div class="v ${camera.live_publication_adapter_drops ? "warn" : ""}">${esc(camera.live_publication_adapter_drops)}</div>
            </div>
            ${camera.live_publication_failure ? `<div class="tiny warn mt">${esc(camera.live_publication_failure)}</div>` : ""}
          </div>
          <div class="card">
            <h3>${icon("shield")} Detection</h3>
            ${fixture
              ? banner("warn", "Detection output here is fixture output.",
                "There is no production model in this build. The counts below describe a wired pipeline, not detections of people, and support no accuracy claim.")
              : ""}
            <div class="kv mt">
              <div class="k">Enabled</div><div class="v">${camera.detection_enabled ? "yes" : "no"}</div>
              <div class="k">State</div><div class="v">${esc(camera.detection_state || "—")}</div>
              <div class="k">Inferred</div><div class="v">${esc(camera.detection_inferred)}</div>
              <div class="k">Published</div><div class="v">${esc(camera.detections_published)}</div>
              <div class="k">Dropped</div><div class="v">${esc(camera.detection_dropped)}</div>
            </div>
          </div>
          <div class="card">
            <h3>${icon("gear")} Lifecycle</h3>
            <div class="row mt">
              <button class="btn" data-act="camera-start" data-camera="${esc(camera.camera_id)}">${icon("play")} Start</button>
              <button class="btn" data-act="camera-stop" data-camera="${esc(camera.camera_id)}">${icon("stop")} Stop</button>
              <button class="btn danger" data-act="camera-remove" data-camera="${esc(camera.camera_id)}">${icon("x")} Remove</button>
            </div>
            <div id="lifecycle-out"></div>
          </div>
        </div>
      </div>`;
  }
  
  async function loadCamera(cameraId) {
    try {
      detail.camera = await api(`/api/cameras/${encodeURIComponent(cameraId)}`);
      detail.error = null;
    } catch (error) {
      detail.camera = null;
      detail.error = error;
    }
    if (route().name === "cameras" && route().args[0] === cameraId) await render();
  }

  return { detail, loadCamera, render: screenCameras };
}
