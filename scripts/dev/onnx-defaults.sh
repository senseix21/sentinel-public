#!/usr/bin/env bash
# Sourced by local-webcam-stack.sh; artifacts stay in ignored local state.
prepare_onnx() {
  export SURVEILLANCE_DETECTION_ENABLED="${SURVEILLANCE_DETECTION_ENABLED:-true}"
  [[ "${SURVEILLANCE_DETECTION_ENABLED}" == true ]] || return 0
  export SURVEILLANCE_CARGO_FEATURES="${SURVEILLANCE_CARGO_FEATURES:+${SURVEILLANCE_CARGO_FEATURES},}onnx-inference"
  local artifact_dir="${repo_root}/.local/onnx"
  mkdir -p "${artifact_dir}"
  export SURVEILLANCE_DETECTION_MODE="${SURVEILLANCE_DETECTION_MODE:-person_face}"
  if [[ "${SURVEILLANCE_DETECTION_MODE}" == face || "${SURVEILLANCE_DETECTION_MODE}" == person_face ]]; then
    [[ "${SURVEILLANCE_DETECTION_MODE}" == person_face || -z "${SURVEILLANCE_DETECTION_MODEL_PATH:-}" ]] || fail "person model override requires SURVEILLANCE_DETECTION_MODE=person; face models use SURVEILLANCE_DETECTION_FACE_MODEL_PATH"
    local face_sha="8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4"
    if [[ -z "${SURVEILLANCE_DETECTION_FACE_MODEL_PATH:-}" ]]; then
      export SURVEILLANCE_DETECTION_FACE_MODEL_PATH="${artifact_dir}/face_detection_yunet_2023mar.onnx"
      export SURVEILLANCE_DETECTION_FACE_MODEL_SHA256="${face_sha}"
      if [[ ! -f "${SURVEILLANCE_DETECTION_FACE_MODEL_PATH}" ]]; then
        status "FETCH" "YuNet development face model"
        curl --fail --location --retry 3 --output "${artifact_dir}/face.download" \
          https://media.githubusercontent.com/media/opencv/opencv_zoo/47534e27c9851bb1128ccc0102f1145e27f23f98/models/face_detection_yunet/face_detection_yunet_2023mar.onnx \
          || fail "face model download failed"
        verify_onnx_model "${artifact_dir}/face.download" "${face_sha}"
        mv "${artifact_dir}/face.download" "${SURVEILLANCE_DETECTION_FACE_MODEL_PATH}"
      fi
    fi
    [[ -n "${SURVEILLANCE_DETECTION_FACE_MODEL_SHA256:-}" ]] || fail "a custom face model requires SURVEILLANCE_DETECTION_FACE_MODEL_SHA256"
    verify_onnx_model "${SURVEILLANCE_DETECTION_FACE_MODEL_PATH}" "${SURVEILLANCE_DETECTION_FACE_MODEL_SHA256}"
  fi
  if [[ "${SURVEILLANCE_DETECTION_MODE}" == person || "${SURVEILLANCE_DETECTION_MODE}" == person_face ]]; then
  local model_sha="427cc366d34e27ff7a03e2899b5e3671425c262ea2291f88bb942bc1cc70b0f7"
  mkdir -p "${artifact_dir}"
  if [[ -z "${SURVEILLANCE_DETECTION_MODEL_PATH:-}" ]]; then
    export SURVEILLANCE_DETECTION_MODEL_PATH="${artifact_dir}/yolox_tiny.onnx"
    export SURVEILLANCE_DETECTION_MODEL_SHA256="${model_sha}"
    if [[ ! -f "${SURVEILLANCE_DETECTION_MODEL_PATH}" ]]; then
      status "FETCH" "YOLOX-Tiny development model"
      curl --fail --location --retry 3 --output "${artifact_dir}/model.download" \
        https://github.com/Megvii-BaseDetection/YOLOX/releases/download/0.1.1rc0/yolox_tiny.onnx \
        || fail "model download failed"
      verify_onnx_model "${artifact_dir}/model.download" "${model_sha}"
      mv "${artifact_dir}/model.download" "${SURVEILLANCE_DETECTION_MODEL_PATH}"
    fi
  fi
  [[ -n "${SURVEILLANCE_DETECTION_MODEL_SHA256:-}" ]] \
    || fail "a custom model requires SURVEILLANCE_DETECTION_MODEL_SHA256"
  verify_onnx_model "${SURVEILLANCE_DETECTION_MODEL_PATH}" "${SURVEILLANCE_DETECTION_MODEL_SHA256}"
  elif [[ "${SURVEILLANCE_DETECTION_MODE}" != face ]]; then
    fail "SURVEILLANCE_DETECTION_MODE must be face, person or person_face"
  fi
  if [[ -z "${ORT_DYLIB_PATH:-}" ]]; then
    local platform library
    case "$(uname -s)-$(uname -m)" in
      Darwin-arm64) platform=osx-arm64; library=libonnxruntime.dylib ;;
      Linux-x86_64) platform=linux-x64; library=libonnxruntime.so ;;
      Linux-aarch64) platform=linux-aarch64; library=libonnxruntime.so ;;
      *) fail "no bundled ONNX runtime for this host; supply ORT_DYLIB_PATH or set SURVEILLANCE_DETECTION_ENABLED=false" ;;
    esac
    local runtime_dir="${artifact_dir}/onnxruntime-${platform}-1.28.3"
    if [[ ! -f "${runtime_dir}/lib/${library}" ]]; then
      status "FETCH" "ONNX Runtime 1.28.3 · ${platform}"
      curl --fail --location --retry 3 --output "${artifact_dir}/runtime.tgz" \
        "https://github.com/microsoft/onnxruntime/releases/download/v1.28.3/onnxruntime-${platform}-1.28.3.tgz" \
        || fail "runtime download failed; supply ORT_DYLIB_PATH to use an installed runtime"
      tar -xzf "${artifact_dir}/runtime.tgz" -C "${artifact_dir}" || fail "runtime extraction failed"
    fi
    export ORT_DYLIB_PATH="${runtime_dir}/lib/${library}"
  fi
  python3 - "${ORT_DYLIB_PATH}" <<'PY' || fail "ONNX runtime cannot load; check ORT_DYLIB_PATH"
import ctypes
import sys
runtime = ctypes.CDLL(sys.argv[1])
runtime.OrtGetApiBase.restype = ctypes.c_void_p
if not runtime.OrtGetApiBase():
    raise SystemExit("ONNX runtime returned no API")
PY
  status "READY" "ONNX ${SURVEILLANCE_DETECTION_MODE} inference"
}

verify_onnx_model() {
  python3 - "$1" "$2" <<'PY' || fail "model missing or SHA-256 mismatch"
import hashlib
import sys
with open(sys.argv[1], 'rb') as model:
    sha = hashlib.sha256()
    for chunk in iter(lambda: model.read(1024 * 1024), b''):
        sha.update(chunk)
    digest = sha.hexdigest()
if digest != sys.argv[2].lower():
    raise SystemExit("model SHA-256 mismatch")
PY
}
