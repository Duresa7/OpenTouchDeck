#include "power.h"
#include <stdatomic.h>
#include <string.h>
#include "board_pins.h"
#include "ch422g.h"
#include "esp_log.h"
#include "esp_timer.h"
#include "freertos/FreeRTOS.h"
#include "keys.h"

#define US_PER_S 1000000LL
#define DARK_RETRY_US (5 * US_PER_S)
#define LIGHT_RETRY_US (250 * 1000LL)
#define WAKE_GUARD_US (250 * 1000LL)

typedef enum {
    CONTACT_NONE,
    CONTACT_PASSED,
    CONTACT_EATEN,
} contact_t;

typedef enum {
    ACT_NONE,
    ACT_DARK,
    ACT_LIGHT,
    ACT_FACE,
} action_t;

static const char *TAG = "power";

/*
 * The LVGL read callback owns the contact, the idle clock, the retry times, `shown`, and the counters, and it is the
 * only task that writes the backlight. The console owns the timeout and the time it was set. Each side writes its own
 * fields, and the idle clock is the later of the two times, so neither overwrites the other. The lock covers field
 * access only and is never held across a CH422G write, a key engine call, or a log line.
 */
static struct {
    contact_t contact;
    bool prev_lit;
    bool chip_in_doubt;
    td_touch_sample_t shown;
    int64_t last_touch_us;
    int64_t guard_until_us;
    int64_t retry_dark_us;
    int64_t retry_light_us;
    int64_t timeout_us;
    int64_t timeout_set_us;
    bool photo_on;
    bool idle_face;
    /* power_wake asked for the screen: the read callback lights it on its next read. */
    bool wake_wanted;
    int64_t photo_timeout_us;
    uint32_t face_count;
    uint32_t face_wakes;
    uint32_t swallowed;
    uint32_t off_count;
    uint32_t wake_count;
    uint32_t wake_us;
    uint32_t errors;
} s;

static portMUX_TYPE s_lock = portMUX_INITIALIZER_UNLOCKED;
static atomic_bool s_started;

static bool backlight_lit(uint8_t output)
{
    return (output & TD_CH422G_IO_DISP) != 0;
}

static int64_t idle_since(void)
{
    return s.last_touch_us > s.timeout_set_us ? s.last_touch_us : s.timeout_set_us;
}

static uint32_t elapsed_ms(int64_t from_us, int64_t to_us)
{
    const int64_t ms = (to_us - from_us) / 1000;
    return ms <= 0 ? 0 : ms > UINT32_MAX ? UINT32_MAX : (uint32_t)ms;
}

esp_err_t power_start(int64_t now_us)
{
    const uint8_t output = ch422g_output();
    /* A started chip never reads 0, because no setter clears SD_CS. */
    const bool started = output != 0;

    portENTER_CRITICAL(&s_lock);
    memset(&s, 0, sizeof s);
    s.prev_lit = backlight_lit(output);
    s.last_touch_us = now_us;
    s.timeout_set_us = now_us;
    s.timeout_us = POWER_IDLE_TIMEOUT_DEFAULT_S * US_PER_S;
    portEXIT_CRITICAL(&s_lock);
    atomic_store(&s_started, started);
    return started ? ESP_OK : ESP_ERR_INVALID_STATE;
}

/* Runs under the lock, so it touches only `s`. */
static action_t step(bool lit, bool touching, bool finger_down, int64_t now_us, bool *swallow)
{
    action_t act = ACT_NONE;

    /* A screen that someone else lit, such as the `backlight on` command, starts a new idle period. */
    if (touching || (lit && !s.prev_lit)) {
        s.last_touch_us = now_us;
    }
    s.prev_lit = lit;

    const bool dark = !lit || s.chip_in_doubt;
    /* A wake counts as a touch for the idle clock and takes the idle face down, but has no contact to eat. A dark
     * screen is lit here unless a contact is open, whose own samples light it. */
    const bool wake = s.wake_wanted;
    s.wake_wanted = false;
    if (wake) {
        s.last_touch_us = now_us;
        s.idle_face = false;
        if (dark && !touching && s.contact == CONTACT_NONE && now_us >= s.retry_light_us) {
            act = ACT_LIGHT;
        }
    }
    if (touching && s.contact == CONTACT_NONE) {
        const bool wake_resumed = finger_down && now_us < s.guard_until_us;
        s.contact = dark || s.idle_face || wake_resumed ? CONTACT_EATEN : CONTACT_PASSED;
        if (s.idle_face) {
            s.idle_face = false;
            s.face_wakes++;
        }
    }
    *swallow = s.contact == CONTACT_EATEN;
    if (*swallow) {
        s.swallowed++;
        if (touching && dark && now_us >= s.retry_light_us) {
            act = ACT_LIGHT;
        }
    }

    if (!touching) {
        const bool resting = s.contact == CONTACT_NONE && lit;
        if (resting && !s.photo_on && now_us >= s.retry_dark_us && now_us - idle_since() >= s.timeout_us) {
            act = ACT_DARK;
        }
        if (resting && s.photo_on && !s.idle_face && !s.chip_in_doubt && now_us - idle_since() >= s.photo_timeout_us) {
            s.idle_face = true;
            s.face_count++;
            act = ACT_FACE;
        }
        if (s.contact == CONTACT_EATEN) {
            s.guard_until_us = now_us + WAKE_GUARD_US;
            s.retry_light_us = 0;
        }
        s.contact = CONTACT_NONE;
    }
    return act;
}

/* Modifiers clear before the write, so a dark screen never holds an armed key. A failed dark write may still have
 * landed, because a timed-out I2C transfer gives no answer, so it is followed by an absolute lit write. */
static void run(action_t act, int64_t entered_us, int64_t now_us)
{
    if (act == ACT_NONE) {
        return;
    }
    if (act == ACT_FACE) {
        keys_clear_all();
        ESP_LOGI(TAG, "idle face on");
        return;
    }
    const bool light = act == ACT_LIGHT;
    if (!light) {
        keys_clear_all();
    }
    const esp_err_t err = ch422g_set_backlight(light);
    const uint32_t took_us = (uint32_t)(esp_timer_get_time() - entered_us);
    const bool healed = err != ESP_OK && !light;
    const esp_err_t heal_err = healed ? ch422g_set_backlight(true) : ESP_OK;
    const bool byte_matches_chip = healed ? heal_err == ESP_OK : err == ESP_OK;

    /* prev_lit follows what this write left on the chip, so a `backlight on` that lands before the next read still
     * counts as someone else lighting the screen. */
    portENTER_CRITICAL(&s_lock);
    if (err == ESP_OK && light) {
        s.wake_count++;
        s.wake_us = took_us;
        s.retry_light_us = 0;
        s.prev_lit = true;
    } else if (err == ESP_OK) {
        s.off_count++;
        s.retry_dark_us = 0;
        s.prev_lit = false;
    } else if (light) {
        s.retry_light_us = now_us + LIGHT_RETRY_US;
    } else {
        s.retry_dark_us = now_us + DARK_RETRY_US;
    }
    if (byte_matches_chip) {
        s.chip_in_doubt = false;
    } else if (healed) {
        s.chip_in_doubt = true;
    }
    s.errors += (err != ESP_OK) + (heal_err != ESP_OK);
    portEXIT_CRITICAL(&s_lock);

    if (err == ESP_OK) {
        ESP_LOGI(TAG, "backlight %s", light ? "on" : "off");
    } else {
        ESP_LOGW(TAG, "backlight %s write failed: %s", light ? "on" : "off", esp_err_to_name(err));
    }
    if (heal_err != ESP_OK) {
        ESP_LOGW(TAG, "backlight on write after the failed off write failed too: %s", esp_err_to_name(heal_err));
    }
}

td_touch_sample_t power_filter(td_touch_sample_t next, bool finger_down, int64_t now_us)
{
    if (!atomic_load(&s_started)) {
        return next;
    }
    const int64_t entered_us = esp_timer_get_time();
    const bool lit = backlight_lit(ch422g_output());

    bool swallow = false;
    portENTER_CRITICAL(&s_lock);
    const action_t act = step(lit, next.pressed || finger_down, finger_down, now_us, &swallow);
    portEXIT_CRITICAL(&s_lock);

    run(act, entered_us, now_us);
    s.shown = swallow ? (td_touch_sample_t){ .x = s.shown.x, .y = s.shown.y, .pressed = false } : next;
    return s.shown;
}

esp_err_t power_get_info(power_info_t *out, int64_t now_us)
{
    if (!atomic_load(&s_started)) {
        return ESP_ERR_INVALID_STATE;
    }
    const uint8_t output = ch422g_output();

    portENTER_CRITICAL(&s_lock);
    *out = (power_info_t){
        .state = backlight_lit(output) ? POWER_ACTIVE : POWER_OFF,
        .swallowing = s.contact == CONTACT_EATEN,
        .idle_timeout_s = (uint32_t)(s.timeout_us / US_PER_S),
        .idle_ms = elapsed_ms(idle_since(), now_us),
        .swallowed = s.swallowed,
        .off_count = s.off_count,
        .wake_count = s.wake_count,
        .wake_us = s.wake_us,
        .errors = s.errors,
        .output = output,
        .photo_idle = s.photo_on,
        .idle_face = s.idle_face,
        .photo_timeout_s = (uint32_t)(s.photo_timeout_us / US_PER_S),
        .face_count = s.face_count,
        .face_wakes = s.face_wakes,
    };
    portEXIT_CRITICAL(&s_lock);
    return ESP_OK;
}

esp_err_t power_set_idle_timeout_s(uint32_t seconds, int64_t now_us)
{
    if (seconds < POWER_IDLE_TIMEOUT_MIN_S || seconds > POWER_IDLE_TIMEOUT_MAX_S) {
        return ESP_ERR_INVALID_ARG;
    }
    if (!atomic_load(&s_started)) {
        return ESP_ERR_INVALID_STATE;
    }
    portENTER_CRITICAL(&s_lock);
    s.timeout_us = (int64_t)seconds * US_PER_S;
    s.timeout_set_us = now_us;
    portEXIT_CRITICAL(&s_lock);
    return ESP_OK;
}

esp_err_t power_set_photo_idle(bool on, uint32_t timeout_s, int64_t now_us)
{
    if (on && (timeout_s < POWER_PHOTO_TIMEOUT_MIN_S || timeout_s > POWER_PHOTO_TIMEOUT_MAX_S)) {
        return ESP_ERR_INVALID_ARG;
    }
    if (!atomic_load(&s_started)) {
        return ESP_ERR_INVALID_STATE;
    }
    portENTER_CRITICAL(&s_lock);
    const bool was_face = s.idle_face;
    s.photo_on = on;
    if (on) {
        s.photo_timeout_us = (int64_t)timeout_s * US_PER_S;
    }
    s.idle_face = s.idle_face && on;
    s.timeout_set_us = now_us;
    portEXIT_CRITICAL(&s_lock);
    if (was_face && !on) {
        ESP_LOGI(TAG, "idle face off: photo idle mode turned off");
    }
    return ESP_OK;
}

esp_err_t power_wake(void)
{
    if (!atomic_load(&s_started)) {
        return ESP_ERR_INVALID_STATE;
    }
    portENTER_CRITICAL(&s_lock);
    s.wake_wanted = true;
    portEXIT_CRITICAL(&s_lock);
    return ESP_OK;
}

bool power_idle_face(uint32_t *entry)
{
    if (!atomic_load(&s_started)) {
        return false;
    }
    portENTER_CRITICAL(&s_lock);
    const bool face = s.idle_face;
    const uint32_t count = s.face_count;
    portEXIT_CRITICAL(&s_lock);
    if (entry != NULL) {
        *entry = count;
    }
    return face;
}
