#!/usr/bin/env python3
"""Build the Open TouchDeck app and tray icons from the drawings in svg/.

    python3 -I build.py                  render png/, write both .ico files, read them back
    python3 -I build.py --check          only read the committed .ico files back against png/
    python3 -I build.py --sheet out.png  with either: also draw a check sheet from the .ico files

Each size has its own drawing, fitted to its own pixel grid, so nothing here
resamples: an SVG is rendered at the size it declares and the build stops if the
two differ.

Needs Pillow, and librsvg's `rsvg-convert` to render (on PATH, or named with
--rsvg-convert). `--check` needs only Pillow.
"""

import argparse
import io
import re
import shutil
import struct
import subprocess
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parent
SVG = ROOT / "svg"
PNG = ROOT / "png"

# Every size Windows 11 asks an app icon for, from 100% to 400% display scale.
APP_SIZES = (16, 20, 24, 30, 32, 36, 40, 48, 60, 64, 72, 80, 96, 256)
# The notification area at 100%, 125%, 150% and 200%.
TRAY_SIZES = (16, 20, 24, 32)

APP_SOURCES = {size: f"icon-{size}.svg" for size in APP_SIZES}
# Up to 24 px the tray shows the same drawing as the app icon. At 32 px it keeps
# that flat drawing (the 16 px one at twice the size), so the tray icon looks the
# same at every display scale, while the 32 px app icon gains the pager.
TRAY_SOURCES = {16: "icon-16.svg", 20: "icon-20.svg", 24: "icon-24.svg", 32: "tray-32.svg"}

ICONS = {
    "open-touchdeck.ico": ("icon", APP_SOURCES),
    "tray.ico": ("tray", TRAY_SOURCES),
}

PNG_ENTRY_FROM = 256  # entries this size and up are stored as PNG, smaller ones as 32-bit BMP


# --- rendering ---------------------------------------------------------------


def declared_size(svg_path):
    """The width and height the drawing's <svg> tag declares, in pixels."""
    tag = re.search(r"<svg\b[^>]*>", svg_path.read_text(encoding="utf-8"))
    width = re.search(r'\bwidth="(\d+)"', tag.group(0)) if tag else None
    height = re.search(r'\bheight="(\d+)"', tag.group(0)) if tag else None
    if not (width and height):
        sys.exit(f"{svg_path.name} does not declare a whole-pixel width and height.")
    return int(width.group(1)), int(height.group(1))


def load_renderer(executable):
    """Return (name, render) where render(svg_path, size) gives an RGBA image."""
    exe = shutil.which(executable)
    if not exe:
        sys.exit(f"{executable} not found. Install librsvg, or pass --rsvg-convert with its path.")

    def render(svg_path, size):
        data = subprocess.run(
            [exe, "--width", str(size), "--height", str(size), str(svg_path)],
            check=True,
            capture_output=True,
        ).stdout
        return Image.open(io.BytesIO(data)).convert("RGBA")

    version = subprocess.run([exe, "--version"], check=True, capture_output=True, text=True).stdout
    return version.splitlines()[0].strip(), render


def render_all(executable):
    name, render = load_renderer(executable)
    PNG.mkdir(exist_ok=True)
    for prefix, sources in ICONS.values():
        for size, source in sources.items():
            svg_path = SVG / source
            if declared_size(svg_path) != (size, size):
                sys.exit(f"{source} is drawn at {declared_size(svg_path)}, not {size} px: it needs its own drawing.")
            image = render(svg_path, size)
            if image.size != (size, size):
                sys.exit(f"{source} rendered at {image.size}, expected {size} px.")
            image.save(PNG / f"{prefix}-{size}.png", optimize=True)
    return name


# --- .ico --------------------------------------------------------------------


def bmp_entry(image):
    """A 32-bit icon bitmap: BITMAPINFOHEADER, BGRA rows bottom-up, then the 1-bit AND mask."""
    width, height = image.size
    rows = [image.crop((0, y, width, y + 1)) for y in reversed(range(height))]
    colour = b"".join(row.tobytes("raw", "BGRA") for row in rows)
    mask_row_bytes = ((width + 31) // 32) * 4
    mask = bytearray()
    for row in rows:
        bits = bytearray(mask_row_bytes)
        for x, alpha in enumerate(row.getchannel("A").tobytes()):
            if alpha == 0:
                bits[x // 8] |= 0x80 >> (x % 8)
        mask += bits
    header = struct.pack("<IiiHHIIiiII", 40, width, height * 2, 1, 32, 0, len(colour) + len(mask), 0, 0, 0, 0)
    return header + colour + bytes(mask)


def png_entry(image):
    buffer = io.BytesIO()
    image.save(buffer, format="PNG", optimize=True)
    return buffer.getvalue()


def write_ico(path, images):
    """Write one entry per image, each stored as given: no entry is scaled from another."""
    images = sorted(images, key=lambda image: image.width)
    payloads = [png_entry(i) if i.width >= PNG_ENTRY_FROM else bmp_entry(i) for i in images]
    offset = 6 + 16 * len(images)
    out = bytearray(struct.pack("<HHH", 0, 1, len(images)))
    for image, payload in zip(images, payloads):
        side = image.width if image.width < 256 else 0  # 0 means 256
        out += struct.pack("<BBBBHHII", side, side, 0, 0, 1, 32, len(payload), offset)
        offset += len(payload)
    for payload in payloads:
        out += payload
    path.write_bytes(bytes(out))


def read_ico(path):
    """Decode every entry of an .ico without Pillow's ICO reader. Returns a list of dicts."""
    data = path.read_bytes()
    reserved, kind, count = struct.unpack_from("<HHH", data, 0)
    if (reserved, kind) != (0, 1):
        raise ValueError(f"{path.name} is not an icon file")
    entries = []
    for index in range(count):
        width, height, _colours, _reserved, planes, bits, size, offset = struct.unpack_from(
            "<BBBBHHII", data, 6 + 16 * index
        )
        width, height = width or 256, height or 256
        payload = data[offset : offset + size]
        if len(payload) != size:
            raise ValueError(f"{path.name} entry {index} runs past the end of the file")
        if payload[:8] == b"\x89PNG\r\n\x1a\n":
            image = Image.open(io.BytesIO(payload))
            image.load()
            stored, image = "PNG", image.convert("RGBA")
        else:
            (header_size, bmp_width, bmp_height, bmp_planes, bmp_bits, compression) = struct.unpack_from(
                "<IiiHHI", payload, 0
            )
            if (header_size, bmp_width, bmp_height, bmp_bits, compression) != (40, width, height * 2, 32, 0):
                raise ValueError(f"{path.name} entry {index} is not a 32-bit icon bitmap")
            colour = payload[40 : 40 + width * height * 4]
            mask_size = ((width + 31) // 32) * 4 * height
            if len(payload) != 40 + len(colour) + mask_size:
                raise ValueError(f"{path.name} entry {index} has the wrong length")
            image = Image.frombuffer("RGBA", (width, height), colour, "raw", "BGRA", 0, -1).copy()
            stored = "32-bit BMP"
        if image.size != (width, height):
            raise ValueError(f"{path.name} entry {index} decodes to {image.size}, directory says {width}")
        entries.append(
            {"size": width, "planes": planes, "bits": bits, "stored": stored, "bytes": size, "image": image}
        )
    return entries


def build_icos():
    for name, (prefix, sources) in ICONS.items():
        images = []
        for size in sources:
            with Image.open(PNG / f"{prefix}-{size}.png") as png:
                images.append(png.convert("RGBA"))
        write_ico(ROOT / name, images)


def check_icos():
    """Read both .ico files back and compare every entry, pixel for pixel, with its PNG."""
    ok = True
    for name, (prefix, sources) in ICONS.items():
        entries = read_ico(ROOT / name)
        found = [entry["size"] for entry in entries]
        wanted = sorted(sources)
        print(f"{name}: {len(entries)} entries, {(ROOT / name).stat().st_size:,} bytes")
        if found != wanted:
            print(f"  MISMATCH: holds {found}, expected {wanted}")
            ok = False
        for entry in entries:
            size = entry["size"]
            png_path = PNG / f"{prefix}-{size}.png"
            verdict = "no PNG to compare"
            if png_path.exists():
                with Image.open(png_path) as png:
                    same = png.convert("RGBA").tobytes() == entry["image"].tobytes()
                verdict = f"decodes, same pixels as png/{png_path.name}" if same else "DIFFERS from " + png_path.name
            if not verdict.startswith("decodes"):
                ok = False
            print(f"  {size:>3} x {size:<3}  {entry['stored']:<10}  {entry['bytes']:>6,} bytes  {verdict}")
    return ok


# --- check sheet -------------------------------------------------------------

LIGHT = (243, 243, 243)  # #f3f3f3, the Windows 11 light taskbar
DARK = (32, 32, 32)  # #202020, the Windows 11 dark taskbar
PAGE = (250, 250, 251)
INK = (26, 29, 34)
INK_2 = (86, 93, 104)
LINE = (222, 225, 229)
SHEET_WIDTH = 1800
MARGIN = 40


def font(size):
    return ImageFont.load_default(size)


def zoomed(image, factor):
    return image.resize((image.width * factor, image.height * factor), Image.NEAREST)


def strip(sheet, top, background, items, height):
    """One row of images, bottom-aligned on a panel. items: (image, zoom, caption)."""
    draw = ImageDraw.Draw(sheet)
    draw.rounded_rectangle(
        [MARGIN, top, SHEET_WIDTH - MARGIN, top + height], radius=10, fill=background, outline=LINE
    )
    caption_colour = (150, 150, 150) if background == DARK else (100, 100, 100)
    x = MARGIN + 24
    for image, factor, caption in items:
        shown = zoomed(image, factor) if factor > 1 else image
        sheet.paste(shown, (x, top + height - 34 - shown.height), shown)
        draw.text((x + shown.width / 2, top + height - 17), caption, font=font(12), fill=caption_colour, anchor="mm")
        x += max(shown.width, 44) + 22
    return top + height


def taskbar(sheet, box, dark, tray):
    """A piece of a Windows 11 taskbar: the tray icon between system glyphs at four display scales."""
    draw = ImageDraw.Draw(sheet)
    x0, y0, x1, y1 = box
    ink = (255, 255, 255) if dark else (27, 27, 27)
    draw.rectangle(box, fill=DARK if dark else LIGHT)
    draw.line([(x0, y0), (x1, y0)], fill=(58, 58, 58) if dark else (214, 214, 214))
    middle = (y0 + y1) // 2
    x = x0 + 24
    for size, scale in ((16, "100%"), (20, "125%"), (24, "150%"), (32, "200%")):
        unit = size / 16
        stroke = max(1, round(unit))
        draw.line(
            [(x, middle + 2 * unit), (x + 4 * unit, middle - 2 * unit), (x + 8 * unit, middle + 2 * unit)],
            fill=ink,
            width=stroke,
        )
        x += round(20 * unit)
        sheet.paste(tray[size], (x, middle - size // 2), tray[size])
        draw.text((x + size / 2, y1 + 13), f"{size} px, {scale}", font=font(12), fill=INK_2, anchor="mm")
        x += size + round(12 * unit)
        radius = round(6 * unit)
        draw.ellipse([x, middle - radius, x + 2 * radius, middle + radius], outline=ink, width=stroke)
        x += 2 * radius + round(10 * unit)
        draw.rounded_rectangle(
            [x, middle - round(5 * unit), x + round(15 * unit), middle + round(5 * unit)],
            radius=round(2 * unit),
            outline=ink,
            width=stroke,
        )
        x += round(15 * unit) + 34


def draw_sheet(path):
    """Draw the check sheet from the images decoded out of the two .ico files."""
    app = {entry["size"]: entry["image"] for entry in read_ico(ROOT / "open-touchdeck.ico")}
    tray = {entry["size"]: entry["image"] for entry in read_ico(ROOT / "tray.ico")}
    sheet = Image.new("RGB", (SHEET_WIDTH, 4400), PAGE)
    draw = ImageDraw.Draw(sheet)

    def heading(y, text, note=""):
        draw.text((MARGIN, y), text, font=font(17), fill=INK)
        if note:
            draw.text((MARGIN, y + 24), note, font=font(13), fill=INK_2)
        return y + (52 if note else 30)

    draw.text((MARGIN, 30), "Open TouchDeck 2.0: app and tray icons", font=font(30), fill=INK)
    draw.text(
        (MARGIN, 70),
        "Every image on this sheet is decoded from open-touchdeck.ico or tray.ico. "
        "Light panels are #f3f3f3 and dark panels #202020, the Windows 11 taskbar colours.",
        font=font(14),
        fill=INK_2,
    )

    y = heading(112, "App icon: every size at 1x", "open-touchdeck.ico, " + ", ".join(map(str, APP_SIZES)) + " px")
    for background in (LIGHT, DARK):
        y = strip(sheet, y, background, [(app[s], 1, str(s)) for s in APP_SIZES], 256 + 58) + 10

    y = heading(y + 22, "App icon: 16 to 48 px enlarged x6", "Nearest neighbour, so these are the real pixels. Hand-fitted sizes.")
    small = [s for s in APP_SIZES if s <= 48]
    for background in (LIGHT, DARK):
        y = strip(sheet, y, background, [(app[s], 6, f"{s} px x6") for s in small], 288 + 58) + 10

    y = heading(y + 22, "App icon: 60 to 96 px enlarged x3", "Nearest neighbour. The master redrawn on each grid.")
    large = [s for s in APP_SIZES if 48 < s < 256]
    for background in (LIGHT, DARK):
        y = strip(sheet, y, background, [(app[s], 3, f"{s} px x3") for s in large], 288 + 58) + 10

    y = heading(y + 22, "App icon: 256 px enlarged x2", "The master, on both backgrounds.")
    half = (SHEET_WIDTH - 2 * MARGIN - 20) // 2
    big = zoomed(app[256], 2)
    for index, background in enumerate((LIGHT, DARK)):
        x = MARGIN + index * (half + 20)
        draw.rounded_rectangle([x, y, x + half, y + 552], radius=10, fill=background, outline=LINE)
        sheet.paste(big, (x + (half - 512) // 2, y + 20), big)
    y += 552

    y = heading(
        y + 32,
        "Tray icon: the notification area at 1x",
        "tray.ico at 100%, 125%, 150% and 200% display scale, between system glyphs.",
    )
    for index, dark in enumerate((False, True)):
        x = MARGIN + index * (half + 20)
        taskbar(sheet, [x, y, x + half, y + 48], dark, tray)
    y += 48 + 30

    y = heading(y + 22, "Tray icon: every size enlarged x8", "Nearest neighbour. 16, 20, 24 and 32 px.")
    for index, background in enumerate((LIGHT, DARK)):
        x = MARGIN + index * (half + 20)
        draw.rounded_rectangle([x, y, x + half, y + 256 + 58], radius=10, fill=background, outline=LINE)
        caption_colour = (150, 150, 150) if background == DARK else (100, 100, 100)
        cx = x + 20
        for size in TRAY_SIZES:
            shown = zoomed(tray[size], 8)
            sheet.paste(shown, (cx, y + 256 + 24 - shown.height), shown)
            draw.text((cx + shown.width / 2, y + 256 + 41), f"{size} px x8", font=font(12), fill=caption_colour, anchor="mm")
            cx += shown.width + 10
    y += 256 + 58

    sheet = sheet.crop((0, 0, SHEET_WIDTH, y + MARGIN))
    path.parent.mkdir(parents=True, exist_ok=True)
    sheet.save(path)
    return sheet.size


# --- entry point -------------------------------------------------------------


def main():
    parser = argparse.ArgumentParser(description="Build the Open TouchDeck app and tray icons.")
    parser.add_argument("--check", action="store_true", help="only read the .ico files back against png/")
    parser.add_argument("--sheet", type=Path, metavar="PNG", help="also draw a check sheet from the .ico files")
    parser.add_argument("--rsvg-convert", default="rsvg-convert", metavar="PATH", help="the rsvg-convert to render with")
    args = parser.parse_args()

    if not args.check:
        renderer = render_all(args.rsvg_convert)
        build_icos()
        print(f"rendered with {renderer}")
    ok = check_icos()
    if args.sheet:
        width, height = draw_sheet(args.sheet)
        print(f"check sheet: {args.sheet} ({width} x {height})")
    if not ok:
        sys.exit("The .ico files do not match png/.")


if __name__ == "__main__":
    main()
