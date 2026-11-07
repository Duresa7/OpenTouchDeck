import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class WifiTest(unittest.TestCase):
    """The radio build's Wi-Fi station, its NVS store, its messages and the console's echo mask: tools/host/wifi_test.c."""

    def test_wifi_station(self):
        with tempfile.TemporaryDirectory(dir=os.environ.get("TD_TEST_OUT")) as tmp:
            out = Path(tmp)
            host = ROOT / "tools/host"
            components = ROOT / "components"
            radio = components / "td_radio"
            cjson = ROOT / "managed_components/espressif__cjson/cJSON"
            # The Wi-Fi stand-ins come first, then the link's FreeRTOS ones.
            includes = [host / "wifi", host / "link", cjson, radio]
            includes += [components / c / "include" for c in ["td_radio", "td_console", "td_link"]]
            sources = [host / "wifi_test.c", radio / "td_wifi_policy.c", radio / "td_wifi_store.c",
                       radio / "td_wifi_msgs.c", components / "td_console/td_rx_sync.c", cjson / "cJSON.c"]
            exe = out / ("test.exe" if os.name == "nt" else "test")
            if os.name == "nt":
                command = ["cl", "/nologo", "/std:clatest", "/W3", "/WX", "/D_CRT_SECURE_NO_WARNINGS"]
                command += ["/I" + str(p) for p in includes] + [str(p) for p in sources] + ["/Fe:" + str(exe)]
            else:
                command = [os.environ.get("CC", "cc"), "-std=c11", "-D_POSIX_C_SOURCE=200809L", "-Wall", "-Wextra",
                           "-Werror"]
                command += ["-I" + str(p) for p in includes] + [str(p) for p in sources] + ["-lm", "-o", str(exe)]
            subprocess.run(command, cwd=out, check=True)
            subprocess.run([str(exe)], cwd=out, check=True)


if __name__ == "__main__":
    unittest.main()
