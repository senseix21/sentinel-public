// Real RTSP -> model -> LiveKit -> browser qualification with decoded scene markers.
const assert=require('node:assert/strict');
const {chromium}=require(process.env.PLAYWRIGHT_MODULE_PATH || 'playwright');
const base=process.env.SENTINEL_BROWSER_URL || 'http://127.0.0.1:3001';
(async()=>{
 const browser=await chromium.launch({headless:true,channel:'chromium'});
 try {
  const page=await browser.newPage();const errors=[];page.on('pageerror',error=>errors.push(error.message));
  await page.goto(base);await page.locator('[name=username]').fill(process.env.SENTINEL_DEV_USERNAME || 'operator');await page.locator('[name=password]').fill(process.env.SENTINEL_DEV_PASSWORD || 'sentinel-local');await page.locator('button[type=submit]').click();await page.waitForURL(url=>url.pathname==='/');
  await page.evaluate(async()=>{
   const {DetectionOverlay}=await import('/ui/js/detection-overlay.js');
   for(const method of ['onFacePacket','onPresencePacket']) {
    const original=DetectionOverlay.prototype[method];
    DetectionOverlay.prototype[method]=function(packet){const accepted=original.call(this,packet);if(accepted)window.qualificationOverlay=this;return accepted;};
   }
  });
  await page.goto(`${base}/#live`);
  // Navigation to the same document's hash retains the installed instrumentation.
  await page.waitForFunction(()=>document.querySelector('[data-live-tile]')?.readyState>=2,null,{timeout:45000});
  const evidence={};let stable=null,stableAt=Date.now();
  const until=Date.now()+40000;
  while(Date.now()<until && Object.keys(evidence).length<3) {
   const sample=await page.evaluate(()=>{
    const video=document.querySelector('[data-live-tile]');const o=window.qualificationOverlay;
    if(!video || !o || video.videoWidth!==1280 || video.videoHeight!==720)return null;
    const canvas=document.createElement('canvas');canvas.width=1;canvas.height=1;const ctx=canvas.getContext('2d');ctx.drawImage(video,1264,704,1,1,0,0,1,1);const [red,green,blue]=ctx.getImageData(0,0,1,1).data;
    const stage=red>180 && green<80 && blue<80 ? 'front' : green>180 && red<80 && blue<80 ? 'back' : blue>180 && red<80 && green<80 ? 'empty' : null;
    const counts=o.personCounts();
    return {stage,counts,facePacketReal:!!o._lastPacket && !o.isFixture() && Date.now()-o._lastReceivedAt<=1000,readout:document.querySelector('.person-count')?.textContent,time:video.currentTime};
   });
   if(sample?.stage!==stable){stable=sample?.stage;stableAt=Date.now();}
   if(stable && Date.now()-stableAt>2000 && sample.counts.available && sample.facePacketReal) {
    const c=sample.counts;
    const correct=stable==='front' ? c.active>0 && c.faces>0 : stable==='back' ? c.active>0 && c.faces===0 : c.active===0 && c.uncertain===0 && c.faces===0;
    if(correct)evidence[stable]=sample;
   }
   await page.waitForTimeout(200);
  }
  assert.equal(errors.length,0,errors.join('; '));
  assert.deepEqual(Object.keys(evidence).sort(),['back','empty','front'],'did not prove all three decoded scenes with fresh real results');
  assert.ok(evidence.back.readout.startsWith('People observed:'),'back-facing readout absent');
  assert.ok(evidence.empty.time>0,'video did not play');
  console.log(JSON.stringify({result:'passed',evidence}));
 }finally{await browser.close();}
})().catch(error=>{console.error(error.message);process.exitCode=1;});
