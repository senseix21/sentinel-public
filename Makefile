SHELL := /bin/bash
.DEFAULT_GOAL := help

CARGO ?= cargo
NODE ?= node
FFMPEG_BIN ?= ffmpeg
FFPLAY_BIN ?= ffplay
CURL_BIN ?= curl

WEBCAM_DEVICE ?= 0
VIDEO_WIDTH ?= 1280
VIDEO_HEIGHT ?= 720
VIDEO_FPS ?= 15
# Cap the development source at something a 720p IP camera would plausibly
# emit. Without it libx264 runs unconstrained -- measured at 3.91 Mbps on this
# webcam -- which is not a realistic ingest and is a poor live-publication
# workload. Not product configuration: this is the fixture, not the camera.
VIDEO_BITRATE_KBPS ?= 1500
CAMERA_ID ?= $(or $(SURVEILLANCE_CAMERA_ID),camera-01)
RTSP_URL ?= $(SURVEILLANCE_RTSP_URL)
RECORDING_ROOT ?= ./recordings
HEALTH_BIND ?= 127.0.0.1:3000

.PHONY: help prerequisites webcam-devices run run-release server direct-camera-qualification health live ready metrics recordings play verify-recordings check

help: ## Show manual development commands and configurable values.
	@awk 'BEGIN {FS = ":.*## "} /^[a-zA-Z0-9_-]+:.*## / {printf "  %-20s %s\n", $$1, $$2}' $(MAKEFILE_LIST)
	@echo
	@echo "Direct camera: RTSP_URL=rtsp://camera-host:554/stream CAMERA_ID=camera-01"
	@echo "Overrides: VIDEO_WIDTH=1280 VIDEO_HEIGHT=720 RECORDING_ROOT=./recordings HEALTH_BIND=127.0.0.1:3000"

prerequisites: ## Verify required manual-run executables are available.
	@command -v "$(CARGO)" >/dev/null || { echo "missing Rust cargo" >&2; exit 1; }
	@command -v "$(FFMPEG_BIN)" >/dev/null || { echo "missing FFmpeg" >&2; exit 1; }
	@command -v "$(CURL_BIN)" >/dev/null || { echo "missing curl" >&2; exit 1; }
	@command -v ffprobe >/dev/null || { echo "missing ffprobe" >&2; exit 1; }

webcam-devices: ## List macOS AVFoundation video and audio device indices.
	@"$(FFMPEG_BIN)" -hide_banner -f avfoundation -list_devices true -i "" 2>&1 || true

run: ## Run the local webcam through the Rust FFmpeg source and LiveKit.
	WEBCAM_DEVICE="$(WEBCAM_DEVICE)" VIDEO_WIDTH="$(VIDEO_WIDTH)" VIDEO_HEIGHT="$(VIDEO_HEIGHT)" VIDEO_FPS="$(VIDEO_FPS)" VIDEO_BITRATE_KBPS="$(VIDEO_BITRATE_KBPS)" CAMERA_ID="$(CAMERA_ID)" RECORDING_ROOT="$(RECORDING_ROOT)" scripts/dev/local-webcam-stack.sh

run-release: ## Run the optimized local webcam stack.
	WEBCAM_DEVICE="$(WEBCAM_DEVICE)" VIDEO_WIDTH="$(VIDEO_WIDTH)" VIDEO_HEIGHT="$(VIDEO_HEIGHT)" VIDEO_FPS="$(VIDEO_FPS)" VIDEO_BITRATE_KBPS="$(VIDEO_BITRATE_KBPS)" CAMERA_ID="$(CAMERA_ID)" RECORDING_ROOT="$(RECORDING_ROOT)" SENTINEL_BUILD_PROFILE=release scripts/dev/local-webcam-stack.sh

server: ## Run the development server against a direct RTSP camera.
	@test -n "$(RTSP_URL)" || { echo "missing RTSP_URL (or SURVEILLANCE_RTSP_URL)" >&2; exit 2; }
	SURVEILLANCE_CAMERA_ID="$(CAMERA_ID)" \
	SURVEILLANCE_RTSP_URL="$(RTSP_URL)" \
	SURVEILLANCE_VIDEO_WIDTH="$(VIDEO_WIDTH)" \
	SURVEILLANCE_VIDEO_HEIGHT="$(VIDEO_HEIGHT)" \
	SURVEILLANCE_RECORDING_ROOT="$(RECORDING_ROOT)" \
	SURVEILLANCE_HEALTH_BIND="$(HEALTH_BIND)" \
	RUST_LOG="$${RUST_LOG:-info}" \
	"$(CARGO)" run -p surveillance-server

direct-camera-qualification: ## Qualify one physical camera over direct RTSP (see SPEC).
	scripts/phase9/direct-camera-qualification.sh

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

check: ## Run the complete Rust workspace quality gate.
	bash -n scripts/dev/local-webcam-stack.sh scripts/phase9/direct-camera-qualification.sh
	python3 -m unittest scripts/dev/test_trust_boundary_proxy.py scripts/phase9/test_direct_camera_report.py
	"$(NODE)" --test apps/server/ui/tests/*.test.js
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
