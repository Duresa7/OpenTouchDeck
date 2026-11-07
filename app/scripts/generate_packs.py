"""Regenerate the curated Icon pack list from pinned npm packages (Python stdlib only).

Each pack is an `@iconify-json/<prefix>` package at a pinned version. The Companion downloads
the same tarball when someone installs the pack and checks it against the integrity recorded
here. Use --inputs DIR to reuse downloaded inputs, including for offline reproducibility checks.
"""
import argparse
import base64
import hashlib
import io
import json
from pathlib import Path
import tarfile
import urllib.request

REGISTRY = "https://registry.npmjs.org/"
# (prefix, pinned version, brand logos). simple-icons is left out: it ships as a catalog.
PACKS = [
    ("logos", "1.2.19", True),
    ("devicon", "1.2.77", True),
    ("skill-icons", "1.2.4", True),
    ("fa6-brands", "1.2.6", True),
    ("mdi", "1.2.3", False),
    ("tabler", "1.2.43", False),
    ("lucide", "1.2.151", False),
]
OUTPUT = Path(__file__).resolve().parents[1] / "crates/td-host/src/packs.json"


def package(prefix):
    return f"@iconify-json/{prefix}"


def integrity_of(data):
    return "sha512-" + base64.b64encode(hashlib.sha512(data).digest()).decode()


def read_info(tarball):
    with tarfile.open(fileobj=io.BytesIO(tarball), mode="r:gz") as tar:
        return json.load(tar.extractfile("package/info.json"))


def entry(prefix, brands, meta, info):
    """One pack, from the registry's version metadata and the package's info.json."""
    dist = meta["dist"]
    tarball = dist["tarball"]
    if not tarball.startswith(REGISTRY):
        raise ValueError(f"{prefix}: tarball is not on the npm registry")
    if not dist["integrity"].startswith("sha512-"):
        raise ValueError(f"{prefix}: integrity is not sha512")
    if info["prefix"] != prefix:
        raise ValueError(f"{prefix}: info.json names {info['prefix']}")
    license = info["license"]
    spdx = license.get("spdx") or license["title"]
    author = info.get("author", {})
    return dict(
        prefix=prefix,
        name=info["name"],
        package=package(prefix),
        version=meta["version"],
        tarball=tarball,
        integrity=dist["integrity"],
        unpacked_size=dist["unpackedSize"],
        icons=info["total"],
        license=dict(title=license["title"], spdx=spdx, url=license.get("url")),
        author=dict(name=author.get("name", ""), url=author.get("url")),
        attribution=spdx.upper().startswith("CC-BY"),
        brands=brands,
        palette=bool(info.get("palette", False)),
        samples=list(info.get("samples", []))[:6],
    )


def generate(packs):
    """`packs` holds (prefix, brands, version metadata, info.json) in the curated order."""
    entries = [entry(prefix, brands, meta, info) for prefix, brands, meta, info in packs]
    return (json.dumps(dict(packs=entries), ensure_ascii=False, indent=1) + "\n").encode()


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inputs", type=Path)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args(argv)

    def read(name, url):
        if args.inputs and (args.inputs / name).exists():
            return (args.inputs / name).read_bytes()
        with urllib.request.urlopen(url, timeout=60) as response:
            data = response.read()
        if args.inputs:
            args.inputs.mkdir(parents=True, exist_ok=True)
            (args.inputs / name).write_bytes(data)
        return data

    packs = []
    for prefix, version, brands in PACKS:
        meta = json.loads(read(f"{prefix}-{version}.json", f"{REGISTRY}{package(prefix)}/{version}"))
        if meta["version"] != version:
            raise ValueError(f"{prefix}: the registry answered version {meta['version']}")
        tarball = read(f"{prefix}-{version}.tgz", meta["dist"]["tarball"])
        if integrity_of(tarball) != meta["dist"]["integrity"]:
            raise ValueError(f"{prefix}: the tarball does not match its integrity")
        packs.append((prefix, brands, meta, read_info(tarball)))
    data = generate(packs)
    if args.check:
        assert OUTPUT.read_bytes() == data, "packs.json differs; regenerate it"
    else:
        OUTPUT.parent.mkdir(parents=True, exist_ok=True)
        OUTPUT.write_bytes(data)
    print(f"Icon packs: {len(packs)}; {sum(p[3]['total'] for p in packs)} icons; {len(data)} bytes")


if __name__ == "__main__":
    main()
