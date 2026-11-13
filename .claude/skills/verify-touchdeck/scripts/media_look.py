"""Reads a raw RGB565 screenshot of the Media page and checks each media tile's look.

usage: python -I media_look.py <shot.rgb565> <tiles.json> <out.json> [pressed <tile id>]

<shot.rgb565> is the `--raw` file of a full-size `$TD serial screenshot`. tiles.json maps a tile id to its
`ui tile` reply (x, y, w, h, cx, cy). The pass flags come from tokens: tile 6406, tile_down 10765, glow 19743,
text 61311 (icon), caption 42360, bg 2114. It exits 0 when every tile has its icon above its caption, both inside
the 16 px inset and within 12 px of the tile's centre column, and a probe at (x + 12, cy) reads `tile`, or
`tile_down` and a 3 px `glow` border for the tile named after `pressed`.
"""
import json
import sys

WIDTH = 1024
HEIGHT = 600
TOKEN = {"bg": 2114, "tile": 6406, "tile_down": 10765, "glow": 19743, "text": 61311, "caption": 42360}
INSET = 16


def main():
    raw_path, tiles_path, out_path = sys.argv[1:4]
    pressed = sys.argv[5] if len(sys.argv) > 5 and sys.argv[4] == "pressed" else None
    data = open(raw_path, "rb").read()
    assert len(data) == WIDTH * HEIGHT * 2, len(data)

    def px(x, y):
        o = (y * WIDTH + x) * 2
        return data[o] | (data[o + 1] << 8)

    tiles = json.load(open(tiles_path))
    report = {}
    all_ok = True
    for tid, t in tiles.items():
        x, y, w, h, cx, cy = (t[k] for k in ("x", "y", "w", "h", "cx", "cy"))
        boxes = {}
        for name in ("text", "caption"):
            want = TOKEN[name]
            xs, ys = [], []
            for yy in range(y + 4, y + h - 4):
                row = (yy * WIDTH) * 2
                for xx in range(x + 4, x + w - 4):
                    o = row + xx * 2
                    if data[o] | (data[o + 1] << 8) == want:
                        xs.append(xx)
                        ys.append(yy)
            boxes[name] = (min(xs), min(ys), max(xs), max(ys)) if xs else None
        icon, cap = boxes["text"], boxes["caption"]
        is_pressed = tid == pressed
        fill = px(x + 12, cy)
        info = {
            "tile": [x, y, w, h],
            "fill_at_x+12": fill,
            "fill_ok": fill == (TOKEN["tile_down"] if is_pressed else TOKEN["tile"]),
            "icon_bbox": icon,
            "caption_bbox": cap,
        }
        if is_pressed:
            info["glow_border"] = [px(x + i, cy) for i in range(3)]
            info["glow_ok"] = info["glow_border"] == [TOKEN["glow"]] * 3
        ok = info["fill_ok"] and icon is not None and cap is not None
        if ok:
            info["icon_inside_inset"] = icon[0] >= x + INSET and icon[2] <= x + w - 1 - INSET
            info["caption_inside_inset"] = cap[0] >= x + INSET and cap[2] <= x + w - 1 - INSET
            info["icon_above_caption"] = icon[3] < cap[1]
            info["icon_centre_off_px"] = (icon[0] + icon[2]) // 2 - cx
            info["caption_centre_off_px"] = (cap[0] + cap[2]) // 2 - cx
            info["stack_top_margin"] = icon[1] - y
            info["stack_bottom_margin"] = y + h - 1 - cap[3]
            ok = (
                info["icon_inside_inset"]
                and info["caption_inside_inset"]
                and info["icon_above_caption"]
                and abs(info["icon_centre_off_px"]) <= 12
                and abs(info["caption_centre_off_px"]) <= 12
                and (not is_pressed or info["glow_ok"])
            )
        info["ok"] = bool(ok)
        all_ok = all_ok and info["ok"]
        report[tid] = info
    gap_probe = {}
    if "prev" in tiles and "playpause" in tiles:
        gx = tiles["prev"]["x"] + tiles["prev"]["w"] + 8
        gap_probe["gap_prev_playpause"] = px(gx, tiles["prev"]["cy"])
        all_ok = all_ok and gap_probe["gap_prev_playpause"] == TOKEN["bg"]
    json.dump({"ok": all_ok, "tiles": report, **gap_probe}, open(out_path, "w"), indent=2)
    print(json.dumps({"ok": all_ok, "tiles": {k: v["ok"] for k, v in report.items()}, **gap_probe}))
    return 0 if all_ok else 1


if __name__ == "__main__":
    sys.exit(main())
