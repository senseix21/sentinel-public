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
VIDEO_FPS ?= 30
# Cap the development source at something a 720p IP camera would plausibly
# emit. Without it libx264 runs unconstrained -- measured at 3.91 Mbps on this
# webcam -- which is not a realistic ingest and is a poor live-publication
# workload. Not product configuration: this is the fixture, not the camera.
VIDEO_BITRATE_KBPS ?= 1500
CAMERA_ID ?= $(or $(SURVEILLANCE_CAMERA_ID),camera-01)
RTSP_URL ?= $(SURVEILLANCE_RTSP_URL)
RECORDING_ROOT ?= ./recordings
HEALTH_BIND ?= 127.0.0.1:3000

.PHONY: help prerequisites webcam-devices webcam webcam-release fixture run run-release server direct-camera-qualification health live ready metrics recordings play verify-recordings check verify verify-onnx container-shell container-verify container-verify-onnx container-verify-model container-verify-accuracy container-verify-livekit container-reproduce-livekit-restart

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
	@command -v initdb >/dev/null || { echo "missing PostgreSQL initdb" >&2; exit 1; }
	@command -v pg_ctl >/dev/null || { echo "missing PostgreSQL pg_ctl" >&2; exit 1; }
	@command -v psql >/dev/null || { echo "missing PostgreSQL psql" >&2; exit 1; }
	@command -v livekit-server >/dev/null || { echo "missing LiveKit livekit-server" >&2; exit 1; }
	@command -v lsof >/dev/null || { echo "missing lsof" >&2; exit 1; }
	@command -v python3 >/dev/null || { echo "missing python3" >&2; exit 1; }

webcam-devices: ## List macOS AVFoundation video and audio device indices.
	@"$(FFMPEG_BIN)" -hide_banner -f avfoundation -list_devices true -i "" 2>&1 || true

run: ## Start the dashboard and services without requiring a camera.
	RECORDING_ROOT="$(RECORDING_ROOT)" scripts/dev/local-stack.sh

run-release: ## Start the camera-independent dashboard with an optimized build.
	RECORDING_ROOT="$(RECORDING_ROOT)" SENTINEL_BUILD_PROFILE=release scripts/dev/local-stack.sh

webcam: ## Run webcam + ONNX detection and open the web UI.
	WEBCAM_DEVICE="$(WEBCAM_DEVICE)" VIDEO_WIDTH="$(VIDEO_WIDTH)" VIDEO_HEIGHT="$(VIDEO_HEIGHT)" VIDEO_FPS="$(VIDEO_FPS)" VIDEO_BITRATE_KBPS="$(VIDEO_BITRATE_KBPS)" CAMERA_ID="$(CAMERA_ID)" RECORDING_ROOT="$(RECORDING_ROOT)" scripts/dev/local-webcam-stack.sh

webcam-release: ## Run the optimized local webcam stack.
	WEBCAM_DEVICE="$(WEBCAM_DEVICE)" VIDEO_WIDTH="$(VIDEO_WIDTH)" VIDEO_HEIGHT="$(VIDEO_HEIGHT)" VIDEO_FPS="$(VIDEO_FPS)" VIDEO_BITRATE_KBPS="$(VIDEO_BITRATE_KBPS)" CAMERA_ID="$(CAMERA_ID)" RECORDING_ROOT="$(RECORDING_ROOT)" SENTINEL_BUILD_PROFILE=release scripts/dev/local-webcam-stack.sh

fixture: ## Start with a loopback RTSP fixture supplied via SENTINEL_RTSP_FIXTURE_URL.
	@test -n "$${SENTINEL_RTSP_FIXTURE_URL}" || { echo "missing SENTINEL_RTSP_FIXTURE_URL" >&2; exit 2; }
	SENTINEL_SOURCE_MODE=fixture scripts/dev/local-webcam-stack.sh

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

ready: ## Query application dependency readiness.
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
	$(MAKE) verify
	$(MAKE) audit

verify-live-browser: ## Check the running local live UI; supply Playwright via PLAYWRIGHT_MODULE_PATH.
	"$(NODE)" scripts/dev/check-live-browser.cjs

.PHONY: verify-live-browser

verify: ## Run the gate without the live advisory database (used by pull-request CI).
	bash -n scripts/dev/local-stack.sh scripts/dev/local-webcam-stack.sh scripts/dev/onnx-defaults.sh scripts/dev/livekit-config.sh scripts/phase9/direct-camera-qualification.sh
	python3 -m unittest scripts/dev/test_trust_boundary_proxy.py scripts/dev/test_onnx_defaults.py scripts/dev/test_livekit_loopback.py scripts/phase9/test_direct_camera_report.py
	"$(NODE)" --check scripts/dev/check-live-browser.cjs
	"$(NODE)" --check scripts/dev/check-rtsp-recovery.cjs
	"$(NODE)" --test apps/server/ui/tests/*.test.js
	"$(CARGO)" fmt --all --check
	"$(CARGO)" clippy --workspace --all-targets -- -D warnings
	"$(CARGO)" test --workspace
	"$(CARGO)" build --workspace --release

# The ONNX backend is optional because its runtime, `ort`, has no prebuilt for
# every platform this is developed on: there is none for x86_64-apple-darwin.
# Keeping it behind a feature means the whole tree still builds, tests and ships
# on any machine, with the fixture backend, and `PersonInference` stays a real
# substitution boundary rather than a notional one. This target is the other
# half of the gate and needs a platform `ort` supports; `container-verify-onnx`
# provides one. See docs/adr/015-onnx-person-inference.md.
# `package/feature` rather than a bare name: this is a virtual workspace, where
# a bare feature is ambiguous and cargo resolves it into a build whose members
# cannot find their own dependencies.
ONNX_FEATURES = surveillance-detector/onnx-inference,surveillance-server/onnx-inference

verify-onnx: ## Run the gate with the ONNX inference backend enabled.
	"$(CARGO)" clippy --workspace --all-targets --features $(ONNX_FEATURES) -- -D warnings
	"$(CARGO)" test --workspace --features $(ONNX_FEATURES)

# --- Containerised build -----------------------------------------------------
# This host is x86_64, so linux/amd64 runs natively rather than emulated, and
# x86_64-unknown-linux-gnu is a target `ort` ships prebuilts for. The image
# carries everything `make verify` needs, so a container run is the same gate.

IMAGE ?= camera-project-dev
# A named volume for the crate registry, so a second container run does not
# re-download every dependency. The build directory is kept out of the host's
# by CARGO_TARGET_DIR in the image.
CARGO_VOLUME ?= camera-project-cargo
DOCKER_RUN = docker run --rm --platform linux/amd64 -v "$(PWD)":/work -v "$(CARGO_VOLUME)":/usr/local/cargo/registry -w /work

container-image: ## Build the development image.
	docker build --platform linux/amd64 -t "$(IMAGE)" -f deploy/dev/Dockerfile .

container-shell: container-image ## Open a shell in the development image.
	$(DOCKER_RUN) -it "$(IMAGE)" bash

container-verify: container-image ## Run the deterministic gate in the container.
	$(DOCKER_RUN) "$(IMAGE)" make verify

container-verify-onnx: container-image ## Run the ONNX gate in the container.
	$(DOCKER_RUN) "$(IMAGE)" make verify-onnx

# DI.R8: the adapter against a real model and a real frame. Neither input is
# committed — the model is someone else's weights and the frame is a
# photograph — so both are supplied, and the target names the missing one
# rather than running a test that can pass by finding nothing.
#
# MODEL and FRAME are paths inside the repository, because the container sees
# only what is mounted. SOURCE_WIDTH and SOURCE_HEIGHT are the camera geometry
# the frame was letterboxed from, which is what the inverse needs; INPUT_WIDTH
# and INPUT_HEIGHT are the model's. MINIMUM is how many detections the frame is
# known to hold.
MODEL ?=
FRAME ?=
SOURCE_WIDTH ?=
SOURCE_HEIGHT ?=
MODEL_SHA256 ?=
INPUT_WIDTH ?= 416
INPUT_HEIGHT ?= 416
MINIMUM ?= 1

container-verify-model: container-image ## Run DI.R8 on a real model. MODEL= FRAME= SOURCE_WIDTH= SOURCE_HEIGHT=
	@test -n "$(MODEL)" || { echo "MODEL= is required: a path in this repository to an .onnx artifact" >&2; exit 1; }
	@test -n "$(FRAME)" || { echo "FRAME= is required: a path in this repository to a raw RGB24 frame at the model's input geometry" >&2; exit 1; }
	@test -n "$(SOURCE_WIDTH)" && test -n "$(SOURCE_HEIGHT)" || { echo "SOURCE_WIDTH= and SOURCE_HEIGHT= are required: the geometry the frame was letterboxed from" >&2; exit 1; }
	$(DOCKER_RUN) \
		-e DETECTION_MODEL_PATH="/work/$(MODEL)" \
		-e DETECTION_FRAME_PATH="/work/$(FRAME)" \
		-e DETECTION_FRAME_WIDTH="$(INPUT_WIDTH)" \
		-e DETECTION_FRAME_HEIGHT="$(INPUT_HEIGHT)" \
		-e DETECTION_SOURCE_WIDTH="$(SOURCE_WIDTH)" \
		-e DETECTION_SOURCE_HEIGHT="$(SOURCE_HEIGHT)" \
		-e DETECTION_MODEL_SHA256="$(MODEL_SHA256)" \
		-e DETECTION_MINIMUM_DETECTIONS="$(MINIMUM)" \
		"$(IMAGE)" cargo test -p surveillance-detector \
		--features surveillance-detector/onnx-inference \
		--test onnx_real_model -- --ignored --nocapture

container-verify-livekit: container-image ## Qualify publication against a real LiveKit room.
	$(DOCKER_RUN) "$(IMAGE)" bash scripts/phase5/room-fixture.sh

# DI.R10: the adapter across a set with ground truth. Build the set first with
# scripts/detection/build-eval-set.py. MIN_RECALL and MIN_PRECISION have no
# defaults on purpose — a measurement that asserts nothing is a report, and a
# report cannot fail.
MANIFEST ?=
IOU ?= 50
SCORE ?= 30
MIN_RECALL ?=
MIN_PRECISION ?=

container-verify-accuracy: container-image ## Run DI.R10 on an evaluation set. MODEL= MANIFEST= MIN_RECALL= MIN_PRECISION=
	@test -n "$(MODEL)" || { echo "MODEL= is required: a path in this repository to an .onnx artifact" >&2; exit 1; }
	@test -n "$(MANIFEST)" || { echo "MANIFEST= is required: a path in this repository to a generated manifest.json" >&2; exit 1; }
	@test -n "$(MIN_RECALL)" && test -n "$(MIN_PRECISION)" || { echo "MIN_RECALL= and MIN_PRECISION= are required: what this run has to beat to pass" >&2; exit 1; }
	$(DOCKER_RUN) \
		-e DETECTION_MODEL_PATH="/work/$(MODEL)" \
		-e DETECTION_EVAL_MANIFEST="/work/$(MANIFEST)" \
		-e DETECTION_EVAL_IOU="$(IOU)" \
		-e DETECTION_SCORE_PERCENT="$(SCORE)" \
		-e DETECTION_MIN_RECALL_PERCENT="$(MIN_RECALL)" \
		-e DETECTION_MIN_PRECISION_PERCENT="$(MIN_PRECISION)" \
		"$(IMAGE)" cargo test -p surveillance-detector \
		--features surveillance-detector/onnx-inference \
		--test onnx_accuracy -- --ignored --nocapture

.PHONY: container-image
.PHONY: audit
audit: ## Phase 8 (P8.10) dependency policy: advisories, licences, sources.
	@command -v cargo-deny >/dev/null || { echo "cargo-deny is not installed: cargo install cargo-deny" >&2; exit 1; }
	@command -v cargo-audit >/dev/null || { echo "cargo-audit is not installed: cargo install cargo-audit" >&2; exit 1; }
	"$(CARGO)" deny check
	"$(CARGO)" audit
