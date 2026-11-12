#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include "esp_console.h"
#include "esp_err.h"
#include "touch_queue.h"

#ifdef __cplusplus
extern "C" {
#endif

#define POWER_IDLE_TIMEOUT_DEFAULT_S 600
#define POWER_IDLE_TIMEOUT_MIN_S 1
#define POWER_IDLE_TIMEOUT_MAX_S 86400
#define POWER_PHOTO_TIMEOUT_MIN_S 10
#define POWER_PHOTO_TIMEOUT_MAX_S 86400

typedef enum {
    POWER_ACTIVE,
    POWER_OFF,
} power_state_t;

typedef struct {
    power_state_t state;     /* ACTIVE when the CH422G's acknowledged output byte has the backlight bit set */
    bool swallowing;         /* a contact is being dropped, from its first sample to the sample after its lift */
    uint32_t idle_timeout_s;
    uint32_t idle_ms;        /* since the last touching sample, or since the timeout was set if that is later */
    uint32_t swallowed;      /* samples dropped since start, the wake contact's press and its closing release included */
    uint32_t off_count;      /* acknowledged backlight-off writes made here */
    uint32_t wake_count;     /* acknowledged backlight-on writes made for a touch on a dark screen or a power_wake */
    uint32_t wake_us;        /* latest wake: from the wake sample reaching power_filter to the backlight write returning */
    uint32_t errors;         /* failed backlight writes made here, the lit write that follows a failed dark write included */
    uint8_t output;          /* the CH422G output byte the chip last acknowledged */
    bool photo_idle;         /* photo idle mode is on */
    bool idle_face;          /* the idle face covers the Page */
    uint32_t photo_timeout_s;
    uint32_t face_count;     /* times the idle face came up */
    uint32_t face_wakes;     /* contacts eaten to take the idle face down */
} power_info_t;

/*
 * The idle gate between the touch stream and LVGL.
 *
 * The screen is lit or dark as the CH422G says: lit is the backlight bit of ch422g_output(). Nothing here keeps a
 * second copy, so the `backlight` console command and this gate cannot disagree about it.
 *
 * A contact is a run of samples with a finger down, real or injected, from its first sample to the first sample
 * without one. Its fate is fixed at its first sample. Lit, and the contact is passed to LVGL unchanged to its release.
 * Dark, and the contact is eaten: LVGL takes the sample it took last, released, for every sample of the contact and for
 * the release that ends it, and the backlight is lit again. LVGL therefore never sees a press appear, move, or lose its
 * release, and a touch that wakes the screen cannot click a tile. A contact whose first sample was lit keeps passing
 * even if the backlight goes dark under it, so a press that LVGL has seen always gets its release.
 *
 * The controller can drop a finger for a few reads mid-touch, which reads as a lift and then a new press. So a contact
 * that starts with a real finger down within 250 ms of the end of an eaten contact is eaten too, whatever the screen
 * shows. An injected contact has no real finger and is never held back by this.
 *
 * With no contact open and the screen lit, the screen goes dark once the idle clock has run for the timeout. Every
 * sample with a finger down restarts the clock, so the screen never goes dark under a finger. Going dark clears every
 * key modifier first (keys_clear_all), so nothing stays armed or locked on a dark screen, and then clears the backlight
 * bit.
 *
 * Failed writes. A failed dark write leaves the screen lit and is followed by one absolute lit write, because a
 * timed-out transfer may have landed. The next dark write waits 5 seconds. If that lit write fails too, the chip may be
 * dark while its cached byte says lit, so the next contact is eaten and rewrites the byte until a write is acknowledged.
 * A failed wake write still eats its contact. It is retried on a later sample of that contact after 250 ms, and on the
 * first sample of the next contact.
 *
 * Photo idle mode (link-v2.md "Photo frame, idle, and clock") replaces going dark. While it is on the gate never
 * darkens the screen: once the idle clock has run for the photo timeout on a lit screen with no contact open, it clears
 * every key modifier and raises the idle face, which the UI draws over the Page. The face counts as dark for the next
 * contact, which is eaten whole and takes the face down, so the touch that wakes the Device to its last Page clicks
 * nothing. The backlight is not written for it. Turning the mode off takes the face down and the v1 timeout applies
 * again.
 */

/* Starts the gate with the idle clock at now_us and the default timeout. ESP_ERR_INVALID_STATE when the CH422G has not
 * started (its output byte reads 0, which a started chip never does), and then power_filter passes every sample and
 * every other call here fails the same way. A chip that started dark is a valid start. Call it once, after
 * display_start() and before touch_start(). A second call resets the gate, which is for tests. */
esp_err_t power_start(int64_t now_us);

/*
 * One LVGL pointer read. `next` is the sample the read callback chose, after the injection queue, and `finger_down`
 * is whether a real finger is on the glass. Returns the sample LVGL takes now, and remembers it as the sample an eaten
 * contact repeats. Only the LVGL read callback calls it, once per read, and it may write the CH422G, so it must not run
 * inside a spinlock. now_us is esp_timer_get_time() in production.
 */
td_touch_sample_t power_filter(td_touch_sample_t next, bool finger_down, int64_t now_us);

/* A coherent snapshot. ESP_ERR_INVALID_STATE before a successful power_start. Any task may call it. */
esp_err_t power_get_info(power_info_t *out, int64_t now_us);

/* Sets the idle timeout, 1 to 86400 seconds, and restarts the idle clock at now_us. Not stored, so a reset restores 600.
 * ESP_ERR_INVALID_ARG out of range, ESP_ERR_INVALID_STATE before a successful power_start. Any task may call it. */
esp_err_t power_set_idle_timeout_s(uint32_t seconds, int64_t now_us);

/* Turns photo idle mode on, with the idle face after timeout_s without a touch, or off. Either way the idle clock
 * restarts at now_us. Not stored here. ESP_ERR_INVALID_ARG for a timeout outside 10 to 86400 when turning it on,
 * ESP_ERR_INVALID_STATE before a successful power_start. Any task may call it. */
esp_err_t power_set_photo_idle(bool on, uint32_t timeout_s, int64_t now_us);

/* Asks for the screen for something the person must see now, such as a pairing code: on the LVGL read callback's next
 * read, a dark screen lights, the idle face goes down and the idle clock starts over, as a touch would, with no contact
 * to eat. ESP_ERR_INVALID_STATE before a successful power_start. Any task may call it. */
esp_err_t power_wake(void);

/* Whether the idle face covers the Page, and, through `entry` when it isn't NULL, the number of the latest idle period.
 * False before power_start. Any task may call it. */
bool power_idle_face(uint32_t *entry);

/* The power console commands, for td_console_register(). Before power_start succeeds, a well-formed command answers
 * "power not ready". */
const esp_console_cmd_t *power_console_commands(size_t *count);

#ifdef __cplusplus
}
#endif
