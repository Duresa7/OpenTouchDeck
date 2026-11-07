import contextlib
import importlib.util
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("packs", Path(__file__).parents[1] / "generate_packs.py")
packs = importlib.util.module_from_spec(spec)
spec.loader.exec_module(packs)


def tarball(info):
    out = io.BytesIO()
    with tarfile.open(fileobj=out, mode="w:gz") as tar:
        for name, data in [("package/info.json", json.dumps(info).encode()), ("package/icons.json", b"{}")]:
            member = tarfile.TarInfo(name)
            member.size = len(data)
            tar.addfile(member, io.BytesIO(data))
    return out.getvalue()


def info(prefix, spdx="MIT"):
    return dict(prefix=prefix, name=prefix.title(), total=3, author=dict(name="Someone", url="https://example.org"),
                license=dict(title=spdx, spdx=spdx, url="https://example.org/license"), samples=list("abcdefgh"), palette=True)


def meta(prefix, version, data):
    return dict(version=version, dist=dict(tarball=f"{packs.REGISTRY}{packs.package(prefix)}/-/{prefix}-{version}.tgz",
                                           integrity=packs.integrity_of(data), unpackedSize=1234))


class PacksTest(unittest.TestCase):
    def test_deterministic_fields_and_order(self):
        data = tarball(info("zed"))
        rows = [("zed", True, meta("zed", "1.0.0", data), info("zed", "CC-BY-4.0")), ("abc", False, meta("abc", "2.0.0", data), info("abc"))]
        first = packs.generate(rows)
        self.assertEqual(first, packs.generate(rows))
        self.assertNotIn(b"\r", first)
        zed, abc = json.loads(first)["packs"]
        self.assertEqual(zed["prefix"], "zed")
        self.assertEqual(zed["package"], "@iconify-json/zed")
        self.assertEqual(zed["integrity"], packs.integrity_of(data))
        self.assertTrue(zed["attribution"])
        self.assertTrue(zed["brands"])
        self.assertEqual(len(zed["samples"]), 6)
        self.assertFalse(abc["attribution"])
        self.assertEqual(abc["license"]["spdx"], "MIT")

    def test_refuses_foreign_tarballs_and_wrong_prefixes(self):
        data = tarball(info("zed"))
        foreign = meta("zed", "1.0.0", data)
        foreign["dist"]["tarball"] = "https://example.org/zed.tgz"
        with self.assertRaises(ValueError):
            packs.generate([("zed", False, foreign, info("zed"))])
        with self.assertRaises(ValueError):
            packs.generate([("zed", False, meta("zed", "1.0.0", data), info("other"))])

    def test_check_passes_on_current_output_and_fails_on_drift(self):
        with tempfile.TemporaryDirectory() as temp:
            inputs = Path(temp) / "inputs"
            inputs.mkdir()
            pinned = [("one", "1.0.0", True), ("two", "2.0.0", False)]
            for prefix, version, _ in pinned:
                data = tarball(info(prefix))
                (inputs / f"{prefix}-{version}.tgz").write_bytes(data)
                (inputs / f"{prefix}-{version}.json").write_text(json.dumps(meta(prefix, version, data)))
            output = Path(temp) / "packs.json"
            old = (packs.PACKS, packs.OUTPUT)
            packs.PACKS, packs.OUTPUT = pinned, output
            try:
                with contextlib.redirect_stdout(io.StringIO()):
                    packs.main(["--inputs", str(inputs)])
                    packs.main(["--inputs", str(inputs), "--check"])
                    self.assertEqual([p["prefix"] for p in json.loads(output.read_bytes())["packs"]], ["one", "two"])
                    output.write_bytes(output.read_bytes().replace(b'"one"', b'"uno"'))
                    with self.assertRaises(AssertionError):
                        packs.main(["--inputs", str(inputs), "--check"])
                    (inputs / "two-2.0.0.tgz").write_bytes(tarball(info("two", "ISC")))
                    with self.assertRaises(ValueError):
                        packs.main(["--inputs", str(inputs)])
            finally:
                packs.PACKS, packs.OUTPUT = old

    def test_committed_list_is_pinned_to_the_registry(self):
        data = json.loads(packs.OUTPUT.read_bytes())["packs"]
        self.assertEqual([p["prefix"] for p in data], [p[0] for p in packs.PACKS])
        self.assertNotIn("simple-icons", [p["prefix"] for p in data])
        for pack, (prefix, version, brands) in zip(data, packs.PACKS):
            self.assertEqual((pack["version"], pack["brands"]), (version, brands))
            self.assertTrue(pack["tarball"].startswith(packs.REGISTRY))
            self.assertTrue(pack["integrity"].startswith("sha512-"))
        self.assertTrue(next(p for p in data if p["prefix"] == "fa6-brands")["attribution"])


if __name__ == "__main__":
    unittest.main()
