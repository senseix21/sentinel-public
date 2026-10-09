"""Regression tests for the development launcher's ONNX selection."""
import hashlib
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

HELPER = Path(__file__).with_name("onnx-defaults.sh").resolve()


class OnnxDefaultsTests(unittest.TestCase):
    def run_helper(self, root, **overrides):
        env = {k: v for k, v in os.environ.items()
               if not k.startswith("SURVEILLANCE_") and k != "ORT_DYLIB_PATH"}
        env["SURVEILLANCE_DETECTION_MODE"] = "person"
        env.update(overrides)
        return subprocess.run(
            ["bash", "-c", 'set -euo pipefail; repo_root="$1"; '
             'status() { :; }; fail() { echo "$*" >&2; exit 1; }; '
             'source "$2"; prepare_onnx; '
             'echo "${SURVEILLANCE_CARGO_FEATURES:-disabled}"',
             "test", str(root), str(HELPER)],
            env=env, capture_output=True, text=True, check=False,
        )

    def test_disabled_skips_artifacts_and_feature(self):
        with tempfile.TemporaryDirectory() as root:
            result = self.run_helper(root, SURVEILLANCE_DETECTION_ENABLED="false")
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout.strip(), "disabled")
            self.assertFalse((Path(root) / ".local").exists())

    def test_custom_model_requires_checksum(self):
        with tempfile.TemporaryDirectory() as root:
            result = self.run_helper(root, SURVEILLANCE_DETECTION_MODEL_PATH="custom.onnx")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("requires SURVEILLANCE_DETECTION_MODEL_SHA256", result.stderr)

    def test_bad_checksum_refused_before_loading_runtime(self):
        with tempfile.TemporaryDirectory() as root:
            model = Path(root) / "custom.onnx"
            model.write_bytes(b"model")
            result = self.run_helper(
                root, SURVEILLANCE_DETECTION_MODEL_PATH=str(model),
                SURVEILLANCE_DETECTION_MODEL_SHA256="0" * 64,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("SHA-256 mismatch", result.stderr)

    def test_missing_runtime_refused_before_server_start(self):
        with tempfile.TemporaryDirectory() as root:
            model = Path(root) / "custom.onnx"
            model.write_bytes(b"model")
            result = self.run_helper(
                root, SURVEILLANCE_DETECTION_MODEL_PATH=str(model),
                SURVEILLANCE_DETECTION_MODEL_SHA256=hashlib.sha256(b"model").hexdigest(),
                ORT_DYLIB_PATH=str(Path(root) / "missing.dylib"),
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("ONNX runtime cannot load", result.stderr)

    def test_face_override_requires_a_face_checksum(self):
        with tempfile.TemporaryDirectory() as root:
            result = self.run_helper(root, SURVEILLANCE_DETECTION_MODE="face", SURVEILLANCE_DETECTION_FACE_MODEL_PATH="face.onnx")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("requires SURVEILLANCE_DETECTION_FACE_MODEL_SHA256", result.stderr)

    def test_face_mode_never_reinterprets_a_person_artifact(self):
        with tempfile.TemporaryDirectory() as root:
            result = self.run_helper(root, SURVEILLANCE_DETECTION_MODE="face", SURVEILLANCE_DETECTION_MODEL_PATH="person.onnx")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("person model override requires", result.stderr)

    def test_invalid_mode_is_refused(self):
        with tempfile.TemporaryDirectory() as root:
            result = self.run_helper(root, SURVEILLANCE_DETECTION_MODE="unknown")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("must be face, person or person_face", result.stderr)


class LauncherBuildTests(unittest.TestCase):
    def test_build_arguments_on_macos_bash_with_empty_arrays(self):
        launcher = HELPER.with_name("local-webcam-stack.sh").read_text()
        build = next(line for line in launcher.splitlines()
                     if line.startswith('(cd "${repo_root}" && cargo build'))
        build = build.rstrip().removesuffix("\\").rstrip()
        for profile, features, expected in (
            ("()", "()", ["build", "-p", "surveillance-server"]),
            ("()", "(--features onnx-inference)",
             ["build", "-p", "surveillance-server", "--features", "onnx-inference"]),
            ("(--release)", "(--features onnx-inference)",
             ["build", "-p", "surveillance-server", "--release", "--features", "onnx-inference"]),
        ):
            with self.subTest(profile=profile, features=features):
                result = subprocess.run(
                    ["/bin/bash", "-c", 'set -euo pipefail; repo_root="$PWD"; '
                     'cargo() { printf "%s\\n" "$@"; }; '
                     f'cargo_profile={profile}; cargo_features={features}; {build}'],
                    capture_output=True, text=True, check=False,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout.splitlines(), expected)
