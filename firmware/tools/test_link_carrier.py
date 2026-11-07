import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class LinkCarrierTest(unittest.TestCase):
    """The link's replies and events leave on the carrier they should: tools/host/link_carrier_test.c."""

    def build_and_run(self, defines):
        with tempfile.TemporaryDirectory(dir=os.environ.get("TD_TEST_OUT")) as tmp:
            out = Path(tmp)
            host = ROOT / "tools/host"
            components = ROOT / "components"
            cjson = ROOT / "managed_components/espressif__cjson/cJSON"
            # The link stand-ins come first, so they win over the shared host headers.
            includes = [host / "link", host / "include", cjson, components / "td_link"]
            includes += [components / c / "include" for c in
                         ["td_link", "td_link_core", "td_console", "td_layout", "td_ui_core", "keys", "ui",
                          "td_store", "td_idle"]]
            sources = [host / "link_carrier_test.c", components / "td_link/td_link_msgs.c", cjson / "cJSON.c"]
            sources += [components / "td_link_core" / f for f in ["td_frame.c", "td_crc16.c", "td_v2_json.c", "td_input.c"]]
            exe = out / ("test.exe" if os.name == "nt" else "test")
            if os.name == "nt":
                # C4244: the dispatcher hands cJSON an int64_t time, which GCC takes without a warning. The forced
                # include gives every file the host's GCC atomics.
                command = ["cl", "/nologo", "/std:clatest", "/W3", "/WX", "/wd4244", "/D_CRT_SECURE_NO_WARNINGS",
                           "/FIfreertos/FreeRTOS.h"]
                command += ["/D" + define for define in defines]
                command += ["/I" + str(p) for p in includes] + [str(p) for p in sources] + ["/Fe:" + str(exe)]
            else:
                command = [os.environ.get("CC", "cc"), "-std=c11", "-D_POSIX_C_SOURCE=200809L", "-Wall", "-Wextra",
                           "-Werror"]
                command += ["-D" + define for define in defines]
                command += ["-I" + str(p) for p in includes] + [str(p) for p in sources] + ["-lm", "-o", str(exe)]
            subprocess.run(command, cwd=out, check=True)
            subprocess.run([str(exe)], cwd=out, check=True)

    def test_link_messages_through_carriers(self):
        self.build_and_run([])

    def test_radio_build_hello(self):
        # The radio build compiles the same link with CONFIG_TD_RADIO_BUILD set, which marks its v2 hello.
        self.build_and_run(["CONFIG_TD_RADIO_BUILD=1"])


if __name__ == "__main__":
    unittest.main()
