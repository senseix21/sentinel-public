from pathlib import Path
import sys

sys.path.insert(0, str(Path('.local/presentation-tools').resolve()))

from pptx import Presentation
from pptx.dml.color import RGBColor
from pptx.enum.shapes import MSO_SHAPE, MSO_CONNECTOR
from pptx.enum.text import PP_ALIGN, MSO_ANCHOR
from pptx.util import Inches, Pt


OUT = Path('docs/presentation/SENTINEL-system-overview.pptx')
W, H = Inches(13.333), Inches(7.5)

NAVY = RGBColor(10, 20, 36)
NAVY_2 = RGBColor(16, 31, 53)
CYAN = RGBColor(39, 207, 191)
BLUE = RGBColor(74, 144, 226)
GREEN = RGBColor(74, 201, 126)
AMBER = RGBColor(247, 181, 56)
RED = RGBColor(238, 101, 101)
WHITE = RGBColor(246, 249, 252)
MUTED = RGBColor(167, 181, 198)
INK = RGBColor(32, 45, 61)
PANEL = RGBColor(22, 40, 65)
LIGHT = RGBColor(232, 239, 246)


def box(slide, x, y, w, h, fill=PANEL, radius=True, line=None):
    shp = slide.shapes.add_shape(
        MSO_SHAPE.ROUNDED_RECTANGLE if radius else MSO_SHAPE.RECTANGLE,
        Inches(x), Inches(y), Inches(w), Inches(h)
    )
    shp.fill.solid(); shp.fill.fore_color.rgb = fill
    shp.line.color.rgb = line or fill
    return shp


def text(slide, value, x, y, w, h, size=20, color=WHITE, bold=False,
         align=PP_ALIGN.LEFT, font='Aptos', valign=MSO_ANCHOR.TOP):
    tb = slide.shapes.add_textbox(Inches(x), Inches(y), Inches(w), Inches(h))
    tf = tb.text_frame; tf.clear(); tf.word_wrap = True
    tf.vertical_anchor = valign
    p = tf.paragraphs[0]; p.text = value; p.alignment = align
    p.font.name = font; p.font.size = Pt(size); p.font.bold = bold; p.font.color.rgb = color
    return tb


def bullets(slide, items, x, y, w, h, size=17, color=WHITE, gap=7):
    tb = slide.shapes.add_textbox(Inches(x), Inches(y), Inches(w), Inches(h))
    tf = tb.text_frame; tf.clear(); tf.word_wrap = True
    for i, item in enumerate(items):
        p = tf.paragraphs[0] if i == 0 else tf.add_paragraph()
        p.text = item; p.level = 0; p.font.name = 'Aptos'; p.font.size = Pt(size)
        p.font.color.rgb = color; p.space_after = Pt(gap); p.text = '•  ' + item
    return tb


def base(slide, title, kicker=None, n=None):
    bg = slide.background.fill; bg.solid(); bg.fore_color.rgb = NAVY
    if kicker:
        text(slide, kicker.upper(), .65, .35, 8.5, .28, 10, CYAN, True)
    text(slide, title, .65, .67, 11.9, .55, 28, WHITE, True)
    box(slide, .65, 1.32, 12.0, .03, CYAN, False)
    if n is not None:
        text(slide, f'{n:02d}', 12.25, 7.08, .4, .2, 9, MUTED, False, PP_ALIGN.RIGHT)


def pill(slide, label, x, y, w, color):
    box(slide, x, y, w, .34, color)
    text(slide, label, x, y+.03, w, .22, 10, NAVY, True, PP_ALIGN.CENTER)


def arrow(slide, x1, y1, x2, y2, color=CYAN, width=2.5):
    ln = slide.shapes.add_connector(MSO_CONNECTOR.STRAIGHT, Inches(x1), Inches(y1), Inches(x2), Inches(y2))
    ln.line.color.rgb = color; ln.line.width = Pt(width); ln.line.end_arrowhead = True
    return ln


def card(slide, title_, body, x, y, w, h, accent=CYAN, status=None):
    box(slide, x, y, w, h, PANEL)
    box(slide, x, y, .06, h, accent, False)
    text(slide, title_, x+.22, y+.18, w-.38, .32, 17, WHITE, True)
    text(slide, body, x+.22, y+.62, w-.4, h-.75, 12.5, MUTED)
    if status:
        pill(slide, status, x+w-1.15, y+.15, .95, accent)


prs = Presentation(); prs.slide_width = W; prs.slide_height = H
blank = prs.slide_layouts[6]

# 1 — title
s = prs.slides.add_slide(blank); s.background.fill.solid(); s.background.fill.fore_color.rgb = NAVY
box(s, .65, .55, .09, 6.3, CYAN, False)
text(s, 'SENTINEL', 1.05, 1.05, 7.8, .8, 42, WHITE, True)
text(s, 'Self-hosted multi-camera surveillance\nand video analytics', 1.05, 1.92, 8.9, 1.15, 27, LIGHT, False)
text(s, 'System overview • architecture • delivered capabilities • gaps', 1.08, 3.35, 8.7, .45, 16, MUTED)
box(s, 9.95, .9, 2.4, 4.9, PANEL)
for i, (label, col) in enumerate([('INGEST', CYAN), ('RECORD', GREEN), ('VIEW', BLUE), ('DETECT', AMBER)]):
    box(s, 10.35, 1.35+i*.92, 1.6, .56, col)
    text(s, label, 10.35, 1.51+i*.92, 1.6, .22, 13, NAVY, True, PP_ALIGN.CENTER)
text(s, 'Rust • RTSP • PostgreSQL • LiveKit/WebRTC', 1.05, 6.45, 8.2, .3, 13, MUTED)
text(s, '26 Aug 2026', 10.2, 6.45, 2.15, .3, 13, MUTED, False, PP_ALIGN.RIGHT)

# 2
s = prs.slides.add_slide(blank); base(s, 'What SENTINEL is', 'The product', 2)
text(s, 'A local-first surveillance control plane that keeps video ownership explicit.', .72, 1.58, 11.7, .45, 21, LIGHT, True)
card(s, 'One ingest per camera', 'A single authenticated RTSP/TCP session feeds every downstream feature. Adding a consumer does not add a camera connection.', .72, 2.25, 3.78, 2.15, CYAN)
card(s, 'Record, view, detect', 'Original encoded media is preserved for recording; live viewing uses LiveKit/WebRTC; detection is sampled and independently degradable.', 4.78, 2.25, 3.78, 2.15, BLUE)
card(s, 'Operate with evidence', 'Bounded queues, typed lifecycle, health, metrics, reconciliation, audit, and explicit failure isolation make degradation diagnosable.', 8.84, 2.25, 3.78, 2.15, GREEN)
text(s, 'First target', .75, 5.0, 2.0, .3, 12, MUTED, True)
text(s, '10 cameras / one machine', .75, 5.34, 3.1, .42, 22, WHITE, True)
text(s, 'Next target', 4.75, 5.0, 2.0, .3, 12, MUTED, True)
text(s, '20 cameras, measured', 4.75, 5.34, 3.1, .42, 22, WHITE, True)
text(s, 'Longer path', 8.75, 5.0, 2.0, .3, 12, MUTED, True)
text(s, '100–200+ without premature distribution', 8.75, 5.34, 3.55, .7, 18, WHITE, True)

# 3 architecture
s = prs.slides.add_slide(blank); base(s, 'End-to-end architecture', 'How it works', 3)
pill(s, 'MEDIA PLANE', .7, 1.55, 1.35, CYAN)
labels = [('RTSP\nCamera', .75, 2.25, 1.45, CYAN), ('Camera\nIngestion', 2.65, 2.25, 1.55, CYAN), ('Bounded Media\nDistributor', 4.65, 2.25, 1.8, CYAN)]
for lab,x,y,w,c in labels:
    box(s,x,y,w,.9,PANEL,line=c); text(s,lab,x,y+.18,w,.48,15,WHITE,True,PP_ALIGN.CENTER)
arrow(s,2.2,2.7,2.65,2.7); arrow(s,4.2,2.7,4.65,2.7)
for lab,y,col in [('Recorder → fMP4',1.65,GREEN),('LiveKit → WebRTC',2.65,BLUE),('Detector → events',3.65,AMBER)]:
    box(s,7.25,y,2.3,.72,PANEL,line=col); text(s,lab,7.25,y+.2,2.3,.28,14,WHITE,True,PP_ALIGN.CENTER); arrow(s,6.45,2.7,7.25,y+.36,col)
box(s,10.1,1.65,2.25,.72,PANEL,line=GREEN); text(s,'Filesystem + PostgreSQL',10.1,1.84,2.25,.28,13,WHITE,True,PP_ALIGN.CENTER); arrow(s,9.55,2.01,10.1,2.01,GREEN)
box(s,10.1,2.65,2.25,.72,PANEL,line=BLUE); text(s,'Browser viewer',10.1,2.84,2.25,.28,14,WHITE,True,PP_ALIGN.CENTER); arrow(s,9.55,3.01,10.1,3.01,BLUE)
box(s,10.1,3.65,2.25,.72,PANEL,line=AMBER); text(s,'Rules / API / analytics',10.1,3.84,2.25,.28,13,WHITE,True,PP_ALIGN.CENTER); arrow(s,9.55,4.01,10.1,4.01,AMBER)
pill(s, 'CONTROL PLANE', .7, 5.15, 1.55, BLUE)
for lab,x,w in [('Browser',2.55,1.25),('Trusted proxy',4.25,1.55),('HTTP/JSON API',6.25,1.75),('Use cases + owners',8.45,2.1)]:
    box(s,x,5.02,w,.78,NAVY_2,line=BLUE); text(s,lab,x,5.26,w,.25,13,WHITE,True,PP_ALIGN.CENTER)
arrow(s,3.8,5.41,4.25,5.41,BLUE); arrow(s,5.8,5.41,6.25,5.41,BLUE); arrow(s,8.0,5.41,8.45,5.41,BLUE)
text(s, 'Media packets never travel on the domain event bus.', 2.55, 6.15, 8.1, .3, 14, AMBER, True)

# 4 components
s = prs.slides.add_slide(blank); base(s, 'Components and singular ownership', 'System map', 4)
components = [
 ('camera', 'RTSP session, lifecycle, timestamps, reconnect', CYAN), ('media', 'Packet/frame types and bounded distribution', CYAN),
 ('recorder', 'Segment creation and finalization', GREEN), ('storage + database', 'Paths, index, retrieval, reconciliation, retention', GREEN),
 ('livekit', 'Viewer-gated real-time publication', BLUE), ('detector', 'Sampling, decode, inference seam, results', AMBER),
 ('events', 'Bounded domain facts and overflow policy', AMBER), ('config + auth', 'Validated settings, secrets, identity, scope, audit', RED),
 ('server + UI', 'HTTP adapter, dashboard, health and metrics', BLUE), ('analytics / rules / worker', 'Reserved seams; not delivered features', MUTED),
]
for i,(name,desc,col) in enumerate(components):
    row,col_i = divmod(i,2); x=.72+col_i*6.05; y=1.55+row*1.04
    card(s,name,desc,x,y,5.78,.82,col)

# 5 ingest
s = prs.slides.add_slide(blank); base(s, 'Camera ingest: one connection, explicit recovery', 'Core invariant', 5)
states=[('DISCONNECTED',MUTED),('CONNECTING',CYAN),('CONNECTED',GREEN),('RECONNECTING',AMBER)]
for i,(lab,col) in enumerate(states):
    x=.75+i*3.05; box(s,x,1.75,2.45,.72,PANEL,line=col); text(s,lab,x,1.97,2.45,.25,13,WHITE,True,PP_ALIGN.CENTER)
    if i<3: arrow(s,x+2.45,2.11,x+3.05,2.11,col)
bullets(s,[
 'One authenticated Retina RTSP/TCP H.264 session per camera.',
 'Epoch identity distinguishes reconnect generations and stale work.',
 'Bounded exponential backoff; retryable and terminal failures are distinct.',
 'Camera supervisors isolate failures across cameras and consumers.',
 'Health snapshots and transition metrics expose degraded state.'
],.78,3.05,5.8,2.75,16)
card(s,'Why this matters','Recording, live viewing, and detection attach to the same distributor. A failed publisher or detector cannot silently create another upstream session or terminate recording.',7.0,3.05,5.55,2.3,CYAN)
text(s,'Qualified path: H.264 over RTSP/TCP. Representative camera diversity remains an external gate.',7.15,5.65,5.2,.65,14,AMBER,True)

# 6 recording
s = prs.slides.add_slide(blank); base(s, 'Recording, indexing, playback, retention', 'Preserve footage first', 6)
steps=[('Encoded packets',CYAN),('Fragmented MP4',GREEN),('Atomic finalize',GREEN),('Probe real metadata',BLUE),('PostgreSQL index',BLUE)]
for i,(lab,col) in enumerate(steps):
    x=.65+i*2.54; box(s,x,1.65,2.05,.72,PANEL,line=col); text(s,lab,x,1.88,2.05,.25,12.5,WHITE,True,PP_ALIGN.CENTER)
    if i<4: arrow(s,x+2.05,2.01,x+2.54,2.01,col)
card(s,'Deterministic storage','camera-id / UTC date / HH-MM-SS.ext. Partial files remain as reconciliation evidence after forced exit.',.72,3.0,3.72,1.55,GREEN)
card(s,'Indexed retrieval','Overlap queries use camera + [start,end); multi-segment H.264 is assembled with documented segment-aligned accuracy.',4.8,3.0,3.72,1.55,BLUE)
card(s,'Pressure-aware retention','Oldest-first indexed cleanup, watermarks with hysteresis, and protected recent footage. Database outage does not immediately stop media preservation.',8.88,3.0,3.72,1.55,AMBER)
text(s,'Crash recovery ordering',.75,5.15,2.35,.3,13,MUTED,True)
text(s,'Durable file → probe → async metadata → continuous reconciliation',.75,5.52,8.7,.45,20,WHITE,True)
pill(s,'WORKING',10.55,5.45,1.35,GREEN)

# 7 live
s = prs.slides.add_slide(blank); base(s, 'Live viewing: browser-compatible and viewer-gated', 'LiveKit / WebRTC', 7)
for lab,x,col in [('H.264 access units',.72,CYAN),('FFmpeg decode',3.15,AMBER),('I420 frames',5.4,AMBER),('WebRTC encode',7.45,BLUE),('LiveKit room',9.65,BLUE),('Browser',11.75,GREEN)]:
    w=1.6 if x<11 else .85; box(s,x,1.72,w,.72,PANEL,line=col); text(s,lab,x,1.92,w,.3,11.5,WHITE,True,PP_ALIGN.CENTER)
for a,b in [(.72+1.6,3.15),(4.75,5.4),(7.0,7.45),(9.05,9.65),(11.25,11.75)]: arrow(s,a,2.08,b,2.08,BLUE)
card(s,'Load-bearing behavior','Nothing decodes while there is no remote participant. A bounded idle grace avoids unnecessary teardown during brief viewer gaps.',.75,3.0,3.7,1.7,BLUE)
card(s,'Failure isolation','The publisher is a bounded consumer behind PublicationAdapter. Non-retryable publication failure stops publication—not recording or ingest.',4.8,3.0,3.7,1.7,GREEN)
card(s,'Measured local evidence','1280×720 @ 15 FPS published frame-for-frame with 1, 2, and 4 viewers in controlled local runs.',8.85,3.0,3.7,1.7,CYAN)
text(s,'Limit: adapter-clock timestamps do not support a camera-to-glass latency claim.',.78,5.45,8.6,.45,17,AMBER,True)
text(s,'Native encoded pass-through is implemented but refused on Apple due to an upstream crash path.',.78,6.03,9.6,.38,14,MUTED)

# 8 detection/events
s = prs.slides.add_slide(blank); base(s, 'Detection and domain events', 'Independent degradation', 8)
steps=[('Sample by media time',CYAN),('Decode in owned FFmpeg',AMBER),('Freshness queue',AMBER),('Inference seam',BLUE),('Typed detection event',GREEN)]
for i,(lab,col) in enumerate(steps):
    x=.67+i*2.53; box(s,x,1.68,2.03,.8,PANEL,line=col); text(s,lab,x,1.88,2.03,.38,12,WHITE,True,PP_ALIGN.CENTER)
    if i<4: arrow(s,x+2.03,2.08,x+2.53,2.08,col)
card(s,'Bounded overload','Slow detection drops/skips stale frames and favors recent work. Memory cannot grow without bound.',.75,3.1,3.72,1.55,AMBER)
card(s,'Structured facts','Results carry camera, media timestamp, normalized boxes, confidence, and model version.',4.82,3.1,3.72,1.55,GREEN)
card(s,'Separate event plane','Camera, recording, storage, and detection facts share explicit ordering and per-class overflow policies—never video payloads.',8.89,3.1,3.72,1.55,CYAN)
box(s,.75,5.3,11.85,.72,NAVY_2,line=RED)
text(s,'Current limit: DeterministicFakeInference is the only model. The systems pipeline works; production detection accuracy is not claimed.',.95,5.51,11.45,.3,15,WHITE,True,PP_ALIGN.CENTER)

# 9 API/security
s = prs.slides.add_slide(blank); base(s, 'Operator surface and security boundary', 'Control plane', 9)
for lab,x,w,col in [('Browser',.75,1.55,BLUE),('Identity-aware proxy',2.75,2.05,RED),('Authorization chokepoint',5.25,2.25,RED),('Domain owners',7.95,1.8,CYAN),('PostgreSQL audit',10.2,2.15,GREEN)]:
    box(s,x,1.75,w,.78,PANEL,line=col); text(s,lab,x,1.99,w,.28,13,WHITE,True,PP_ALIGN.CENTER)
for a,b in [(2.3,2.75),(4.8,5.25),(7.5,7.95),(9.75,10.2)]: arrow(s,a,2.14,b,2.14,RED)
bullets(s,[
 'Opaque bearer tokens are Argon2id-hashed; principals have roles and camera scope.',
 'Every protected route reaches one decision point; out-of-scope resources return 404.',
 'Forwarded identity is read only after peer allowlist + constant-time shared-secret proof.',
 'Non-loopback binding is refused without a valid trust boundary and enabled principal.',
 'Secrets use 0600-enforced files plus per-key environment override; values are redacted.'
],.78,3.1,6.1,3.0,15)
card(s,'HTTP surface','Dashboard, camera lifecycle, health, recording search/detail/playback, settings, system state, identity administration, viewer tokens, readiness and Prometheus metrics.',7.25,3.1,5.3,1.85,BLUE)
card(s,'Audit categories','Authentication, authorization, camera lifecycle, recording export, configuration, and identity changes. Fingerprints—not secret values.',7.25,5.18,5.3,1.2,GREEN)

# 10 operations
s = prs.slides.add_slide(blank); base(s, 'Failure model and observability', 'Production-oriented behavior', 10)
items=[
 ('Recorder slow/fails','Preservation-first policy; inability to keep up is surfaced.',GREEN),
 ('Database unavailable','Recording continues within bounded metadata backlog; reconciliation repairs.',BLUE),
 ('LiveKit unavailable','Publication degrades independently; ingest and recording continue.',BLUE),
 ('Detector overloaded','Stale work is discarded; fresh frames win.',AMBER),
 ('One camera fails','Independent supervisor and reconnect loop; other cameras continue.',CYAN),
 ('Storage pressure','Warning/critical watermarks, hysteresis, protected recent footage.',RED),
]
for i,(a,b,c) in enumerate(items):
    row,col_i=divmod(i,2); card(s,a,b,.72+col_i*6.05,1.55+row*1.35,5.78,1.08,c)
text(s,'Observed with bounded-cardinality metrics',.75,5.8,4.0,.3,13,MUTED,True)
text(s,'camera state • queue depth/drops • segments • storage • detection • LiveKit • API latency',.75,6.17,11.4,.4,16,WHITE,True)

# 11 delivered
s = prs.slides.add_slide(blank); base(s, 'What is working today', 'Delivered capability', 11)
working=[
 'End-to-end local H.264: RTSP ingest → recording → index → playback',
 'Independent multi-camera supervision and bounded media distribution',
 'PostgreSQL reconciliation, retention, and storage-pressure control',
 'Browser live view through viewer-gated LiveKit transcoding',
 'Sampled detection pipeline and bounded typed domain event bus',
 'Persistent validated configuration with transactional audit',
 'Authentication, camera-scoped authorization, proxy trust proof, secret redaction',
 'Operator dashboard, HTTP API, health/readiness, Prometheus metrics',
]
for i,item in enumerate(working):
    row,col_i=divmod(i,2); x=.75+col_i*6.05; y=1.55+row*1.25
    box(s,x,y,5.75,.92,PANEL,line=GREEN); pill(s,'✓',x+.16,y+.27,.45,GREEN); text(s,item,x+.78,y+.18,4.75,.58,13.5,WHITE,True)
text(s,'Quality gate last documented: 509 passed, 0 failed, 34 ignored across 43 suites; external fixtures remain separate.',.78,6.68,11.6,.3,12.5,MUTED)

# 12 gaps
s = prs.slides.add_slide(blank); base(s, 'What we still lack', 'No overclaiming', 12)
gaps=[
 ('Production detection','No real model, labeled dataset, or accuracy acceptance criteria.',RED),
 ('ROI / rules / analytics','No confirmed ROI coordinates, rule semantics, actions, or heatmap meaning.',RED),
 ('Deployment package','Linux target, storage layout, service unit, TLS/IdP integration, ownership policy open.',AMBER),
 ('Operational proof','Upgrade/rollback, backup/restore, real disk-full, network partition, promtool evaluation incomplete.',AMBER),
 ('Camera qualification','Representative cameras, H.265/profile diversity, GOP policy, audio/PTZ requirements open.',CYAN),
 ('Live SLOs','Cold start, first picture, reconnect experience, camera-to-glass latency, concurrency target open.',BLUE),
 ('Concurrency/fuzzing','Concurrent settings writers, boundary-applied changes, and fuzz targets are unqualified.',MUTED),
 ('Scale claim','No sustained 10- or 20-camera qualification on target hardware.',RED),
]
for i,(a,b,c) in enumerate(gaps):
    row,col_i=divmod(i,2); card(s,a,b,.72+col_i*6.05,1.48+row*1.31,5.78,1.05,c)

# 13 scale
s = prs.slides.add_slide(blank); base(s, 'Scale by measurement—not architecture alone', 'Phase 9', 13)
for lab,x,col in [('1 camera\nbaseline',.85,CYAN),('10 camera\nsoak',3.35,GREEN),('Profile the\nconstraint',5.85,AMBER),('20 camera\nqualification',8.35,BLUE),('Extraction\ndecision',10.85,RED)]:
    box(s,x,1.75,1.65,1.05,PANEL,line=col); text(s,lab,x,2.02,1.65,.55,15,WHITE,True,PP_ALIGN.CENTER)
for x in [2.5,5.0,7.5,10.0]: arrow(s,x,2.28,x+.85,2.28,MUTED)
card(s,'Measure','CPU/GPU, memory, disk, network, queue drops, recovery, and p50/p90/p99 latency under a named codec/resolution/FPS/bitrate/viewer/detection workload.',.78,3.45,5.75,2.0,CYAN)
card(s,'Extract only when justified','A worker, recording node, LiveKit node, external broker, or internal gRPC boundary requires a measured bottleneck and ADR. Media stays off ordinary gRPC.',6.82,3.45,5.75,2.0,AMBER)
text(s,'Current status: architecture is ready to be tested; supported camera count is not yet proven.',.8,6.15,11.7,.4,18,WHITE,True,PP_ALIGN.CENTER)

# 14 next
s = prs.slides.add_slide(blank); base(s, 'Recommended next milestones', 'From working system to deployable product', 14)
milestones=[
 ('1', 'Confirm product + deployment gates', 'Target Linux, storage, cameras, live SLO, production IdP, ROI/rule/heatmap semantics.'),
 ('2', 'Package and rehearse operations', 'Service lifecycle, ownership, TLS proxy, backup/restore, upgrade/rollback, alert evaluation.'),
 ('3', 'Integrate a production detector', 'Choose model/hardware, labeled dataset, accuracy criteria, and throughput budget.'),
 ('4', 'Run Phase 9 qualification', 'Baseline → 10-camera soak → profile → tune → 20-camera result → extraction decision.'),
]
for i,(num,title_,body) in enumerate(milestones):
    y=1.5+i*1.28; pill(s,num,.75,y+.15,.55,[CYAN,GREEN,AMBER,BLUE][i]); text(s,title_,1.55,y,4.2,.35,17,WHITE,True); text(s,body,5.05,y,7.3,.65,14,MUTED)
text(s,'SENTINEL already demonstrates the hard topology: ingest once, distribute safely, preserve evidence, and degrade visibly.',.78,6.65,11.7,.4,16,CYAN,True,PP_ALIGN.CENTER)

OUT.parent.mkdir(parents=True, exist_ok=True)
prs.save(OUT)
print(f'Wrote {OUT} ({len(prs.slides)} slides)')
