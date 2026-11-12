#!/usr/bin/env bash
# Local signalling stays on loopback; media uses a selected local IPv4 address.
prepare_livekit_config() {
  livekit_media_ip="$(python3 - "${LIVEKIT_MEDIA_IP:-auto}" <<'PY'
import ipaddress
import socket
import sys
import re
import subprocess
value = sys.argv[1]
if value == 'auto':
    if sys.platform == 'darwin':
        try:
            # Query the host route: a process UDP socket can report a VPN or
            # tool transport address that the browser cannot reach.
            route = subprocess.check_output(['route', '-n', 'get', 'default'], text=True)
            match = re.search(r'interface:\s*(\S+)', route)
            if match is None:
                raise ValueError('no default interface')
            value = subprocess.check_output(['ipconfig', 'getifaddr', match.group(1)], text=True).strip()
        except (OSError, subprocess.CalledProcessError, ValueError):
            value = '127.0.0.1'
    else:
        try:
            with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as route:
                route.connect(('1.1.1.1', 9))
                value = route.getsockname()[0]
        except OSError:
            value = '127.0.0.1'
address = ipaddress.IPv4Address(value)
if address.is_unspecified or address.is_multicast or address.is_link_local or not address.is_private:
    raise SystemExit('LIVEKIT_MEDIA_IP must be a private or loopback IPv4 address')
# Reject an address that is not assigned to this machine before starting services.
with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as probe:
    probe.bind((str(address), 0))
print(address)
PY
)" || fail "invalid LIVEKIT_MEDIA_IP; choose a private IPv4 address assigned to this machine"
  livekit_config="port: ${livekit_port}
rtc:
  node_ip: ${livekit_media_ip}
  use_external_ip: false
  enable_loopback_candidate: true
  tcp_port: 0
  udp_port: $((livekit_port + 2))
  ips:
    includes:
      - ${livekit_media_ip}/32"
}
