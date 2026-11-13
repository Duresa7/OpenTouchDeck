"""net-v2's Device side on the host: tools/host/netlink_test.c runs the production td_netlink_core against every net-v2
vector and shared negative vector, and against pairing, lockout, backoff, record rules and the TCP connection
lifecycle with replaceable sockets.

It compiles ESP-IDF's own TF-PSA-Crypto (the Mbed TLS PSA core the Firmware uses) for the host, so it needs IDF_PATH,
a C compiler (`cl` from an x64 Visual Studio developer terminal on Windows, `cc` elsewhere) and one firmware build
first, which fetches cJSON into firmware/managed_components.
"""

from concurrent.futures import ThreadPoolExecutor
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
REPO = ROOT.parent
WINDOWS = os.name == "nt"
# TF-PSA-Crypto's private header folders, as its CMakeLists.txt lists them.
PRIVATE = ["core", "dispatch", "extras", "platform", "utilities"]
SHIMS = ROOT / "tools/host/netlink"


def psa_tree():
    idf = os.environ.get("IDF_PATH")
    if not idf:
        raise unittest.SkipTest("IDF_PATH must name the ESP-IDF tree, which holds TF-PSA-Crypto")
    return Path(idf) / "components/mbedtls/mbedtls/tf-psa-crypto"


def psa_includes(psa):
    # The shims come first: TF-PSA-Crypto as ESP-IDF patches it includes the wrappers mbedtls/bignum.h and mbedtls/ecp.h.
    folders = ["include", "drivers/builtin/include", *PRIVATE, "drivers/builtin/src"]
    return [SHIMS] + [psa / f for f in folders]


def compile_psa(psa, out):
    """Compiles TF-PSA-Crypto with its default configuration into object files. Its warnings are not ours to fix."""
    sources = []
    for folder in ["core", "drivers/builtin/src", "platform", "utilities", "extras"]:
        sources += sorted((psa / folder).glob("*.c"))
    out.mkdir(parents=True, exist_ok=True)
    includes = psa_includes(psa)
    if WINDOWS:
        command = ["cl", "/nologo", "/c", "/MP", "/W0", "/O2", "/D_CRT_SECURE_NO_WARNINGS", "/Fo" + str(out) + "\\"]
        command += ["/I" + str(p) for p in includes] + [str(s) for s in sources]
        subprocess.run(command, cwd=out, check=True)
        return sorted(out.glob("*.obj"))
    cc = os.environ.get("CC", "cc")

    def one(source):
        obj = out / (source.parent.name + "_" + source.stem + ".o")
        subprocess.run([cc, "-c", "-O2", "-w", "-std=c99", *("-I" + str(p) for p in includes), str(source), "-o",
                        str(obj)], check=True)
        return obj

    with ThreadPoolExecutor(max_workers=os.cpu_count() or 2) as pool:
        return list(pool.map(one, sources))


class NetlinkTest(unittest.TestCase):
    def test_netlink_core_on_the_host(self):
        psa = psa_tree()
        cjson = ROOT / "managed_components/espressif__cjson/cJSON"
        core = ROOT / "components/td_netlink_core"
        link_core = ROOT / "components/td_link_core"
        host = ROOT / "tools/host"
        with tempfile.TemporaryDirectory(dir=os.environ.get("TD_TEST_OUT")) as tmp:
            out = Path(tmp)
            objects = compile_psa(psa, out / "psa")
            includes = [core / "include", link_core / "include", cjson] + psa_includes(psa)
            sources = [host / "netlink_test.c", cjson / "cJSON.c"]
            sources += sorted(core.glob("*.c"))
            sources += [link_core / "td_frame.c", link_core / "td_crc16.c"]
            exe = out / ("netlink.exe" if WINDOWS else "netlink")
            if WINDOWS:
                command = ["cl", "/nologo", "/std:clatest", "/DTD_NETLINK_TEST", "/W3", "/WX", "/D_CRT_SECURE_NO_WARNINGS", "/utf-8"]
                command += ["/I" + str(p) for p in includes] + [str(p) for p in sources] + [str(o) for o in objects]
                command += ["/Fe:" + str(exe), "/link", "bcrypt.lib"]
            else:
                command = [os.environ.get("CC", "cc"), "-std=c11", "-DTD_NETLINK_TEST", "-D_POSIX_C_SOURCE=200809L", "-Wall", "-Wextra",
                           "-Werror"]
                command += ["-I" + str(p) for p in includes] + [str(p) for p in sources] + [str(o) for o in objects]
                command += ["-lm", "-o", str(exe)]
            subprocess.run(command, cwd=out, check=True)
            vectors = REPO / "schema/protocol/net-v2-vectors.json"
            run = subprocess.run([str(exe), str(vectors)], cwd=out, timeout=300, capture_output=True, text=True)
            print(run.stdout)
            self.assertEqual(run.returncode, 0, run.stdout[-4000:] + run.stderr)
            self.assertIn("tests passed", run.stdout)


if __name__ == "__main__":
    unittest.main()
