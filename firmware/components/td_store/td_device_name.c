#include "td_device.h"
#include <stdint.h>
#include <string.h>

/* The format characters (General_Category Cf) of Unicode 17.0, as inclusive ranges. link-v2.md and net-v2.md pin
 * this table, and schema/protocol/name-vectors-v2.json checks it. */
static const struct { uint32_t first, last; } FORMAT[] = {
    { 0x00ad, 0x00ad }, { 0x0600, 0x0605 }, { 0x061c, 0x061c }, { 0x06dd, 0x06dd },
    { 0x070f, 0x070f }, { 0x0890, 0x0891 }, { 0x08e2, 0x08e2 }, { 0x180e, 0x180e },
    { 0x200b, 0x200f }, { 0x202a, 0x202e }, { 0x2060, 0x2064 }, { 0x2066, 0x206f },
    { 0xfeff, 0xfeff }, { 0xfff9, 0xfffb }, { 0x110bd, 0x110bd }, { 0x110cd, 0x110cd },
    { 0x13430, 0x1343f }, { 0x1bca0, 0x1bca3 }, { 0x1d173, 0x1d17a }, { 0xe0001, 0xe0001 },
    { 0xe0020, 0xe007f },
};

/* Letters (Lo) that render as blank space. */
static bool hangul_filler(uint32_t c)
{
    return c == 0x115f || c == 0x1160 || c == 0x3164 || c == 0xffa0;
}

static bool format_character(uint32_t c)
{
    for (size_t i = 0; i < sizeof FORMAT / sizeof FORMAT[0]; i++) {
        if (c >= FORMAT[i].first && c <= FORMAT[i].last) return true;
    }
    return false;
}

bool td_device_name_valid(const char *name)
{
    if (!name || !*name || strlen(name) > TD_DEVICE_NAME_MAX) return false;
    const unsigned char *p = (const unsigned char *)name;
    bool visible = false;
    while (*p) {
        uint32_t c = *p++;
        unsigned count = 0;
        uint32_t min = 0;
        if (c >= 0xc2 && c <= 0xdf) { c &= 31; count = 1; min = 0x80; }
        else if (c >= 0xe0 && c <= 0xef) { c &= 15; count = 2; min = 0x800; }
        else if (c >= 0xf0 && c <= 0xf4) { c &= 7; count = 3; min = 0x10000; }
        else if (c >= 0x80) return false;
        while (count--) {
            if ((*p & 0xc0) != 0x80) return false;
            c = (c << 6) | (*p++ & 63);
        }
        if (c < min || c > 0x10ffff || (c >= 0xd800 && c <= 0xdfff) || c < 32 || c == 127) return false;
        if (format_character(c) || hangul_filler(c)) return false;
        bool space = c == 32 || c == 0x85 || c == 0xa0 || c == 0x1680 ||
                     (c >= 0x2000 && c <= 0x200a) || c == 0x2028 || c == 0x2029 ||
                     c == 0x202f || c == 0x205f || c == 0x3000;
        visible |= !space;
    }
    return visible;
}
