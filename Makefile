SHELL := /bin/bash
.DEFAULT_GOAL := help

CARGO ?= cargo
FFMPEG_BIN ?= ffmpeg
FFPLAY_BIN ?= ffplay
MEDIAMTX_BIN ?= mediamtx
CURL_BIN ?= curl

WEBCAM_DEVICE ?= 0
VIDEO_WIDTH ?= 1280
VIDEO_HEIGHT ?= 720
VIDEO_FPS ?= 15
# Cap the development source at something a 720p IP camera would plausibly
# emit. Without it libx264 runs unconstrained -- measured at 3.91 Mbps on this
# webcam -- which is not a realistic ingest and is a poor live-publication
# workload. Not product configuration: this is the fixture, not the camera.
VIDEO_BITRATE ?= 1500k
CAMERA_ID ?= mac-webcam
RTSP_URL ?= rtsp://127.0.0.1:8554/webcam
RECORDING_ROOT ?= ./recordings
HEALTH_BIND ?= 127.0.0.1:3000
MEDIAMTX_CONFIG ?= config/mediamtx-webcam.yml

.PHONY: help prerequisites webcam-devices run run-release mediamtx webcam server health live ready metrics recordings play verify-recordings fixture check

help: ## Show manual development commands and configurable values.
	@awk 'BEGIN {FS = ":.*## "} /^[a-zA-Z0-9_-]+:.*## / {printf "  %-20s %s\n", $$1, $$2}' $(MAKEFILE_LIST)
	@echo
	@echo "Overrides: WEBCAM_DEVICE=0 VIDEO_WIDTH=1280 VIDEO_HEIGHT=720 VIDEO_FPS=15 VIDEO_BITRATE=1500k"
	@echo "           CAMERA_ID=mac-webcam RECORDING_ROOT=./recordings HEALTH_BIND=127.0.0.1:3000"

prerequisites: ## Verify required manual-run executables are available.
	@command -v "$(CARGO)" >/dev/null || { echo "missing Rust cargo" >&2; exit 1; }
	@command -v "$(FFMPEG_BIN)" >/dev/null || { echo "missing FFmpeg" >&2; exit 1; }
	@command -v "$(MEDIAMTX_BIN)" >/dev/null || { echo "missing MediaMTX" >&2; exit 1; }
	@command -v "$(CURL_BIN)" >/dev/null || { echo "missing curl" >&2; exit 1; }
	@command -v ffprobe >/dev/null || { echo "missing ffprobe" >&2; exit 1; }

webcam-devices: ## List macOS AVFoundation video and audio device indices.
	@"$(FFMPEG_BIN)" -hide_banner -f avfoundation -list_devices true -i "" 2>&1 || true

# One command for the whole local loop. The three targets below still exist and
# still run standalone; this supervises them together, adds the PostgreSQL the
# Playback and Settings screens need, adds LiveKit so the live tile carries
# video, and puts the trust-boundary proxy in front that an authenticated
# dashboard session requires. Development only, loopback only — the proxy owns
# a bounded in-memory password session. See the script's header.
# It does not depend on `prerequisites`: it needs a strict superset of those
# tools and checks them itself, and two competing "missing X" messages for the
# same run would be one too many.
run: ## Bring up the full local stack and serve the dashboard.
	scripts/dev/local-stack.sh

run-release: ## Run the local stack with the optimized qualification binary.
	SENTINEL_BUILD_PROFILE=release scripts/dev/local-stack.sh

mediamtx: prerequisites ## Run loopback-only MediaMTX for the webcam RTSP bridge.
	"$(MEDIAMTX_BIN)" "$(MEDIAMTX_CONFIG)"

webcam: prerequisites ## Publish the selected Mac webcam as qualified H.264 RTSP.
	"$(FFMPEG_BIN)" -hide_banner -loglevel info \
		-f avfoundation -framerate "$(VIDEO_FPS)" -video_size "$(VIDEO_WIDTH)x$(VIDEO_HEIGHT)" \
		-i "$(WEBCAM_DEVICE):none" -an -vf format=yuv420p \
		-r "$(VIDEO_FPS)" \
		-c:v libx264 -profile:v baseline -preset veryfast -tune zerolatency \
		-b:v "$(VIDEO_BITRATE)" -maxrate "$(VIDEO_BITRATE)" -bufsize "$(VIDEO_BITRATE)" \
		-g "$(VIDEO_FPS)" -keyint_min "$(VIDEO_FPS)" -sc_threshold 0 -bf 0 \
		-f rtsp -rtsp_transport tcp "$(RTSP_URL)"

server: ## Run the standalone server against the configured local RTSP source.
	SURVEILLANCE_CAMERA_ID="$(CAMERA_ID)" \
	SURVEILLANCE_RTSP_URL="$(RTSP_URL)" \
	SURVEILLANCE_VIDEO_WIDTH="$(VIDEO_WIDTH)" \
	SURVEILLANCE_VIDEO_HEIGHT="$(VIDEO_HEIGHT)" \
	SURVEILLANCE_RECORDING_ROOT="$(RECORDING_ROOT)" \
	SURVEILLANCE_HEALTH_BIND="$(HEALTH_BIND)" \
	RUST_LOG="$${RUST_LOG:-info}" \
	"$(CARGO)" run -p surveillance-server

health: live ready metrics ## Query all operational endpoints.

live: ## Query process liveness.
	@"$(CURL_BIN)" --fail --silent --show-error "http://$(HEALTH_BIND)/live"

ready: ## Query camera-dependent readiness.
	@"$(CURL_BIN)" --fail --silent --show-error "http://$(HEALTH_BIND)/ready"

metrics: ## Print Prometheus text metrics.
	@"$(CURL_BIN)" --fail --silent --show-error "http://$(HEALTH_BIND)/metrics"

recordings: ## List durable recordings for the selected camera.
	@find "$(RECORDING_ROOT)/$(CAMERA_ID)" -type f -name '*.mp4' ! -name '*.partial.mp4' -print 2>/dev/null | sort

play: ## Play one recording: make play FILE=/absolute/or/relative/file.mp4
	@test -n "$(FILE)" || { echo "usage: make play FILE=path/to/recording.mp4" >&2; exit 2; }
	@test -f "$(FILE)" || { echo "recording does not exist: $(FILE)" >&2; exit 2; }
	@command -v "$(FFPLAY_BIN)" >/dev/null || { echo "missing ffplay" >&2; exit 1; }
	"$(FFPLAY_BIN)" "$(FILE)"

verify-recordings: ## Fully decode every durable recording for the selected camera.
	@found=0; while IFS= read -r file; do \
		found=1; echo "verifying $$file"; \
		"$(FFMPEG_BIN)" -v error -i "$$file" -map 0:v:0 -f null - || exit; \
	done < <(find "$(RECORDING_ROOT)/$(CAMERA_ID)" -type f -name '*.mp4' ! -name '*.partial.mp4' -print 2>/dev/null | sort); \
	if [[ $$found -eq 0 ]]; then echo "no durable recordings found" >&2; exit 1; fi

fixture: ## Run the complete deterministic MediaMTX/Retina Phase 2 qualification.
	scripts/phase0/retina-rtsp-fixture.sh

check: ## Run the complete Rust workspace quality gate.
	bash -n scripts/dev/local-stack.sh scripts/phase9/authenticated-ui-fixture.sh
	python3 -m unittest scripts/dev/test_trust_boundary_proxy.py
	"$(CARGO)" fmt --all --check
	"$(CARGO)" clippy --workspace --all-targets --all-features -- -D warnings
	"$(CARGO)" test --workspace --all-features
	"$(CARGO)" build --workspace --release
	$(MAKE) audit

.PHONY: audit
audit: ## Phase 8 (P8.10) dependency policy: advisories, licences, sources.
	@command -v cargo-deny >/dev/null || { echo "cargo-deny is not installed: cargo install cargo-deny" >&2; exit 1; }
	@command -v cargo-audit >/dev/null || { echo "cargo-audit is not installed: cargo install cargo-audit" >&2; exit 1; }
	"$(CARGO)" deny check
	"$(CARGO)" audit
