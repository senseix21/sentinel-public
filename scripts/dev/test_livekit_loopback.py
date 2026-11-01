"""Qualify the launcher's actual LiveKit config when the binary is available."""
import json
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import time
import unittest


class LiveKitLoopbackTest(unittest.TestCase):
    def test_macos_auto_uses_host_interface_instead_of_socket_route(self):
        import contextlib
        import io
        from unittest.mock import patch, call
        helper = Path(__file__).with_name("livekit-config.sh").read_text()
        script = helper.split("<<'PY'\n", 1)[1].split("\nPY\n", 1)[0]
        output = io.StringIO()
        with patch("sys.platform", "darwin"), patch("sys.argv", ["test", "auto"]), \
             patch("subprocess.check_output", side_effect=["interface: test-nic\n", "127.0.0.1\n"]) as commands, \
             contextlib.redirect_stdout(output):
            exec(compile(script, "livekit-ip-selection", "exec"), {})
        self.assertEqual(output.getvalue().strip(), "127.0.0.1")
        self.assertEqual(commands.call_args_list, [call(["route", "-n", "get", "default"], text=True),
                                                   call(["ipconfig", "getifaddr", "test-nic"], text=True)])

    def test_linux_auto_public_route_falls_back_to_loopback(self):
        import contextlib
        import io
        from unittest.mock import patch
        helper = Path(__file__).with_name("livekit-config.sh").read_text()
        script = helper.split("<<'PY'\n", 1)[1].split("\nPY\n", 1)[0]
        output = io.StringIO()
        with patch("sys.platform", "linux"), patch("sys.argv", ["test", "auto"]), \
             patch("socket.socket") as sockets, contextlib.redirect_stdout(output):
            probe = sockets.return_value.__enter__.return_value
            probe.getsockname.return_value = ("172.104.62.66", 12345)
            exec(compile(script, "livekit-ip-selection", "exec"), {})
            probe.bind.assert_called_once_with(("127.0.0.1", 0))
        self.assertEqual(output.getvalue().strip(), "127.0.0.1")

    def test_linux_auto_keeps_a_private_route_address(self):
        import contextlib
        import io
        from unittest.mock import patch
        helper = Path(__file__).with_name("livekit-config.sh").read_text()
        script = helper.split("<<'PY'\n", 1)[1].split("\nPY\n", 1)[0]
        output = io.StringIO()
        with patch("sys.platform", "linux"), patch("sys.argv", ["test", "auto"]), \
             patch("socket.socket") as sockets, contextlib.redirect_stdout(output):
            probe = sockets.return_value.__enter__.return_value
            probe.getsockname.return_value = ("10.20.0.5", 12345)
            exec(compile(script, "livekit-ip-selection", "exec"), {})
            probe.bind.assert_called_once_with(("10.20.0.5", 0))
        self.assertEqual(output.getvalue().strip(), "10.20.0.5")

    @unittest.skipUnless(shutil.which("livekit-server") and shutil.which("lsof"),
                         "requires livekit-server and lsof")
    def test_local_stack_advertises_loopback_media_address(self):
        helper = Path(__file__).with_name("livekit-config.sh").resolve()
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            port = reservation.getsockname()[1]
        expanded = subprocess.run(
            ["/bin/bash", "-c", 'set -euo pipefail; livekit_port="$1"; '
             'LIVEKIT_MEDIA_IP=127.0.0.1; fail() { echo "$*" >&2; exit 1; }; '
             'source "$2"; prepare_livekit_config; printf "%s" "$livekit_config"',
             "test", str(port), str(helper)],
            check=True, capture_output=True, text=True,
        ).stdout
        with tempfile.TemporaryFile(mode="w+") as log:
            child = subprocess.Popen(
                ["livekit-server", "--dev", "--bind", "127.0.0.1",
                 "--config-body", expanded, "--keys", "testkey: test-secret"],
                stdout=log, stderr=subprocess.STDOUT,
            )
            try:
                deadline = time.monotonic() + 10
                output = ""
                while time.monotonic() < deadline:
                    log.seek(0)
                    output = log.read()
                    if "starting LiveKit server" in output or child.poll() is not None:
                        break
                    time.sleep(0.05)
                self.assertIsNone(child.poll(), output)
                startup = next((line for line in output.splitlines()
                                if "starting LiveKit server" in line), None)
                self.assertIsNotNone(startup, output)
                metadata = json.loads(startup[startup.index("{"):])
                self.assertEqual(metadata["nodeIP"], "127.0.0.1")
                self.assertEqual(metadata["bindAddresses"], ["127.0.0.1"])
                self.assertEqual(metadata.get("rtc.portTCP", 0), 0)
                listeners = subprocess.run(
                    ["lsof", "-nP", "-a", "-p", str(child.pid), "-iTCP",
                     "-sTCP:LISTEN", "-Fn"],
                    check=True, capture_output=True, text=True,
                ).stdout
                endpoints = [line[1:] for line in listeners.splitlines() if line.startswith("n")]
                self.assertEqual(endpoints, [f"127.0.0.1:{port}"])
                self.assertEqual(metadata["rtc.portUDP"]["Start"], port + 2)
            finally:
                child.terminate()
                try:
                    child.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait()


class MediaAddressTest(unittest.TestCase):
    def test_invalid_or_unassigned_media_addresses_are_refused(self):
        helper = Path(__file__).with_name("livekit-config.sh").resolve()
        for address in ["0.0.0.0", "224.0.0.1", "8.8.8.8", "not-an-address", "192.0.2.1", "127.0.0.1;exit"]:
            with self.subTest(address=address):
                result = subprocess.run(
                    ["/bin/bash", "-c", 'set -euo pipefail; livekit_port=17880; '
                     'LIVEKIT_MEDIA_IP="$1"; fail() { echo "$*" >&2; exit 1; }; '
                     'source "$2"; prepare_livekit_config', "test", address, str(helper)],
                    capture_output=True, text=True, check=False,
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("invalid LIVEKIT_MEDIA_IP", result.stderr)
