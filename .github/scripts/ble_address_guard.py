"""Fail closed on public BLE addresses and undocumented advertising call sites.

This lexical guard complements host tests and review; it is not C data-flow analysis.
Run from any directory, or pass --root for a fixture checkout.
"""
import argparse
from pathlib import Path
import re

# No production exception today. An exception must identify one function, explain
# its pairing-only control flow here, and be recorded in the Emitters table.
PAIRING_ONLY_ALLOWLIST: dict[str, str] = {}
PUBLIC = re.compile(
    r"\b(?:BLE_OWN_ADDR_PUBLIC|BLE_ADDR_TYPE_PUBLIC)\b"
    r"|\bble_hs_id_infer_auto\s*\((?!\s*1\s*,)"
    r"|\bble_gap_adv_start\s*\(\s*(?:0[xX]0+|0)[uUlL]*\s*,"
)
START = re.compile(r"\b(?:ble_gap_(?:ext_)?adv_start|esp_ble_gap_(?:start_advertising|ext_adv_start)|adv_start)\s*\(")
FUNCTION = re.compile(r"^[A-Za-z_][\w \t*]*\b(\w+)\s*\([^;{}]*\)\s*\{", re.M)


def stripped(source):
    # Preserve newlines for actionable diagnostics; ignore comments and literals.
    return re.sub(r'/\*.*?\*/|//[^\n]*|"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])*\'',
                  lambda m: "\n" * m[0].count("\n") or " ", source, flags=re.S)


def check(root):
    spec = (root / "schema/protocol/net-v2.md").read_text()
    table = spec.split("**Emitters**", 1)[-1].split("\n\n", 2)
    documented = set(re.findall(r"`(firmware/[^`]+:\w+)`", "\n\n".join(table[:2]))) if "**Emitters**" in spec else set()
    errors = []
    for path in sorted((root / "firmware").rglob("*")):
        # SDK/vendor code and host fixtures are not shipped application emitters.
        parts = path.relative_to(root / "firmware").parts
        if (parts[0] in {"managed_components", "tools", "build"}
                or parts[0].startswith("build-")
                or any(part in {"test", "tests"} for part in parts[:-1])):
            continue
        if path.suffix not in {".c", ".h", ".cpp", ".cc", ".cxx", ".hpp"}:
            continue
        source = stripped(path.read_text())
        functions = list(FUNCTION.finditer(source))
        for pattern, message in ((PUBLIC, "public BLE address"), (START, "unlisted advertising start")):
            for match in pattern.finditer(source):
                owners = [f for f in functions if f.start() < match.start()]
                owner = owners[-1][1] if owners else "global"
                site = f"{path.relative_to(root).as_posix()}:{owner}"
                if pattern is PUBLIC and site in PAIRING_ONLY_ALLOWLIST:
                    continue
                if pattern is START and site in documented:
                    continue
                line = source[:match.start()].count("\n") + 1
                errors.append(f"{site}:{line}: {message}")
    return errors


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2])
    args = parser.parse_args()
    failures = check(args.root)
    print("\n".join(failures) if failures else "BLE address guard passed")
    raise SystemExit(bool(failures))
