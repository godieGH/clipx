#!/usr/bin/env python3
"""Cross-platform fake ClipX device emulator.

This script lives in the project-level tests directory and uses the shared proto
schema in ../protos/clipx.proto.
"""

from __future__ import annotations

import argparse
import hashlib
import os
import socket
import sys
import threading
import time
from pathlib import Path
from typing import Optional

ROOT = Path(__file__).resolve().parent.parent


def _load_generated_module():
    """Load the generated protobuf module from the project tests directory."""
    generated = ROOT / "tests" / "clipx_pb2.py"
    if generated.exists():
        import importlib.util

        spec = importlib.util.spec_from_file_location("clipx_pb2", generated)
        if spec and spec.loader:
            module = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(module)
            return module
    return None


def _build_error_message() -> str:
    return (
        "The fake device needs the protobuf runtime and generated bindings.\n"
        "Install runtime with: python -m pip install --user protobuf\n"
        "If you need to regenerate bindings, install: python -m pip install --user grpcio-tools\n"
        "Then run: python tests/setup_fake_device.py"
    )


def _import_clipx_proto():
    try:
        import google.protobuf  # noqa: F401
    except Exception as exc:  # pragma: no cover
        raise RuntimeError(_build_error_message()) from exc

    module = _load_generated_module()
    if module is not None:
        return module

    raise RuntimeError(
        "No generated protobuf module was found.\n"
        f"Expected: {ROOT / 'tests' / 'clipx_pb2.py'}\n"
        f"Source proto: {ROOT / 'protos' / 'clipx.proto'}\n"
        "Run: python tests/setup_fake_device.py"
    )


class FakeDevice:
    def __init__(self, name: str, device_type: int, port: int = 9999, ws_port: int = 8080):
        self.name = name
        self.device_type = device_type
        self.port = port
        self.ws_port = ws_port
        self.private_key = os.urandom(32)
        self.public_key = self.private_key
        self.fingerprint = hashlib.sha256(self.public_key).digest()
        self.proto = _import_clipx_proto()

    def announce_message(self):
        announce = self.proto.Announce()
        announce.fingerprint = self.fingerprint
        announce.device_name = self.name
        announce.device_type = self.device_type
        announce.ws_port = self.ws_port
        return announce.SerializeToString()

    def broadcast_loop(self, interval: float = 2.0, stop_event: Optional[threading.Event] = None):
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sock.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
        sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        try:
            while stop_event is None or not stop_event.is_set():
                sock.sendto(self.announce_message(), ("255.255.255.255", self.port))
                time.sleep(interval)
        finally:
            sock.close()

    def listen_loop(self, stop_event: Optional[threading.Event] = None):
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        sock.bind(("0.0.0.0", self.port))
        print(f"Listening for announcements on {self.port}...")
        try:
            while stop_event is None or not stop_event.is_set():
                data, addr = sock.recvfrom(4096)
                try:
                    message = self.proto.Announce()
                    message.ParseFromString(data)
                    print(f"Received announce from {addr}: {message.device_name} ({message.device_type})")
                except Exception as exc:  # pragma: no cover
                    print(f"Could not decode announcement from {addr}: {exc}")
        finally:
            sock.close()

    def simulate_pair(self, target_id: str):
        challenge = self.proto.PairChallenge()
        challenge.request_id = os.urandom(16)
        challenge.nonce = os.urandom(16)
        challenge.initiator_public = self.public_key
        print(f"Pairing request simulated for target={target_id}")
        print(f"Challenge nonce={challenge.nonce.hex()}")
        return challenge


def _parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Fake ClipX device emulator")
    parser.add_argument("--name", default="Fake Device", help="The device name to advertise")
    parser.add_argument("--mode", choices=["broadcast", "listen", "pair", "all"], default="all")
    parser.add_argument("--port", type=int, default=9999, help="UDP port for discovery traffic")
    parser.add_argument("--ws-port", type=int, default=8080, help="WebSocket port advertised in the announcement")
    parser.add_argument("--device-type", type=int, default=2, help="Proto device type code (default: ANDROID=2)")
    parser.add_argument("--interval", type=float, default=2.0, help="Broadcast interval in seconds")
    parser.add_argument("--target", default="", help="Device ID to simulate pairing with")
    return parser.parse_args()


def main() -> int:
    args = _parse_args()
    try:
        device = FakeDevice(args.name, args.device_type, port=args.port, ws_port=args.ws_port)
    except RuntimeError as exc:
        print(str(exc), file=sys.stderr)
        return 2

    if args.mode == "pair":
        device.simulate_pair(args.target)
        return 0

    if args.mode in {"broadcast", "all"}:
        print(f"Broadcasting announcements as {args.name} every {args.interval}s")
        broadcast_thread = threading.Thread(target=device.broadcast_loop, args=(args.interval,), daemon=True)
        broadcast_thread.start()

    if args.mode in {"listen", "all"}:
        device.listen_loop()
        return 0

    if args.mode == "broadcast":
        try:
            while True:
                time.sleep(1)
        except KeyboardInterrupt:
            print("Stopped")
    return 0


if __name__ == "__main__":
    sys.exit(main())
