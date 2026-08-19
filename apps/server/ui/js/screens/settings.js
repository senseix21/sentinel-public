import { banner } from "../components/banner.js";
import { absent } from "../components/empty-state.js";
import { esc, icon } from "../formatting.js";

export function createSettingsScreen({ state, errorBanner }) {
  const edits = new Map();
  
  function settingValue(key) {
    if (!state.settings) return null;
    const found = state.settings.settings.find((entry) => entry.key === key);
    return found ? found.value : null;
  }
  
  function screenSettings() {
    if (!state.settings) {
      // A configuration surface that is not there must say so. Leaving a spinner
      // on screen would read as slow rather than as absent, and this endpoint
      // genuinely answers `503` when no settings store is configured.
      return `<div class="card">
          <h3>${icon("gear")} Configuration</h3>
          ${state.settingsError
            ? errorBanner(state.settingsError)
            : `<div class="muted">Loading configuration…</div>`}
          ${state.settingsError && state.settingsError.status === 503
            ? `<div class="tiny muted mt">Persistent configuration needs a database. Without one the server runs from its bootstrap layer, which this page cannot edit.</div>`
            : ""}
          <div class="row mt"><button class="btn" data-act="settings-reload">${icon("refresh")} Try again</button></div>
        </div>`;
    }
    const groups = new Map();
    for (const entry of state.settings.settings) {
      const group = entry.key.split(".")[0];
      if (!groups.has(group)) groups.set(group, []);
      groups.get(group).push(entry);
    }
  
    const stale = state.settings.freshness === "stale";
    return `${stale ? banner("warn", "The configuration snapshot is stale.",
        "The server is serving its last known good configuration because the store is unreachable. Changes cannot be applied while this is true.") : ""}
      <div class="row" style="justify-content:space-between;margin-bottom:14px">
        <div class="muted tiny">
          Generation <b>${esc(state.settings.generation)}</b> &middot; cache <b>${esc(state.settings.cache)}</b> &middot;
          ${esc(state.settings.settings.length)} settings
        </div>
        <div class="muted tiny">Changes are applied in one transaction, validated as a group, and audited in the same transaction.</div>
      </div>
      ${banner("info", "Secrets are not readable or writable from this page.",
        "Credentials render as <code>***</code> and are managed in the secret store or by environment override. " +
        "This page holds no credential of any kind, which is why there is no reveal control and no login here.")}
      <div class="grid cols-2">
        ${[...groups.entries()].map(([group, entries]) => settingsGroup(group, entries)).join("")}
      </div>
      ${saveBar()}
      <div id="settings-out"></div>
      <div class="card mt">
        ${absent("There is no message-broker configuration",
          "The reference design configures NATS, Redis Streams or Kafka as an event backend, with an endpoint, credentials and per-type topic names. This system has no external broker and is not going to grow one from a settings page.",
          ["The event bus is bounded and in-process by design; ADR 002 records that persistence and replay belong to explicit adapters.",
           "The engineering policy forbids introducing Kafka or NATS without a measured constraint and an architecture decision record.",
           "An external broker changes delivery, ordering, persistence and failure semantics — it is not a backend toggle."])}
      </div>`;
  }
  
  function settingsGroup(group, entries) {
    return `<div class="card">
        <h3>${icon("gear")} ${esc(group)}</h3>
        ${entries.map((entry) => settingRow(entry)).join("")}
      </div>`;
  }
  
  function settingRow(entry) {
    const editable = entry.class === "operational" && !entry.secret;
    const dirty = edits.has(entry.key);
    const shown = dirty ? edits.get(entry.key) : entry.value;
    const tail = entry.key.split(".").slice(1).join(".") || entry.key;
    return `<div class="srow ${dirty ? "dirty" : ""}">
        <div class="key">
          ${esc(tail)}
          <small>${esc(entry.origin)} &middot; ${esc(entry.boundary)}${entry.secret ? " &middot; secret" : ""}${editable ? "" : " &middot; read-only"}</small>
        </div>
        <div>
          <input class="f" data-setting="${esc(entry.key)}" value="${esc(shown)}" ${editable ? "" : "disabled"}>
        </div>
        <div>${dirty ? `<button class="btn icon" data-act="setting-revert" data-key="${esc(entry.key)}" title="Revert">${icon("refresh")}</button>` : ""}</div>
      </div>`;
  }
  
  function saveBar() {
    const count = edits.size;
    return `<div class="savebar">
        <span class="${count ? "warn" : "muted"}">${count
          ? `${icon("alert")} ${count} unsaved change${count === 1 ? "" : "s"}`
          : `${icon("check")} No unsaved changes`}</span>
        <span class="sp"></span>
        <button class="btn" data-act="settings-discard" ${count ? "" : "disabled"}>Discard changes</button>
        <button class="btn primary" data-act="settings-save" ${count ? "" : "disabled"}>${icon("check")} Save settings</button>
      </div>`;
  }

  return { edits, settingValue, saveBar, render: screenSettings };
}
