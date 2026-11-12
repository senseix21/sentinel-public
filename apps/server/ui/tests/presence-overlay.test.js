import assert from "node:assert/strict";
import test from "node:test";
import {DetectionOverlay} from "../js/detection-overlay.js";
const presence=(sequence="1",extra={})=>({camera_id:"cam",epoch:"1",session:"1",sequence,available:true,saturated:false,results_are_fixture:false,observed:1,uncertain:0,tentative:0,...extra});
const faces=(sequence="1",detections=[])=>({camera_id:"cam",epoch:"1",session:"1",sequence,results_are_fixture:false,detections});
const overlay=()=>new DetectionOverlay({mode:"person_face",cameraId:"cam",maxAgeMs:1000});
test("turning away clears face boxes while independently observed presence remains",()=>{
 const o=overlay();o.onPresencePacket(presence());o.onFacePacket(faces("1",[{left:0.1,top:0.1,width:0.1,height:0.1,confidence:0.7}]));
 assert.equal(o.personCounts().active,1);assert.equal(o.personCounts().faces,1);
 o.onFacePacket(faces("2"));assert.equal(o.personCounts().faces,0);assert.equal(o.personCounts().active,1);
 o.onPacket({detections:[{left:0,top:0,width:1,height:1,confidence:1,track_id:8}]});assert.equal(o.personCounts().faces,0);
});
test("uncertainty, unknown and fresh zero are different",()=>{
 const o=overlay();let now=0;o._setClock(()=>now);
 o.onPresencePacket(presence("1",{observed:0,uncertain:1}));assert.equal(o.personCounts().uncertain,1);assert.equal(o.personCounts().available,true);
 o.onPresencePacket(presence("2",{observed:0}));assert.equal(o.personCounts().available,true);assert.equal(o.personCounts().active,0);
 now=1001;assert.equal(o.personCounts().available,false);
 o.onPresencePacket(presence("3",{available:false,observed:0}));assert.equal(o.personCounts().available,false);
});
test("malformed, old-session and fixture presence cannot become fresh real presence",()=>{
 const o=overlay();
 for(const x of [{observed:65},{observed:64,uncertain:1},{observed:-1},{available:false},{sequence:1},{session:"0"},{camera_id:"other"},{available:"true"}])assert.equal(o.onPresencePacket(presence("1",x)),false);
 assert.equal(o.onPresencePacket(presence("2")),true);assert.equal(o.onPresencePacket(presence("1")),false);
 assert.equal(o.onPresencePacket(presence("1",{session:"2"})),true);assert.equal(o.onPresencePacket(presence("99")),false);
 assert.equal(o.onPresencePacket(presence("2",{session:"2",results_are_fixture:true})),true);assert.equal(o.personCounts().available,false);
});
test("face failure leaves person presence; person failure leaves faces",()=>{
 const o=overlay();o.onPresencePacket(presence());o.onFacePacket(faces("1",[{left:0.1,top:0.1,width:0.1,height:0.1,confidence:0.9}]));
 o.setUnavailable(true);assert.equal(o.personCounts().active,1);assert.equal(o.personCounts().faces,0);
 o.setUnavailable(false);o.onFacePacket(faces("2",[{left:0.1,top:0.1,width:0.1,height:0.1,confidence:0.9}]));o.setPersonUnavailable(true);assert.equal(o.personCounts().available,false);assert.equal(o.personCounts().faces,1);
 o.updateConfiguration("face",1000);o.updateConfiguration("person_face",1000);assert.equal(o.personCounts().received,false);
});
test("old session faces cannot return after presence restarts; invalid packets cannot reset ordering",()=>{
 const o=overlay();o.onPresencePacket(presence("5"));o.onFacePacket(faces("6",[{left:0.1,top:0.1,width:0.1,height:0.1,confidence:0.9}]));
 assert.equal(o.onFacePacket({...faces("1"),session:"2",detections:[{left:NaN,top:0,width:1,height:1,confidence:1}]}),false);
 assert.equal(o.onFacePacket(faces("2")),false);
 o.onPresencePacket(presence("7",{session:"2"}));assert.equal(o.personCounts().faces,0);
 assert.equal(o.onFacePacket(faces("99")),false);
 assert.equal(o.onFacePacket({...faces("1"),session:"2"}),true);
});
