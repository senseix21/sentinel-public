# Decisions and open questions

Accepted: tight axis-aligned face rectangles, confidence threshold 70%, no person
boxes in face mode. YuNet 2023mar fixed 640 geometry and pinned MIT artifact is a
reversible development default. Person mode remains an explicit rollback.

Open: representative production accuracy, minimum face size, pose/occlusion/
lighting, memory and target hardware; actual Firefox playback; full manual
recovery matrix; file-backed RTSP face end-to-end qualification. No 10/20-camera
capacity or hard foreign-inference timeout claim is supported. These limits do
not prevent use of the completed local development slice.
