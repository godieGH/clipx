#!/usr/bin/env python3
"""Setup helper for the fake device emulator.

This script lives in the project-level tests directory and uses the shared proto
schema in ../protos/clipx.proto.
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PROTO = ROOT / "protos" / "clipx.proto"
OUTPUT = ROOT / "tests" / "clipx_pb2.py"


def run(cmd: list[str], env: dict[str, str] | None = None) -> None:
    print("$", " ".join(cmd))
    subprocess.check_call(cmd, cwd=str(ROOT), env=env)


def create_venv(venv_dir: Path) -> Path:
    if not venv_dir.exists():
        print(f"Creating virtual environment at {venv_dir}")
        run([sys.executable, "-m", "venv", str(venv_dir)])

    if os.name == "nt":
        return venv_dir / "Scripts" / "python.exe"
    return venv_dir / "bin" / "python"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Set up the fake-device Python environment")
    parser.add_argument("--venv", action="store_true", help="Create a local virtual environment for the fake-device tools")
    parser.add_argument("--venv-dir", default=str(ROOT / ".venv"), help="Path for the virtual environment")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    python_exe = sys.executable
    env = None

    if args.venv:
        venv_dir = Path(args.venv_dir).resolve()
        python_exe = create_venv(venv_dir)
        print(f"Using Python from {python_exe}")
        try:
            run([str(python_exe), "-m", "pip", "install", "--upgrade", "pip"], env=env)
            run([str(python_exe), "-m", "pip", "install", "protobuf", "grpcio-tools"], env=env)
        except subprocess.CalledProcessError as exc:
            print(f"Package installation failed: {exc}", file=sys.stderr)
            print("You can still try again later or use the system Python installation.", file=sys.stderr)
            return 2

    print("Checking Python protobuf support...")
    try:
        import google.protobuf  # noqa: F401
    except Exception as exc:  # pragma: no cover
        print(f"protobuf runtime is missing: {exc}", file=sys.stderr)
        print("Install it with: python -m pip install --user protobuf", file=sys.stderr)
        return 2

    print("protobuf runtime is available")

    if shutil.which("protoc"):
        print("protoc was found on PATH")
        run(["protoc", "-I", str(PROTO.parent), "--python_out", str(OUTPUT.parent), str(PROTO)], env=env)
        print(f"Generated bindings at {OUTPUT}")
        return 0

    try:
        import grpc_tools.protoc  # noqa: F401
    except Exception:
        print("grpcio-tools is not available, so the bindings were not generated.", file=sys.stderr)
        print("Install it with: python -m pip install --user grpcio-tools", file=sys.stderr)
        print("If protoc is not on PATH, install the Protocol Buffers compiler from https://protobuf.dev/downloads/", file=sys.stderr)
        return 2

    print("grpcio-tools is available; generating bindings via python -m grpc_tools.protoc")
    run([
        str(python_exe),
        "-m",
        "grpc_tools.protoc",
        "-I",
        str(PROTO.parent),
        "--python_out",
        str(OUTPUT.parent),
        str(PROTO),
    ], env=env)
    print(f"Generated bindings at {OUTPUT}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
