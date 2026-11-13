#!/usr/bin/env python3
"""Run the native app in isolation and use its existing mock-only snapshot hook."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import struct
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[4]
APP = ROOT / "app"


def output(*args):
    return subprocess.check_output(args, cwd=APP, text=True).strip()


def identity():
    metadata = json.loads(output("cargo", "metadata", "--no-deps", "--format-version", "1", "--locked"))
    binary = Path(metadata["target_directory"]) / "debug" / ("td-app.exe" if os.name == "nt" else "td-app")
    package = next(p for p in metadata["packages"] if p["name"] == "td-app")
    with binary.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
    return binary, {
        "head": output("git", "rev-parse", "HEAD"),
        "dirty": bool(output("git", "status", "--porcelain")),
        "package_version": package["version"],
        "rustc": output("rustc", "--version"),
        "cargo": output("cargo", "--version"),
        "profile": "debug",
        "expected_features": ["mock"],
        "binary_name": binary.name,
        "binary_sha256": digest,
    }


def stop(process):
    if process.poll() is not None:
        return "already exited"
    try:
        if os.name == "nt":
            process.send_signal(signal.CTRL_BREAK_EVENT)
        else:
            os.killpg(process.pid, signal.SIGINT)
        process.wait(timeout=10)
        return "interrupt"
    except (OSError, subprocess.TimeoutExpired):
        if os.name == "nt":
            subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"],
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
        if process.poll() is None:
            process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        return "terminated"


def png_size(path):
    # Read only after the encoder has written the complete PNG, including IEND.
    data = path.read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n" or data[-12:] != b"\0\0\0\0IEND\xaeB`\x82":
        raise ValueError("incomplete PNG")
    return struct.unpack(">II", data[16:24])


def launch(args):
    out = (ROOT / args.out).resolve()
    out.relative_to((ROOT / ".verify").resolve())
    out.mkdir(parents=True, exist_ok=False)
    snapshots = out / "snapshots"
    snapshots.mkdir()
    binary, facts = identity()
    (out / "build.json").write_text(json.dumps(facts, indent=2) + "\n", encoding="utf-8")
    # Cargo builds stay in the foreground. Snapshot runs use the binary just identified.
    command = [str(binary), "--mock", args.mock] if args.command == "screenshot" else [
        "cargo", "run", "-p", "td-app", "--features", "mock", "--", "--mock-devices", "2"
    ]
    with tempfile.TemporaryDirectory(prefix="data-", dir=out) as data:
        env = os.environ.copy()
        env.update(TOUCHDECK_APP_DATA_DIR=data, TOUCHDECK_CONFIG_DIR=str(Path(data) / "config"),
                   TOUCHDECK_SNAPSHOT_DIR=str(snapshots), TOUCHDECK_MOCK_PAGE=args.page,
                   TOUCHDECK_TEXT_SCALE="1", SLINT_BACKEND="winit-software")
        # No persistent start-at-sign-in registration or real Device connection is requested.
        options = {"creationflags": subprocess.CREATE_NEW_PROCESS_GROUP} if os.name == "nt" else {"start_new_session": True}
        process = None
        result = {"command": args.command, "page": args.page}
        try:
            if args.command == "run":
                process = subprocess.Popen(command, cwd=APP, env=env, **options)
                result["exit_code"] = process.wait()
                if result["exit_code"]:
                    raise RuntimeError(f"cargo run exited {result['exit_code']}")
            else:
                with (out / "app.log").open("w", encoding="utf-8") as log:
                    process = subprocess.Popen(command, cwd=APP, env=env, stdout=log, stderr=subprocess.STDOUT, **options)
                    started = time.monotonic()
                    deadline = started + args.timeout
                    requested = False
                    while time.monotonic() < deadline:
                        if process.poll() is not None:
                            raise RuntimeError(f"app exited {process.returncode}; see app.log")
                        # Let mock discovery and Slint's first frames settle before requesting.
                        if not requested and time.monotonic() >= started + args.settle_seconds:
                            (snapshots / "request").write_text("screen.png", encoding="utf-8")
                            requested = True
                        try:
                            width, height = png_size(snapshots / "screen.png")
                            result.update(screenshot=(snapshots / "screen.png").relative_to(ROOT).as_posix(),
                                          width=width, height=height)
                            break
                        except (OSError, ValueError, struct.error):
                            time.sleep(0.1)
                    else:
                        raise RuntimeError("snapshot timed out; see app.log; a display is required")
        except KeyboardInterrupt:
            result["interrupted"] = True
        except (OSError, ValueError, RuntimeError) as error:
            result["error"] = str(error)
            raise
        finally:
            if process is not None:
                result["cleanup"] = stop(process)
                result["app_exit_code"] = process.returncode
            if args.command == "run":
                # cargo run may have rebuilt the binary; record the final executable.
                _, facts = identity()
                (out / "build.json").write_text(json.dumps(facts, indent=2) + "\n", encoding="utf-8")
            (out / "result.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
        print(json.dumps(result))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("identity", help="identify the prebuilt debug mock binary and checkout")
    for name in ("run", "screenshot"):
        action = sub.add_parser(name)
        action.add_argument("--out", required=True, help="fresh directory under .verify/")
        action.add_argument("--page", default="home", help="home, setup, settings, integrations, about, device:mock0001 or integration:<id>")
        if name == "screenshot":
            action.add_argument("--mock", default="device,device")
            action.add_argument("--timeout", type=float, default=30)
            action.add_argument("--settle-seconds", type=float, default=3)
    args = parser.parse_args()
    if args.command == "identity":
        print(json.dumps(identity()[1], indent=2))
    else:
        integrations = {"integration:" + i for i in ("cliproxyapi", "this_pc", "media", "volume", "custom", "icon-packs", "brandfetch")}
        if args.page not in {"home", "setup", "settings", "integrations", "about", "device:mock0001"} | integrations:
            parser.error("unsupported page")
        if args.command == "screenshot" and (not 0 <= args.settle_seconds < args.timeout or not set(args.mock.split(",")) <= {"device", "radio", "v1", "board", "fresh"}):
            parser.error("require 0 <= settle-seconds < timeout and valid mock kinds")
        launch(args)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"tdapp: {error}", file=sys.stderr)
        sys.exit(2)
