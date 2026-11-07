"""Run the radio build's QEMU test image and pair with it over net-v2, for CI.

Build the image first, in a build directory of its own:

    idf.py -C firmware -B build/firmware-qemu -DTD_RADIO_BUILD=1 -DTD_QEMU_TEST=1 build
    python firmware/tools/qemu_net_check.py build/firmware-qemu

The image runs the link and the net-v2 service over QEMU's emulated Ethernet (open_eth) and opens a pairing window at
boot. QEMU's user network forwards 127.0.0.1:47820 to it. firmware/tools/qemu/net_v2_client.mjs, built on the
reference in schema/protocol/net-ref, pairs with the code the image logs, opens a session and exchanges a hello and
the other link checks in records. Needs IDF_PATH, qemu-system-xtensa (idf_tools.py install qemu-xtensa) and Node 22+.
"""

import argparse
import json
import os
import queue
import shutil
import subprocess
import sys
import threading
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
CLIENT = HERE / "qemu" / "net_v2_client.mjs"
PORT = 47820
CODE_PREFIX = "qemu-test pairing code "


def qemu_target():
    sys.path.insert(0, str(Path(os.environ["IDF_PATH"]) / "tools"))
    from idf_py_actions.qemu_ext import QEMU_TARGETS  # noqa: E402  (IDF's own QEMU settings)

    return QEMU_TARGETS["esp32s3"]


def pump(stream, prefix, lines, log):
    for raw in iter(stream.readline, b""):
        line = raw.decode("utf-8", "replace").rstrip("\r\n")
        log.append(f"{prefix}{line}")
        lines.put((prefix, line))
    lines.put((prefix, None))


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("build_dir", type=Path)
    parser.add_argument("--timeout", type=float, default=300.0)
    args = parser.parse_args()
    build = args.build_dir.resolve()
    config = json.loads((build / "config" / "sdkconfig.json").read_text(encoding="utf-8"))
    if not config.get("TD_QEMU_TEST"):
        sys.exit(f"{build} is not the QEMU test image: build it with -DTD_RADIO_BUILD=1 -DTD_QEMU_TEST=1")

    target = qemu_target()
    flash = build / "qemu_flash.bin"
    subprocess.run([sys.executable, "-m", "esptool", "--chip=esp32s3", "merge-bin", f"--output={flash}",
                    f"--pad-to-size={config['ESPTOOLPY_FLASHSIZE']}", "@flash_args"], cwd=build, check=True,
                   stdout=subprocess.DEVNULL)
    efuse = build / "qemu_efuse.bin"
    efuse.write_bytes(target.default_efuse)

    program = shutil.which(target.qemu_prog)
    if not program:
        sys.exit(f"{target.qemu_prog} not found: python $IDF_PATH/tools/idf_tools.py install qemu-xtensa")
    qemu_args = [program, *target.qemu_args.split(), "-drive", f"file={flash},if=mtd,format=raw",
                 "-drive", f"file={efuse},if=none,format=raw,id=efuse",
                 "-global", "driver=nvram.esp32s3.efuse,property=drive,value=efuse",
                 "-global", "driver=timer.esp32s3.timg,property=wdt_disable,value=true",
                 "-nic", f"user,model=open_eth,hostfwd=tcp:127.0.0.1:{PORT}-:{PORT}",
                 "-nographic", "-serial", "stdio", "-monitor", "none"]
    if config.get("SPIRAM_MODE_OCT"):
        qemu_args += ["-global", "driver=ssi_psram,property=is_octal,value=true"]

    lines = queue.Queue()
    log = []
    qemu = subprocess.Popen(qemu_args, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    threading.Thread(target=pump, args=(qemu.stdout, "qemu| ", lines, log), daemon=True).start()
    client = None
    passed = False
    try:
        deadline = time.monotonic() + args.timeout
        code = None
        need_code = False
        listening = False
        while time.monotonic() < deadline:
            try:
                source, line = lines.get(timeout=1.0)
            except queue.Empty:
                continue
            if line is None:
                if source == "qemu| ":
                    raise SystemExit("QEMU exited")
                break
            if source == "qemu| ":
                if not listening and f"listening on {PORT}" in line:
                    listening = True
                    client = subprocess.Popen(["node", str(CLIENT), str(PORT)], stdin=subprocess.PIPE,
                                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
                    threading.Thread(target=pump, args=(client.stdout, "node| ", lines, log), daemon=True).start()
                if CODE_PREFIX in line:
                    code = line.split(CODE_PREFIX, 1)[1].split()[0]
                if "Guru Meditation" in line or "abort()" in line:
                    raise SystemExit("the image crashed")
            elif line == "need-code":
                need_code = True
            elif line == "net-v2 qemu check passed":
                passed = True
            if need_code and code and client:
                client.stdin.write(f"{code}\n".encode())
                client.stdin.flush()
                need_code = False
        if client:
            try:
                client.wait(timeout=max(1.0, deadline - time.monotonic()))
            except subprocess.TimeoutExpired:
                pass
        if not passed or not client or client.returncode != 0:
            raise SystemExit("net-v2 QEMU check failed" if listening else "the image never listened on TCP")
        print("\n".join(line for line in log if line.startswith("node| ")))
        print("net-v2 QEMU check passed")
    except SystemExit:
        print("\n".join(log[-200:]), file=sys.stderr)
        raise
    finally:
        for process in (client, qemu):
            if process and process.poll() is None:
                process.kill()
                process.wait()


if __name__ == "__main__":
    main()
