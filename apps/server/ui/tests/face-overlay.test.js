import assert from "node:assert/strict";
import test from "node:test";
import { DetectionOverlay } from "../js/detection-overlay.js";
const face = { left: 0.2, top: 0.1, width: 0.15, height: 0.2, confidence: 0.7 };
const packet = (sequence = "1", extra = {}) => ({ camera_id: "camera-01", epoch: "1", sequence, results_are_fixture: false, detections: [face], ...extra });
const overlay = () => new DetectionOverlay({ mode: "face", cameraId: "camera-01" });
test("face mode draws only face observations and counts faces, not unique people", () => {
  const o = overlay();
  o.onPacket({ detections: [{...face, track_id: 4}] });
  assert.equal(o.personCounts().active, 0);
  assert.equal(o.onFacePacket(packet()), true);
  assert.deepEqual(o.personCounts(), { mode: "face", active: 1, cumulative: 0, received: true });
  assert.equal(o.onFacePacket(packet("2", { detections: [] })), true);
  assert.equal(o.personCounts().active, 0);
});
test("wrong camera, malformed geometry, track labels and count overflow are rejected", () => {
  for (const extra of [ {camera_id:"other"}, {epoch:1}, {sequence:"0"}, {results_are_fixture: "false"},
    {detections:[{...face,left:NaN}]}, {detections:[{...face,width:0}]},
    {detections:[{...face,top:0.95}]}, {detections:[{...face,confidence:1.1}]},
    {detections:[{...face,track_id:2}]}, {detections:Array(65).fill(face)} ]) {
    assert.equal(overlay().onFacePacket(packet("1",extra)), false);
  }
});
test("old sequence and epoch cannot restore boxes after clearing", () => {
  const o=overlay();
  o.onFacePacket(packet("3"));
  o.onFacePacket(packet("4", {detections:[]}));
  assert.equal(o.onFacePacket(packet("2")),false);
  assert.equal(o.personCounts().active,0);
  assert.equal(o.onFacePacket(packet("1", {epoch:"2"})),true);
  assert.equal(o.onFacePacket(packet("99")),false);
});
test("fixture and expired face packets produce no visible count", () => {
  const o=overlay(); let now=100; o._setClock(()=>now);
  o.onFacePacket(packet("1", {results_are_fixture:true})); assert.equal(o.personCounts().active,0);
  o.onFacePacket(packet("2")); now+=3001; assert.equal(o.personCounts().active,0);
  o.detach(); assert.equal(o.personCounts().received,false);
});
test("person mode refuses face packets", () => {
  assert.equal(new DetectionOverlay().onFacePacket(packet()),false);
});

test("face boxes map through video bars and DPR without identity labels", () => {
  const original=globalThis.window; globalThis.window={devicePixelRatio:2};
  try {
    const o=overlay(), strokes=[];
    o._video={clientWidth:640,clientHeight:480,videoWidth:1280,videoHeight:720};
    const ctx={clearRect(){},strokeRect(...rect){strokes.push(rect)},fillText(){assert.fail("face boxes have no identity labels")}};
    o._canvas={width:1280,height:960,getContext:()=>ctx};
    o.onFacePacket(packet()); o._render();
    assert.deepEqual(strokes,[[256,192,192,144]]);
    o.setUnavailable(true); assert.equal(o.onFacePacket(packet("2")),false);
    assert.equal(o.personCounts().active,0);
  } finally { globalThis.window=original; }
});

test("configured expiry and mode changes replace old overlay state", () => {
  const o=overlay(); let now=0; o._setClock(()=>now); o.onFacePacket(packet());
  o.updateConfiguration("face",1000); now=1001; assert.equal(o.personCounts().active,0);
  o.updateConfiguration("person",3000); assert.equal(o.onFacePacket(packet("2")),false);
  o.updateConfiguration("face",3000); assert.equal(o.personCounts().received,false);
  assert.equal(o.onFacePacket(packet("1")),true);
});
