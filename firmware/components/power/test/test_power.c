#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "esp_rom_sys.h"
#include "unity.h"
#include "board_pins.h"
#include "ch422g.h"
#include "keys.h"
#include "power.h"
#include "touch_queue.h"

#define T0 INT64_C(1000000000)
#define MS INT64_C(1000)
#define SEC INT64_C(1000000)

#define LIT 0x1E
#define DARK 0x1A
#define DISP_BIT ((uint8_t)TD_CH422G_IO_DISP)

#define MAX_WRITES 64
#define FUZZ_STEPS 20000

#define EXPECT_SAMPLE(got, want_x, want_y, want_pressed)  \
    do {                                                  \
        TEST_ASSERT_EQUAL_UINT16((want_x), (got).x);      \
        TEST_ASSERT_EQUAL_UINT16((want_y), (got).y);      \
        TEST_ASSERT_EQUAL((want_pressed), (got).pressed); \
    } while (0)

#define EXPECT_SAMPLE_MSG(got, want_x, want_y, want_pressed, msg)  \
    do {                                                           \
        TEST_ASSERT_EQUAL_UINT16_MESSAGE((want_x), (got).x, (msg));  \
        TEST_ASSERT_EQUAL_UINT16_MESSAGE((want_y), (got).y, (msg));  \
        TEST_ASSERT_EQUAL_MESSAGE((want_pressed), (got).pressed, (msg)); \
    } while (0)

typedef struct {
    uint8_t addr;
    uint8_t value;
    uint8_t before;
    bool acked;
    bool landed;      /* the byte reached the chip: acked, or failed but latched */
    bool mods_clear;
} write_rec_t;

typedef struct {
    write_rec_t writes[MAX_WRITES];
    size_t count;
    bool overflow;
    uint8_t physical;       /* the IO byte the chip really holds, so the real backlight: bit DISP clear is dark */
    unsigned fail_next;
    /* A bit mask over the failing writes still to come, bit 0 for the next one: a set bit means that failed write still
     * lands on the chip, as a timed-out transfer can. The driver sees the failure and keeps its cached byte. */
    uint32_t latch_on_fail;
    bool fail_random;
    uint32_t fail_one_in;
    bool latch_random;
    bool external;
    uint32_t fail_rng;
} fake_t;

/* Static so the driver never holds a pointer into a returned test's stack frame. */
static fake_t s_fake;

static uint32_t xorshift(uint32_t *s)
{
    uint32_t x = *s;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *s = x;
    return x;
}

static bool mods_all_off(const key_engine_state_t *ks)
{
    for (int i = 0; i < KEY_MOD_COUNT; i++) {
        if (ks->mods[i].state != KEY_MOD_OFF) {
            return false;
        }
    }
    return true;
}

static bool physical_lit(void)
{
    return (s_fake.physical & DISP_BIT) != 0;
}

/* Runs inside the driver's lock, so it only records. The tests assert afterwards. */
static esp_err_t fake_write(void *ctx, uint8_t addr, uint8_t value)
{
    fake_t *f = ctx;
    bool fail = false;
    if (!f->external) {
        if (f->fail_next > 0) {
            f->fail_next--;
            fail = true;
        } else if (f->fail_random) {
            fail = xorshift(&f->fail_rng) % f->fail_one_in == 0;
        }
    }
    bool landed = !fail;
    if (fail) {
        if (f->latch_random) {
            landed = (xorshift(&f->fail_rng) & 1) != 0;
        } else {
            landed = (f->latch_on_fail & 1) != 0;
            f->latch_on_fail >>= 1;
        }
    }

    const uint8_t before = f->physical;
    if (addr == TD_CH422G_ADDR_IO && landed) {
        f->physical = value;
    }
    if (f->count == MAX_WRITES) {
        f->overflow = true;
    } else {
        const key_engine_state_t ks = keys_get_state();
        f->writes[f->count++] = (write_rec_t){
            .addr = addr, .value = value, .before = before, .acked = !fail, .landed = landed,
            .mods_clear = mods_all_off(&ks),
        };
    }
    esp_rom_delay_us(200);
    return fail ? ESP_ERR_INVALID_RESPONSE : ESP_OK;
}

static void fake_delay_ms(void *ctx, uint32_t ms)
{
    (void)ctx;
    (void)ms;
}

static esp_err_t fake_hold_touch_int_low(void *ctx)
{
    (void)ctx;
    return ESP_OK;
}

static esp_err_t start_chip(void)
{
    memset(&s_fake, 0, sizeof s_fake);
    const ch422g_platform_t platform = {
        .write = fake_write,
        .delay_ms = fake_delay_ms,
        .hold_touch_int_low = fake_hold_touch_int_low,
        .ctx = &s_fake,
    };
    return ch422g_start(&platform);
}

static void writes_clear(void)
{
    s_fake.count = 0;
    s_fake.overflow = false;
}

static void expect_write_count(size_t n)
{
    TEST_ASSERT_FALSE_MESSAGE(s_fake.overflow, "write recorder overflow");
    TEST_ASSERT_EQUAL_UINT32(n, s_fake.count);
}

static void expect_write(size_t i, uint8_t value, bool acked)
{
    TEST_ASSERT_TRUE_MESSAGE(i < s_fake.count, "fewer writes than expected");
    TEST_ASSERT_EQUAL_HEX8(TD_CH422G_ADDR_IO, s_fake.writes[i].addr);
    TEST_ASSERT_EQUAL_HEX8(value, s_fake.writes[i].value);
    TEST_ASSERT_EQUAL(acked, s_fake.writes[i].acked);
}

/* The console `backlight` command: a write the gate did not make. It never fails and always lands. */
static void external_backlight(bool on)
{
    s_fake.external = true;
    const esp_err_t err = ch422g_set_backlight(on);
    s_fake.external = false;
    TEST_ASSERT_EQUAL(ESP_OK, err);
    writes_clear();
}

static unsigned s_sink_reports;

static void count_report(void *ctx, const key_report_t *report)
{
    (void)ctx;
    (void)report;
    s_sink_reports++;
}

static void ensure_keys(void)
{
    if (!keys_started()) {
        const key_sink_t sink = { .report = count_report };
        TEST_ASSERT_EQUAL(ESP_OK, keys_start(sink));
    }
}

/* Two quick taps lock a modifier, which gives going dark something to clear. */
static void lock_ctrl(void)
{
    keys_mod(KEY_MOD_CTRL);
    keys_mod(KEY_MOD_CTRL);
    TEST_ASSERT_EQUAL_MESSAGE(KEY_MOD_LOCKED, keys_get_state().mods[KEY_MOD_CTRL].state, "Ctrl did not lock");
}

static void expect_mods_off(void)
{
    const key_engine_state_t ks = keys_get_state();
    TEST_ASSERT_TRUE_MESSAGE(mods_all_off(&ks), "a modifier is still armed");
}

/* What power_filter returned last. The gate keeps its own copy; this one only lets the test state what an eaten sample
 * must equal: the previous return, released. */
static td_touch_sample_t s_prev;
static uint16_t s_id;

/* Every sample has its own x, so "passed unchanged" and "eaten" can never look alike. */
static td_touch_sample_t sample(bool pressed)
{
    s_id++;
    return (td_touch_sample_t){ .x = (uint16_t)(s_id * 3), .y = (uint16_t)(s_id * 5 + 1), .pressed = pressed };
}

static td_touch_sample_t feed(td_touch_sample_t next, bool finger_down, int64_t now)
{
    s_prev = power_filter(next, finger_down, now);
    return s_prev;
}

static void expect_passes_f(td_touch_sample_t next, bool finger_down, int64_t now)
{
    const td_touch_sample_t got = feed(next, finger_down, now);
    EXPECT_SAMPLE(got, next.x, next.y, next.pressed);
}

static void expect_eaten_f(td_touch_sample_t next, bool finger_down, int64_t now)
{
    const td_touch_sample_t before = s_prev;
    const td_touch_sample_t got = feed(next, finger_down, now);
    EXPECT_SAMPLE(got, before.x, before.y, false);
}

/* A real finger is down exactly when the sample says pressed. */
static void expect_passes(td_touch_sample_t next, int64_t now)
{
    expect_passes_f(next, next.pressed, now);
}

static void expect_eaten(td_touch_sample_t next, int64_t now)
{
    expect_eaten_f(next, next.pressed, now);
}

static power_info_t info_at(int64_t now)
{
    power_info_t info;
    TEST_ASSERT_EQUAL(ESP_OK, power_get_info(&info, now));
    return info;
}

static void expect_counters(const power_info_t *i, uint32_t swallowed, uint32_t off, uint32_t wake, uint32_t errors)
{
    TEST_ASSERT_EQUAL_UINT32(swallowed, i->swallowed);
    TEST_ASSERT_EQUAL_UINT32(off, i->off_count);
    TEST_ASSERT_EQUAL_UINT32(wake, i->wake_count);
    TEST_ASSERT_EQUAL_UINT32(errors, i->errors);
}

static void expect_wake_us_sane(const power_info_t *i)
{
    TEST_ASSERT_GREATER_OR_EQUAL_UINT32(200, i->wake_us);
    TEST_ASSERT_LESS_THAN_UINT32(150000, i->wake_us);
}

/* A lit chip, a started gate with the clock at T0, modifiers off and no history. One released sample goes through first,
 * so the sample the gate remembers is one the test knows and not whatever a start leaves behind. */
static void fresh(void)
{
    ensure_keys();
    keys_clear_all();
    TEST_ASSERT_EQUAL(ESP_OK, start_chip());
    TEST_ASSERT_EQUAL_HEX8(LIT, ch422g_output());
    TEST_ASSERT_TRUE(physical_lit());
    writes_clear();
    s_prev = (td_touch_sample_t){ 0 };
    s_id = 0;
    TEST_ASSERT_EQUAL(ESP_OK, power_start(T0));
    expect_passes(sample(false), T0);
    expect_write_count(0);
}

static int64_t go_dark(int64_t now, uint32_t timeout_s)
{
    TEST_ASSERT_EQUAL(ESP_OK, power_set_idle_timeout_s(timeout_s, now));
    const int64_t at = now + (int64_t)timeout_s * SEC;
    expect_passes(sample(false), at);
    TEST_ASSERT_EQUAL_HEX8(DARK, ch422g_output());
    TEST_ASSERT_FALSE(physical_lit());
    writes_clear();
    return at;
}

static void idle_passes(int64_t from, int64_t until)
{
    for (int64_t t = from; t < until; t += 10 * MS) {
        expect_passes(sample(false), t);
    }
    expect_write_count(0);
}

/* The screen stays lit up to one microsecond before `deadline`, and the sample at `deadline` darkens it. */
static void expect_goes_dark_at(int64_t from, int64_t deadline)
{
    idle_passes(from, deadline);
    expect_passes(sample(false), deadline - 1);
    expect_write_count(0);
    TEST_ASSERT_EQUAL_HEX8(LIT, ch422g_output());
    expect_passes(sample(false), deadline);
    TEST_ASSERT_EQUAL_HEX8(DARK, ch422g_output());
}

/* A real finger taps the dark screen: an eaten press at `at` and the eaten closing sample 10 ms later, which is returned.
 * The wake write is left in the recorder. */
static int64_t eaten_real_tap(int64_t at)
{
    expect_eaten_f(sample(true), true, at);
    expect_eaten_f(sample(false), false, at + 10 * MS);
    return at + 10 * MS;
}

TEST_CASE("a started gate is active with the default timeout and every counter at zero", "[power]")
{
    fresh();
    const power_info_t info = info_at(T0);
    TEST_ASSERT_EQUAL(POWER_ACTIVE, info.state);
    TEST_ASSERT_EQUAL_UINT32(600, info.idle_timeout_s);
    TEST_ASSERT_EQUAL_HEX8(LIT, info.output);
    TEST_ASSERT_FALSE(info.swallowing);
    TEST_ASSERT_EQUAL_UINT32(0, info.idle_ms);
    TEST_ASSERT_EQUAL_UINT32(0, info.wake_us);
    expect_counters(&info, 0, 0, 0, 0);
}

TEST_CASE("a chip that failed to start makes the gate refuse everything and pass every sample", "[power]")
{
    ensure_keys();
    keys_clear_all();

    /* A chip that failed to start has its byte at 0. */
    memset(&s_fake, 0, sizeof s_fake);
    s_fake.fail_next = 1;
    const ch422g_platform_t platform = {
        .write = fake_write,
        .delay_ms = fake_delay_ms,
        .hold_touch_int_low = fake_hold_touch_int_low,
        .ctx = &s_fake,
    };
    TEST_ASSERT_EQUAL(ESP_ERR_INVALID_RESPONSE, ch422g_start(&platform));
    TEST_ASSERT_EQUAL_HEX8(0x00, ch422g_output());
    writes_clear();
    s_prev = (td_touch_sample_t){ 0 };

    power_info_t info;
    TEST_ASSERT_EQUAL(ESP_ERR_INVALID_STATE, power_start(T0));
    TEST_ASSERT_EQUAL(ESP_ERR_INVALID_STATE, power_get_info(&info, T0));
    TEST_ASSERT_EQUAL(ESP_ERR_INVALID_STATE, power_set_idle_timeout_s(5, T0));
    expect_passes(sample(false), T0 + 1000 * SEC);
    expect_passes(sample(true), T0 + 1001 * SEC);
    expect_passes(sample(true), T0 + 1002 * SEC);
    expect_passes(sample(false), T0 + 1003 * SEC);
    expect_passes_f(sample(true), false, T0 + 1004 * SEC);
    expect_passes_f(sample(false), false, T0 + 1005 * SEC);
    expect_write_count(0);
}

TEST_CASE("a chip that started dark starts the gate, which reads OFF and eats the first touch and lights the screen",
          "[power]")
{
    fresh();
    external_backlight(false);
    TEST_ASSERT_EQUAL_HEX8(DARK, ch422g_output());
    TEST_ASSERT_FALSE(physical_lit());

    s_prev = (td_touch_sample_t){ 0 };
    TEST_ASSERT_EQUAL(ESP_OK, power_start(T0));
    power_info_t info = info_at(T0);
    TEST_ASSERT_EQUAL(POWER_OFF, info.state);
    TEST_ASSERT_EQUAL_HEX8(DARK, info.output);
    TEST_ASSERT_EQUAL_UINT32(600, info.idle_timeout_s);
    TEST_ASSERT_FALSE(info.swallowing);
    expect_counters(&info, 0, 0, 0, 0);
    TEST_ASSERT_EQUAL(ESP_OK, power_set_idle_timeout_s(600, T0));

    expect_passes(sample(false), T0 + SEC);
    expect_write_count(0);

    expect_eaten(sample(true), T0 + 2 * SEC);
    expect_write_count(1);
    expect_write(0, LIT, true);
    TEST_ASSERT_TRUE(physical_lit());
    expect_eaten(sample(false), T0 + 2 * SEC + 10 * MS);
    info = info_at(T0 + 2 * SEC + 10 * MS);
    TEST_ASSERT_EQUAL(POWER_ACTIVE, info.state);
    TEST_ASSERT_EQUAL_HEX8(LIT, info.output);
    TEST_ASSERT_FALSE(info.swallowing);
    expect_counters(&info, 2, 0, 1, 0);
    expect_wake_us_sane(&info);

    expect_passes(sample(true), T0 + 3 * SEC);
    expect_passes(sample(false), T0 + 3 * SEC + 10 * MS);
    expect_write_count(1);
}

TEST_CASE("the screen goes dark exactly when the idle time reaches the timeout and clears the modifiers first", "[power]")
{
    fresh();
    TEST_ASSERT_EQUAL(ESP_OK, power_set_idle_timeout_s(5, T0));
    lock_ctrl();

    expect_goes_dark_at(T0 + 10 * MS, T0 + 5 * SEC);

    expect_write_count(1);
    expect_write(0, DARK, true);
    TEST_ASSERT_TRUE_MESSAGE(s_fake.writes[0].mods_clear, "a modifier was still armed when the backlight went off");
    expect_mods_off();
    const power_info_t info = info_at(T0 + 5 * SEC);
    TEST_ASSERT_EQUAL(POWER_OFF, info.state);
    TEST_ASSERT_EQUAL_HEX8(DARK, info.output);
    expect_counters(&info, 0, 1, 0, 0);
}

TEST_CASE("a touch moves the deadline to its last touching sample plus the timeout", "[power]")
{
    fresh();
    TEST_ASSERT_EQUAL(ESP_OK, power_set_idle_timeout_s(5, T0));
    idle_passes(T0 + 10 * MS, T0 + 4 * SEC);
    expect_passes(sample(true), T0 + 4 * SEC);
    expect_passes(sample(false), T0 + 4 * SEC + 10 * MS);

    expect_goes_dark_at(T0 + 4 * SEC + 20 * MS, T0 + 9 * SEC);
    expect_write_count(1);
    expect_write(0, DARK, true);
}

TEST_CASE("a press held for twice the timeout never darkens the screen and the idle time counts from its last sample",
          "[power]")
{
    fresh();
    TEST_ASSERT_EQUAL(ESP_OK, power_set_idle_timeout_s(5, T0));
    for (int k = 1; k <= 1000; k++) {
        expect_passes(sample(true), T0 + k * 10 * MS);
    }
    expect_write_count(0);
    TEST_ASSERT_EQUAL_HEX8(LIT, ch422g_output());
    TEST_ASSERT_EQUAL_UINT32(0, info_at(T0 + 10 * SEC).off_count);

    const int64_t last_touch = T0 + 10 * SEC;
    expect_passes(sample(false), last_touch + 10 * MS);
    expect_goes_dark_at(last_touch + 20 * MS, last_touch + 5 * SEC);
}

TEST_CASE("a tap on the dark screen is eaten and lights it, and the next tap passes", "[power]")
{
    fresh();
    const int64_t t = go_dark(T0, 5);

    expect_eaten(sample(true), t + SEC);
    expect_write_count(1);
    expect_write(0, LIT, true);
    TEST_ASSERT_TRUE(physical_lit());
    power_info_t info = info_at(t + SEC);
    TEST_ASSERT_EQUAL(POWER_ACTIVE, info.state);
    TEST_ASSERT_EQUAL_HEX8(LIT, info.output);
    TEST_ASSERT_TRUE(info.swallowing);
    expect_counters(&info, 1, 1, 1, 0);
    expect_wake_us_sane(&info);

    expect_eaten(sample(false), t + SEC + 10 * MS);
    info = info_at(t + SEC + 10 * MS);
    TEST_ASSERT_FALSE(info.swallowing);
    expect_counters(&info, 2, 1, 1, 0);

    expect_passes(sample(true), t + 2 * SEC);
    expect_passes(sample(false), t + 2 * SEC + 10 * MS);
    info = info_at(t + 2 * SEC + 10 * MS);
    expect_counters(&info, 2, 1, 1, 0);
    expect_write_count(1);
}

TEST_CASE("a held press and a swipe that start on the dark screen are eaten whole", "[power]")
{
    static td_touch_sample_t gesture[TD_TOUCH_QUEUE_CAP];
    fresh();
    int64_t t = go_dark(T0, 5);

    size_t n = td_touch_swipe(500, 300, 500, 300, 1500, gesture, TD_TOUCH_QUEUE_CAP);
    TEST_ASSERT_EQUAL_UINT32(152, n);
    for (size_t i = 0; i < n; i++) {
        expect_eaten(gesture[i], t + SEC + (int64_t)i * 10 * MS);
        TEST_ASSERT_EQUAL_MESSAGE(i + 1 < n, info_at(t + SEC + (int64_t)i * 10 * MS).swallowing,
                                  "swallowing must hold until the release has been eaten");
    }
    power_info_t info = info_at(t + 3 * SEC);
    TEST_ASSERT_FALSE(info.swallowing);
    expect_counters(&info, 152, 1, 1, 0);
    TEST_ASSERT_EQUAL_HEX8(LIT, ch422g_output());
    expect_write_count(1);

    t += 3 * SEC;
    t = go_dark(t, 5);
    n = td_touch_swipe(900, 300, 100, 300, 300, gesture, TD_TOUCH_QUEUE_CAP);
    TEST_ASSERT_EQUAL_UINT32(32, n);
    for (size_t i = 0; i < n; i++) {
        expect_eaten(gesture[i], t + SEC + (int64_t)i * 10 * MS);
    }
    info = info_at(t + 2 * SEC);
    TEST_ASSERT_FALSE(info.swallowing);
    expect_counters(&info, 152 + 32, 2, 2, 0);
}

TEST_CASE("an injected tap is eaten on the dark screen and passes on a lit one", "[power]")
{
    static td_touch_queue_t queue;
    const td_touch_sample_t bare = { .x = 300, .y = 200, .pressed = false };
    td_touch_sample_t tap[2];
    bool injected = false;
    fresh();
    memset(&queue, 0, sizeof queue);
    const int64_t t = go_dark(T0, 5);

    td_touch_tap(512, 490, tap);
    TEST_ASSERT_TRUE(td_touch_queue_push(&queue, tap, 2));
    for (int i = 0; i < 2; i++) {
        const td_touch_sample_t next = td_touch_next(&queue, bare, &injected);
        TEST_ASSERT_TRUE(injected);
        expect_eaten_f(next, false, t + SEC + i * 10 * MS);
    }
    power_info_t info = info_at(t + SEC + 20 * MS);
    TEST_ASSERT_EQUAL(POWER_ACTIVE, info.state);
    expect_counters(&info, 2, 1, 1, 0);

    TEST_ASSERT_TRUE(td_touch_queue_push(&queue, tap, 2));
    for (int i = 0; i < 2; i++) {
        const td_touch_sample_t next = td_touch_next(&queue, bare, &injected);
        TEST_ASSERT_TRUE(injected);
        expect_passes_f(next, false, t + 2 * SEC + i * 10 * MS);
    }
    const td_touch_sample_t drained = td_touch_next(&queue, bare, &injected);
    TEST_ASSERT_FALSE(injected);
    expect_passes_f(drained, false, t + 2 * SEC + 20 * MS);
    info = info_at(t + 2 * SEC + 20 * MS);
    expect_counters(&info, 2, 1, 1, 0);
}

TEST_CASE("a real finger down across an injected release keeps the contact eaten until the first sample without one",
          "[power]")
{
    static td_touch_queue_t queue;
    const td_touch_sample_t bare = { .x = 300, .y = 200, .pressed = false };
    td_touch_sample_t tap[2];
    bool injected = false;
    fresh();
    memset(&queue, 0, sizeof queue);
    int64_t t = go_dark(T0, 5) + SEC;

    td_touch_tap(512, 490, tap);
    TEST_ASSERT_TRUE(td_touch_queue_push(&queue, tap, 2));
    td_touch_sample_t next = td_touch_next(&queue, bare, &injected);
    TEST_ASSERT_TRUE(next.pressed);
    expect_eaten_f(next, false, t);
    TEST_ASSERT_TRUE(info_at(t).swallowing);

    t += 10 * MS;
    next = td_touch_next(&queue, bare, &injected);
    TEST_ASSERT_FALSE(next.pressed);
    expect_eaten_f(next, true, t);
    TEST_ASSERT_TRUE_MESSAGE(info_at(t).swallowing, "the real finger is still down, so the contact is still open");

    for (int i = 0; i < 3; i++) {
        t += 10 * MS;
        expect_eaten_f(sample(true), true, t);
        TEST_ASSERT_TRUE(info_at(t).swallowing);
    }

    t += 10 * MS;
    expect_eaten_f(sample(false), false, t);
    const power_info_t info = info_at(t);
    TEST_ASSERT_FALSE(info.swallowing);
    expect_counters(&info, 6, 1, 1, 0);

    expect_passes_f(sample(true), true, t + 300 * MS);
    expect_passes_f(sample(false), false, t + 310 * MS);
}

TEST_CASE("released samples on the dark screen pass unchanged and are not counted", "[power]")
{
    fresh();
    const int64_t t = go_dark(T0, 5);
    const int64_t at[] = { t + SEC, t + 2 * SEC, t + 10 * SEC, t + 1000 * SEC };
    for (size_t i = 0; i < sizeof at / sizeof at[0]; i++) {
        expect_passes(sample(false), at[i]);
    }
    expect_write_count(0);
    TEST_ASSERT_EQUAL_HEX8(DARK, ch422g_output());
    const power_info_t info = info_at(t + 1000 * SEC);
    TEST_ASSERT_EQUAL(POWER_OFF, info.state);
    TEST_ASSERT_FALSE(info.swallowing);
    expect_counters(&info, 0, 1, 0, 0);
}

TEST_CASE("a contact that started lit keeps passing when the backlight goes dark under it", "[power]")
{
    fresh();
    expect_passes(sample(true), T0 + SEC);
    external_backlight(false);
    TEST_ASSERT_EQUAL(POWER_OFF, info_at(T0 + SEC).state);
    for (int i = 1; i <= 3; i++) {
        expect_passes(sample(true), T0 + SEC + i * 10 * MS);
    }
    expect_passes(sample(false), T0 + SEC + 40 * MS);
    expect_write_count(0);
    power_info_t info = info_at(T0 + SEC + 40 * MS);
    TEST_ASSERT_EQUAL(POWER_OFF, info.state);
    expect_counters(&info, 0, 0, 0, 0);

    expect_eaten(sample(true), T0 + 2 * SEC);
    TEST_ASSERT_EQUAL_HEX8(LIT, ch422g_output());
    expect_eaten(sample(false), T0 + 2 * SEC + 10 * MS);
    info = info_at(T0 + 2 * SEC + 10 * MS);
    TEST_ASSERT_EQUAL(POWER_ACTIVE, info.state);
    expect_counters(&info, 2, 0, 1, 0);
    expect_write_count(1);
    expect_write(0, LIT, true);
}

TEST_CASE("an external backlight off makes the next touch a wake", "[power]")
{
    fresh();
    external_backlight(false);
    TEST_ASSERT_EQUAL(POWER_OFF, info_at(T0 + SEC).state);

    expect_eaten(sample(true), T0 + SEC);
    expect_eaten(sample(false), T0 + SEC + 10 * MS);
    TEST_ASSERT_EQUAL_HEX8(LIT, ch422g_output());
    expect_write_count(1);
    expect_write(0, LIT, true);
    power_info_t info = info_at(T0 + SEC + 10 * MS);
    TEST_ASSERT_EQUAL(POWER_ACTIVE, info.state);
    expect_counters(&info, 2, 0, 1, 0);

    expect_passes(sample(true), T0 + 2 * SEC);
    expect_passes(sample(false), T0 + 2 * SEC + 10 * MS);
}

TEST_CASE("an external backlight on restarts the idle clock of a dark screen", "[power]")
{
    fresh();
    const int64_t t = go_dark(T0, 5);
    external_backlight(true);

    const int64_t noticed = t + 95 * SEC;
    expect_goes_dark_at(noticed, noticed + 5 * SEC);
    expect_write_count(1);
    expect_write(0, DARK, true);
    const power_info_t info = info_at(noticed + 5 * SEC);
    TEST_ASSERT_EQUAL(POWER_OFF, info.state);
    expect_counters(&info, 0, 2, 0, 0);
}

TEST_CASE("a real finger that drops for one read mid-touch on the dark screen is eaten whole and wakes the screen once",
          "[power]")
{
    fresh();
    const int64_t t = go_dark(T0, 5);
    const int64_t at = t + SEC;

    expect_eaten_f(sample(true), true, at);
    expect_write_count(1);
    expect_write(0, LIT, true);
    expect_eaten_f(sample(true), true, at + 10 * MS);
    expect_eaten_f(sample(false), false, at + 20 * MS);
    TEST_ASSERT_FALSE(info_at(at + 20 * MS).swallowing);
    expect_passes(sample(false), at + 30 * MS);
    expect_passes(sample(false), at + 40 * MS);

    expect_eaten_f(sample(true), true, at + 50 * MS);
    TEST_ASSERT_TRUE(info_at(at + 50 * MS).swallowing);
    expect_eaten_f(sample(true), true, at + 60 * MS);
    expect_eaten_f(sample(false), false, at + 70 * MS);

    expect_write_count(1);
    TEST_ASSERT_EQUAL_HEX8(LIT, ch422g_output());
    TEST_ASSERT_TRUE(physical_lit());
    const power_info_t info = info_at(at + 70 * MS);
    TEST_ASSERT_EQUAL(POWER_ACTIVE, info.state);
    TEST_ASSERT_FALSE(info.swallowing);
    expect_counters(&info, 6, 1, 1, 0);
}

TEST_CASE("an injected press 30 ms after an eaten close passes", "[power]")
{
    fresh();
    const int64_t t = go_dark(T0, 5);
    const int64_t close = eaten_real_tap(t + SEC);
    writes_clear();

    idle_passes(close + 10 * MS, close + 30 * MS);
    expect_passes_f(sample(true), false, close + 30 * MS);
    expect_passes_f(sample(false), false, close + 40 * MS);
    expect_write_count(0);
    const power_info_t info = info_at(close + 40 * MS);
    TEST_ASSERT_FALSE(info.swallowing);
    expect_counters(&info, 2, 1, 1, 0);
}

/* A real finger lands `gap` after the closing sample of an eaten contact, on a screen that is lit again. */
static void real_finger_after_eaten_close(int64_t gap, bool eaten)
{
    fresh();
    const int64_t t = go_dark(T0, 5);
    const int64_t close = eaten_real_tap(t + SEC);
    writes_clear();

    idle_passes(close + 10 * MS, close + gap);
    const int64_t at = close + gap;
    if (eaten) {
        expect_eaten_f(sample(true), true, at);
        TEST_ASSERT_TRUE(info_at(at).swallowing);
        expect_eaten_f(sample(false), false, at + 10 * MS);
    } else {
        expect_passes_f(sample(true), true, at);
        TEST_ASSERT_FALSE(info_at(at).swallowing);
        expect_passes_f(sample(false), false, at + 10 * MS);
    }
    expect_write_count(0);
    TEST_ASSERT_EQUAL_HEX8(LIT, ch422g_output());
    const power_info_t info = info_at(at + 10 * MS);
    TEST_ASSERT_FALSE(info.swallowing);
    expect_counters(&info, eaten ? 4 : 2, 1, 1, 0);
}

TEST_CASE("a real finger 240 ms after an eaten close is eaten, 260 ms after it passes", "[power]")
{
    real_finger_after_eaten_close(240 * MS, true);
    real_finger_after_eaten_close(260 * MS, false);
}

TEST_CASE("a real finger press 300 ms after an eaten close passes", "[power]")
{
    real_finger_after_eaten_close(300 * MS, false);
}

TEST_CASE("a contact eaten by the guard extends it, and a finger 260 ms after the last one passes", "[power]")
{
    fresh();
    const int64_t t = go_dark(T0, 5);
    int64_t close = eaten_real_tap(t + SEC);
    writes_clear();

    for (int k = 0; k < 3; k++) {
        const int64_t at = close + 240 * MS;
        expect_eaten_f(sample(true), true, at);
        expect_eaten_f(sample(false), false, at + 10 * MS);
        close = at + 10 * MS;
    }
    expect_write_count(0);
    power_info_t info = info_at(close);
    expect_counters(&info, 8, 1, 1, 0);

    expect_passes_f(sample(true), true, close + 260 * MS);
    expect_passes_f(sample(false), false, close + 270 * MS);
    info = info_at(close + 270 * MS);
    expect_counters(&info, 8, 1, 1, 0);
}

TEST_CASE("a failed dark write leaves the screen lit, heals it and retries after five seconds", "[power]")
{
    fresh();
    TEST_ASSERT_EQUAL(ESP_OK, power_set_idle_timeout_s(5, T0));
    lock_ctrl();
    s_fake.fail_next = 1;

    expect_passes(sample(false), T0 + 5 * SEC);
    expect_write_count(2);
    expect_write(0, DARK, false);
    expect_write(1, LIT, true);
    TEST_ASSERT_TRUE_MESSAGE(s_fake.writes[0].mods_clear, "a modifier was still armed at the failed dark write");
    expect_mods_off();
    TEST_ASSERT_EQUAL_HEX8(LIT, ch422g_output());
    TEST_ASSERT_TRUE(physical_lit());
    power_info_t info = info_at(T0 + 5 * SEC);
    TEST_ASSERT_EQUAL(POWER_ACTIVE, info.state);
    TEST_ASSERT_EQUAL_HEX8(LIT, info.output);
    TEST_ASSERT_EQUAL_UINT32(1, info.errors);
    TEST_ASSERT_EQUAL_UINT32(0, info.off_count);
    TEST_ASSERT_EQUAL_UINT32(0, info.wake_count);

    writes_clear();
    idle_passes(T0 + 5 * SEC + 10 * MS, T0 + 10 * SEC);
    expect_passes(sample(false), T0 + 10 * SEC - 1);
    expect_write_count(0);

    expect_passes(sample(false), T0 + 10 * SEC);
    expect_write_count(1);
    expect_write(0, DARK, true);
    TEST_ASSERT_TRUE(s_fake.writes[0].mods_clear);
    info = info_at(T0 + 10 * SEC);
    TEST_ASSERT_EQUAL(POWER_OFF, info.state);
    TEST_ASSERT_EQUAL_HEX8(DARK, info.output);
    TEST_ASSERT_EQUAL_UINT32(1, info.off_count);
    TEST_ASSERT_EQUAL_UINT32(1, info.errors);
}

TEST_CASE("a failed wake write still eats the contact and the next contact retries it", "[power]")
{
    fresh();
    const int64_t t = go_dark(T0, 5);
    s_fake.fail_next = 1;

    const int64_t first = t + SEC;
    expect_eaten(sample(true), first);
    expect_write_count(1);
    expect_write(0, LIT, false);
    TEST_ASSERT_EQUAL_HEX8(DARK, ch422g_output());
    power_info_t info = info_at(first);
    TEST_ASSERT_EQUAL(POWER_OFF, info.state);
    TEST_ASSERT_TRUE(info.swallowing);
    expect_counters(&info, 1, 1, 0, 1);

    expect_eaten(sample(false), first + 10 * MS);
    expect_write_count(1);
    info = info_at(first + 10 * MS);
    TEST_ASSERT_EQUAL(POWER_OFF, info.state);
    TEST_ASSERT_FALSE(info.swallowing);
    expect_counters(&info, 2, 1, 0, 1);

    expect_eaten(sample(true), first + 500 * MS);
    expect_write_count(2);
    expect_write(1, LIT, true);
    expect_eaten(sample(false), first + 510 * MS);
    info = info_at(first + 510 * MS);
    TEST_ASSERT_EQUAL(POWER_ACTIVE, info.state);
    TEST_ASSERT_EQUAL_HEX8(LIT, info.output);
    expect_counters(&info, 4, 1, 1, 1);
}

TEST_CASE("the first sample of the next contact retries a failed wake write at once, without waiting 250 ms", "[power]")
{
    fresh();
    const int64_t t = go_dark(T0, 5);
    s_fake.fail_next = 1;

    const int64_t first = t + SEC;
    expect_eaten_f(sample(true), false, first);
    expect_write_count(1);
    expect_write(0, LIT, false);
    expect_eaten_f(sample(false), false, first + 10 * MS);
    TEST_ASSERT_EQUAL(POWER_OFF, info_at(first + 10 * MS).state);

    /* An injected press, so the real-finger guard has nothing to say, 100 ms after the failed contact ended. */
    expect_eaten_f(sample(true), false, first + 110 * MS);
    expect_write_count(2);
    expect_write(1, LIT, true);
    TEST_ASSERT_EQUAL_HEX8(LIT, ch422g_output());
    TEST_ASSERT_TRUE(physical_lit());
    expect_eaten_f(sample(false), false, first + 120 * MS);
    expect_write_count(2);
    const power_info_t info = info_at(first + 120 * MS);
    TEST_ASSERT_EQUAL(POWER_ACTIVE, info.state);
    TEST_ASSERT_FALSE(info.swallowing);
    expect_counters(&info, 4, 1, 1, 1);

    expect_passes_f(sample(true), false, first + 2 * SEC);
    expect_passes_f(sample(false), false, first + 2 * SEC + 10 * MS);
    expect_write_count(2);
}

TEST_CASE("a failed wake write on a long contact is retried on a later sample after 250 ms", "[power]")
{
    fresh();
    const int64_t t = go_dark(T0, 5);
    s_fake.fail_next = 1;

    const int64_t first = t + SEC;
    expect_eaten(sample(true), first);
    for (int i = 1; i <= 24; i++) {
        expect_eaten(sample(true), first + i * 10 * MS);
    }
    expect_write_count(1);
    TEST_ASSERT_EQUAL_HEX8(DARK, ch422g_output());
    TEST_ASSERT_EQUAL(POWER_OFF, info_at(first + 240 * MS).state);

    expect_eaten(sample(true), first + 260 * MS);
    expect_write_count(2);
    expect_write(1, LIT, true);
    TEST_ASSERT_EQUAL_HEX8(LIT, ch422g_output());
    expect_eaten(sample(true), first + 270 * MS);
    expect_eaten(sample(false), first + 280 * MS);
    expect_write_count(2);
    const power_info_t info = info_at(first + 280 * MS);
    TEST_ASSERT_EQUAL(POWER_ACTIVE, info.state);
    TEST_ASSERT_FALSE(info.swallowing);
    TEST_ASSERT_EQUAL_UINT32(1, info.wake_count);
    TEST_ASSERT_EQUAL_UINT32(1, info.errors);
    TEST_ASSERT_EQUAL_UINT32(28, info.swallowed);
}

/* The dark write fails and the chip takes the byte anyway, then the lit write that heals it fails and does not land. The
 * driver's cache says lit, the real backlight is dark. Returns the time of the sample that did it. */
static int64_t doubt_setup(void)
{
    fresh();
    TEST_ASSERT_EQUAL(ESP_OK, power_set_idle_timeout_s(5, T0));
    lock_ctrl();
    s_fake.fail_next = 2;
    s_fake.latch_on_fail = 0x1;

    expect_passes(sample(false), T0 + 5 * SEC);
    expect_write_count(2);
    expect_write(0, DARK, false);
    expect_write(1, LIT, false);
    TEST_ASSERT_TRUE_MESSAGE(s_fake.writes[0].mods_clear, "a modifier was still armed at the failed dark write");
    expect_mods_off();
    TEST_ASSERT_EQUAL_HEX8(LIT, ch422g_output());
    TEST_ASSERT_FALSE(physical_lit());
    const power_info_t info = info_at(T0 + 5 * SEC);
    TEST_ASSERT_EQUAL(POWER_ACTIVE, info.state);
    TEST_ASSERT_EQUAL_HEX8(LIT, info.output);
    expect_counters(&info, 0, 0, 0, 2);
    writes_clear();
    return T0 + 5 * SEC;
}

static void doubt_touch_is_eaten(bool real_finger)
{
    const int64_t t = doubt_setup();
    const int64_t at = t + SEC;

    expect_eaten_f(sample(true), real_finger, at);
    expect_write_count(1);
    expect_write(0, LIT, true);
    TEST_ASSERT_TRUE(physical_lit());
    TEST_ASSERT_EQUAL_HEX8(LIT, ch422g_output());
    power_info_t info = info_at(at);
    TEST_ASSERT_EQUAL(POWER_ACTIVE, info.state);
    TEST_ASSERT_TRUE(info.swallowing);
    expect_counters(&info, 1, 0, 1, 2);

    expect_eaten_f(sample(false), false, at + 10 * MS);
    info = info_at(at + 10 * MS);
    TEST_ASSERT_FALSE(info.swallowing);
    expect_counters(&info, 2, 0, 1, 2);

    /* The write was acknowledged, so the doubt is gone: the next touch passes. */
    expect_passes_f(sample(true), real_finger, at + SEC);
    expect_passes_f(sample(false), false, at + SEC + 10 * MS);
    expect_write_count(1);
    info = info_at(at + SEC + 10 * MS);
    expect_counters(&info, 2, 0, 1, 2);
}

TEST_CASE("a dark write that fails and lands, then a failed heal, leaves the chip dark and a real touch is eaten and relights it",
          "[power]")
{
    doubt_touch_is_eaten(true);
}

TEST_CASE("a dark write that fails and lands, then a failed heal, leaves the chip dark and an injected touch is eaten and relights it",
          "[power]")
{
    doubt_touch_is_eaten(false);
}

TEST_CASE("a failed heal that the chip took still casts doubt and the next touch is eaten", "[power]")
{
    fresh();
    TEST_ASSERT_EQUAL(ESP_OK, power_set_idle_timeout_s(5, T0));
    s_fake.fail_next = 2;
    s_fake.latch_on_fail = 0x2;  /* only the heal lands, so the chip is lit but the gate cannot know it */

    expect_passes(sample(false), T0 + 5 * SEC);
    expect_write_count(2);
    expect_write(0, DARK, false);
    expect_write(1, LIT, false);
    TEST_ASSERT_TRUE(physical_lit());
    TEST_ASSERT_EQUAL_HEX8(LIT, ch422g_output());
    power_info_t info = info_at(T0 + 5 * SEC);
    expect_counters(&info, 0, 0, 0, 2);
    writes_clear();

    expect_eaten(sample(true), T0 + 6 * SEC);
    expect_write_count(1);
    expect_write(0, LIT, true);
    expect_eaten(sample(false), T0 + 6 * SEC + 10 * MS);
    info = info_at(T0 + 6 * SEC + 10 * MS);
    expect_counters(&info, 2, 0, 1, 2);

    expect_passes(sample(true), T0 + 7 * SEC);
    expect_passes(sample(false), T0 + 7 * SEC + 10 * MS);
    expect_write_count(1);
}

TEST_CASE("a heal that succeeds after a dark write that landed relights the chip and clears the doubt", "[power]")
{
    fresh();
    TEST_ASSERT_EQUAL(ESP_OK, power_set_idle_timeout_s(5, T0));
    s_fake.fail_next = 1;
    s_fake.latch_on_fail = 0x1;

    expect_passes(sample(false), T0 + 5 * SEC);
    expect_write_count(2);
    expect_write(0, DARK, false);
    expect_write(1, LIT, true);
    TEST_ASSERT_TRUE(physical_lit());
    TEST_ASSERT_EQUAL_HEX8(LIT, ch422g_output());
    power_info_t info = info_at(T0 + 5 * SEC);
    TEST_ASSERT_EQUAL(POWER_ACTIVE, info.state);
    expect_counters(&info, 0, 0, 0, 1);
    writes_clear();

    expect_passes(sample(true), T0 + 6 * SEC);
    expect_passes(sample(false), T0 + 6 * SEC + 10 * MS);
    expect_passes_f(sample(true), false, T0 + 7 * SEC);
    expect_passes_f(sample(false), false, T0 + 7 * SEC + 10 * MS);
    expect_write_count(0);
    info = info_at(T0 + 7 * SEC + 10 * MS);
    expect_counters(&info, 0, 0, 0, 1);
}

TEST_CASE("a rewrite that fails keeps the doubt until a write is acknowledged, retried by the next contact at once",
          "[power]")
{
    const int64_t t = doubt_setup();
    const int64_t at = t + SEC;
    s_fake.fail_next = 1;

    expect_eaten_f(sample(true), false, at);
    expect_write_count(1);
    expect_write(0, LIT, false);
    TEST_ASSERT_FALSE(physical_lit());
    power_info_t info = info_at(at);
    TEST_ASSERT_EQUAL(POWER_ACTIVE, info.state);
    expect_counters(&info, 1, 0, 0, 3);
    expect_eaten_f(sample(false), false, at + 10 * MS);

    expect_eaten_f(sample(true), false, at + 110 * MS);
    expect_write_count(2);
    expect_write(1, LIT, true);
    TEST_ASSERT_TRUE(physical_lit());
    expect_eaten_f(sample(false), false, at + 120 * MS);
    info = info_at(at + 120 * MS);
    expect_counters(&info, 4, 0, 1, 3);

    expect_passes_f(sample(true), false, at + 2 * SEC);
    expect_passes_f(sample(false), false, at + 2 * SEC + 10 * MS);
    expect_write_count(2);
}

TEST_CASE("the idle timeout accepts 1 to 86400 s, refuses the rest and leaves the clock alone when it refuses", "[power]")
{
    fresh();
    TEST_ASSERT_EQUAL(ESP_ERR_INVALID_ARG, power_set_idle_timeout_s(0, T0 + 3 * SEC));
    TEST_ASSERT_EQUAL(ESP_ERR_INVALID_ARG, power_set_idle_timeout_s(86401, T0 + 3 * SEC));
    TEST_ASSERT_EQUAL(ESP_ERR_INVALID_ARG, power_set_idle_timeout_s(UINT32_MAX, T0 + 3 * SEC));
    power_info_t info = info_at(T0 + 4 * SEC);
    TEST_ASSERT_EQUAL_UINT32(600, info.idle_timeout_s);
    TEST_ASSERT_EQUAL_UINT32(4000, info.idle_ms);

    TEST_ASSERT_EQUAL(ESP_OK, power_set_idle_timeout_s(1, T0 + 5 * SEC));
    TEST_ASSERT_EQUAL_UINT32(1, info_at(T0 + 5 * SEC).idle_timeout_s);
    TEST_ASSERT_EQUAL(ESP_OK, power_set_idle_timeout_s(86400, T0 + 5 * SEC));
    TEST_ASSERT_EQUAL_UINT32(86400, info_at(T0 + 5 * SEC).idle_timeout_s);

    fresh();
    TEST_ASSERT_EQUAL(ESP_OK, power_set_idle_timeout_s(5, T0));
    TEST_ASSERT_EQUAL(ESP_ERR_INVALID_ARG, power_set_idle_timeout_s(0, T0 + 3 * SEC));
    expect_passes(sample(false), T0 + 5 * SEC);
    TEST_ASSERT_EQUAL_HEX8(DARK, ch422g_output());
}

TEST_CASE("setting the idle timeout restarts the idle clock", "[power]")
{
    fresh();
    TEST_ASSERT_EQUAL(ESP_OK, power_set_idle_timeout_s(86400, T0));
    const int64_t later = T0 + 1000 * SEC;
    TEST_ASSERT_EQUAL(ESP_OK, power_set_idle_timeout_s(5, later));
    expect_goes_dark_at(later, later + 5 * SEC);
    expect_write_count(1);
    expect_write(0, DARK, true);
}

TEST_CASE("setting the idle timeout on a dark screen keeps it dark", "[power]")
{
    fresh();
    const int64_t t = go_dark(T0, 5);
    TEST_ASSERT_EQUAL(ESP_OK, power_set_idle_timeout_s(10, t + SEC));
    expect_passes(sample(false), t + 100 * SEC);
    expect_write_count(0);
    const power_info_t info = info_at(t + 100 * SEC);
    TEST_ASSERT_EQUAL(POWER_OFF, info.state);
    TEST_ASSERT_EQUAL_HEX8(DARK, info.output);
    TEST_ASSERT_EQUAL_UINT32(10, info.idle_timeout_s);
}

TEST_CASE("idle_ms counts from the later of the last touching sample and the moment the timeout was set", "[power]")
{
    fresh();
    TEST_ASSERT_EQUAL_UINT32(500, info_at(T0 + 500 * MS).idle_ms);

    TEST_ASSERT_EQUAL(ESP_OK, power_set_idle_timeout_s(86400, T0 + SEC));
    TEST_ASSERT_EQUAL_UINT32(2000, info_at(T0 + 3 * SEC).idle_ms);

    expect_passes(sample(true), T0 + 10 * SEC);
    expect_passes(sample(false), T0 + 10 * SEC + 10 * MS);
    TEST_ASSERT_EQUAL_UINT32(2000, info_at(T0 + 12 * SEC).idle_ms);

    TEST_ASSERT_EQUAL(ESP_OK, power_set_idle_timeout_s(86400, T0 + 20 * SEC));
    TEST_ASSERT_EQUAL_UINT32(1000, info_at(T0 + 21 * SEC).idle_ms);
}

typedef struct {
    bool on;
    uint32_t left;
} run_t;

static bool run_step(run_t *r, uint32_t *rng)
{
    if (r->left == 0) {
        r->on = !r->on;
        r->left = 1 + xorshift(rng) % 30;
    }
    r->left--;
    return r->on;
}

static uint32_t stream(uint32_t seed, uint32_t k)
{
    const uint32_t v = seed ^ (k * 0x9E3779B9u);
    return v ? v : 1;
}

typedef struct {
    unsigned latched_dark;
    unsigned healed;
} fuzz_stats_t;

#define GUARD_US (250 * MS)

/*
 * The oracle: a contact is a maximal run of steps with a finger, real or injected. Its fate is decided at its first step
 * and every step of it, plus the step after it, is delivered unchanged if the contact was passed and eaten if it was
 * eaten. The contact is eaten when
 *   - the real backlight (the fake chip's byte) is dark, or
 *   - the driver's cached byte says dark: a failed lit write that still landed leaves the real backlight lit while the
 *     driver, and so the gate, takes the screen for dark, or
 *   - the gate is in doubt: a lit write that heals a failed dark write failed too, so the real backlight is unknown until
 *     some write is acknowledged, or
 *   - its first step has a real finger and the closing step of the last eaten contact was less than 250 ms ago.
 * Doubt is tracked here from the writes the gate made. The test's own backlight writes wait while a doubt is open, so
 * that nothing but the gate's own acknowledged write can resolve it.
 */
static void fuzz_one(uint32_t seed, uint32_t fail_one_in, fuzz_stats_t *stats)
{
    char msg[48];
    fresh();
    uint32_t r_press = stream(seed, 1);
    uint32_t r_finger = stream(seed, 2);
    uint32_t r_time = stream(seed, 3);
    uint32_t r_ext = stream(seed, 4);
    uint32_t r_timeout = stream(seed, 5);
    uint32_t r_arm = stream(seed, 6);
    s_fake.fail_rng = stream(seed, 7);
    s_fake.fail_random = true;
    s_fake.fail_one_in = fail_one_in;
    s_fake.latch_random = true;
    run_t press_run = { 0 };
    run_t finger_run = { 0 };
    int64_t now = T0;
    TEST_ASSERT_EQUAL(ESP_OK, power_set_idle_timeout_s(1 + xorshift(&r_timeout) % 3, now));

    bool in_contact = false;
    bool fate_dark = false;
    bool doubt = false;
    bool awaiting_lit_ack = false;
    bool have_close = false;
    int64_t last_close = 0;
    uint32_t eaten = 0;
    uint32_t want_errors = 0;
    uint32_t want_off = 0;
    uint32_t want_wake = 0;

    for (int step = 0; step < FUZZ_STEPS; step++) {
        snprintf(msg, sizeof msg, "seed 0x%08x step %d", (unsigned)seed, step);

        now += 10 * MS;
        if (xorshift(&r_time) % 50 == 0) {
            now += 500 * MS + (int64_t)(xorshift(&r_time) % 2500001u);
        }
        if (xorshift(&r_ext) % 100 == 0) {
            const bool on = (xorshift(&r_ext) & 1) != 0;
            if (!awaiting_lit_ack) {
                s_fake.external = true;
                const esp_err_t err = ch422g_set_backlight(on);
                s_fake.external = false;
                TEST_ASSERT_EQUAL_MESSAGE(ESP_OK, err, msg);
            }
        }
        if (xorshift(&r_timeout) % 200 == 0) {
            TEST_ASSERT_EQUAL_MESSAGE(ESP_OK, power_set_idle_timeout_s(1 + xorshift(&r_timeout) % 3, now), msg);
        }
        if (xorshift(&r_arm) % 60 == 0) {
            keys_mod((xorshift(&r_arm) & 1) ? KEY_MOD_SHIFT : KEY_MOD_CTRL);
        }

        const bool pressed = run_step(&press_run, &r_press);
        const bool finger = run_step(&finger_run, &r_finger);
        const td_touch_sample_t next = sample(pressed);
        const bool contact = pressed || finger;
        const bool starts = contact && !in_contact;
        const bool closing = !contact && in_contact;
        if (starts) {
            const bool cache_dark = (ch422g_output() & DISP_BIT) == 0;
            const bool guard = finger && have_close && now - last_close < GUARD_US;
            fate_dark = !physical_lit() || cache_dark || doubt || guard;
        }
        const bool want_eaten = (contact || closing) && fate_dark;

        writes_clear();
        const td_touch_sample_t before = s_prev;
        const td_touch_sample_t got = feed(next, finger, now);

        if (want_eaten) {
            EXPECT_SAMPLE_MSG(got, before.x, before.y, false, msg);
            eaten++;
        } else {
            EXPECT_SAMPLE_MSG(got, next.x, next.y, next.pressed, msg);
        }
        if (closing && fate_dark) {
            have_close = true;
            last_close = now;
        }

        TEST_ASSERT_FALSE_MESSAGE(s_fake.overflow, msg);
        bool after_failed_dark = false;
        for (size_t i = 0; i < s_fake.count; i++) {
            const write_rec_t *w = &s_fake.writes[i];
            const bool lit_write = (w->value & DISP_BIT) != 0;
            if (!w->acked) {
                want_errors++;
                if (!lit_write && w->landed) {
                    stats->latched_dark++;
                }
                if (lit_write && after_failed_dark) {
                    doubt = true;
                    awaiting_lit_ack = true;
                }
            } else {
                if (lit_write && !after_failed_dark) {
                    want_wake++;
                } else if (!lit_write) {
                    want_off++;
                }
                if (doubt && lit_write && (w->before & DISP_BIT) == 0) {
                    stats->healed++;
                }
                doubt = false;
                if (lit_write) {
                    awaiting_lit_ack = false;
                }
            }
            if (!lit_write) {
                TEST_ASSERT_TRUE_MESSAGE(w->mods_clear, msg);
                TEST_ASSERT_FALSE_MESSAGE(contact, msg);
            }
            after_failed_dark = !lit_write && !w->acked;
        }

        power_info_t info;
        TEST_ASSERT_EQUAL_MESSAGE(ESP_OK, power_get_info(&info, now), msg);
        TEST_ASSERT_EQUAL_HEX8_MESSAGE(ch422g_output(), info.output, msg);
        TEST_ASSERT_EQUAL_MESSAGE((ch422g_output() & DISP_BIT) ? POWER_ACTIVE : POWER_OFF, info.state, msg);
        TEST_ASSERT_EQUAL_UINT32_MESSAGE(want_errors, info.errors, msg);
        TEST_ASSERT_EQUAL_UINT32_MESSAGE(want_off, info.off_count, msg);
        TEST_ASSERT_EQUAL_UINT32_MESSAGE(want_wake, info.wake_count, msg);
        TEST_ASSERT_EQUAL_UINT32_MESSAGE(eaten, info.swallowed, msg);
        TEST_ASSERT_EQUAL_MESSAGE(contact && fate_dark, info.swallowing, msg);

        in_contact = contact;
    }

    snprintf(msg, sizeof msg, "seed 0x%08x too little exercised", (unsigned)seed);
    TEST_ASSERT_TRUE_MESSAGE(eaten > 0 && want_off > 0 && want_wake > 0 && want_errors > 0, msg);
}

TEST_CASE("a random touch stream never shows LVGL a press the gate should have eaten", "[power]")
{
    static const struct {
        uint32_t seed;
        uint32_t fail_one_in;
    } runs[] = {
        { 0x1234ABCDu, 8 }, { 0xC0FFEE11u, 8 }, { 0x9E3779B9u, 8 }, { 0xDEADBEEFu, 4 },
        { 0x0BADF00Du, 4 }, { 0x5EED5EEDu, 3 }, { 0xA5A5A5A5u, 3 }, { 0x13579BDFu, 5 },
    };
    fuzz_stats_t stats = { 0 };
    for (size_t i = 0; i < sizeof runs / sizeof runs[0]; i++) {
        fuzz_one(runs[i].seed, runs[i].fail_one_in, &stats);
        vTaskDelay(1);  /* let the idle task feed the watchdog between the long runs */
    }
    TEST_ASSERT_GREATER_THAN_UINT32_MESSAGE(0, stats.latched_dark, "no dark write failed and still reached the chip");
    TEST_ASSERT_GREATER_THAN_UINT32_MESSAGE(0, stats.healed, "no rewrite resolved a doubt over a dark chip");
}

TEST_CASE("photo idle mode raises the idle face at its timeout instead of darkening the screen", "[power]")
{
    fresh();
    TEST_ASSERT_EQUAL(ESP_OK, power_set_idle_timeout_s(5, T0));
    TEST_ASSERT_EQUAL(ESP_OK, power_set_photo_idle(true, 10, T0));
    lock_ctrl();
    uint32_t entry = 99;
    idle_passes(T0 + 10 * MS, T0 + 10 * SEC);
    TEST_ASSERT_FALSE(power_idle_face(&entry));
    TEST_ASSERT_EQUAL_UINT32(0, entry);

    expect_passes(sample(false), T0 + 10 * SEC);
    TEST_ASSERT_TRUE(power_idle_face(&entry));
    TEST_ASSERT_EQUAL_UINT32(1, entry);
    expect_write_count(0);
    TEST_ASSERT_EQUAL_HEX8(LIT, ch422g_output());
    expect_mods_off();
    const power_info_t info = info_at(T0 + 10 * SEC);
    TEST_ASSERT_EQUAL(POWER_ACTIVE, info.state);
    TEST_ASSERT_TRUE(info.photo_idle);
    TEST_ASSERT_TRUE(info.idle_face);
    TEST_ASSERT_EQUAL_UINT32(10, info.photo_timeout_s);
    TEST_ASSERT_EQUAL_UINT32(1, info.face_count);
    expect_counters(&info, 0, 0, 0, 0);

    idle_passes(T0 + 10 * SEC + 10 * MS, T0 + 20 * SEC);
    TEST_ASSERT_TRUE(power_idle_face(&entry));
    TEST_ASSERT_EQUAL_UINT32_MESSAGE(1, entry, "the face comes up once per idle period");
}

TEST_CASE("the touch that takes the idle face down is eaten whole, and the idle clock starts over", "[power]")
{
    fresh();
    TEST_ASSERT_EQUAL(ESP_OK, power_set_photo_idle(true, 10, T0));
    expect_passes(sample(false), T0 + 10 * SEC);
    TEST_ASSERT_TRUE(power_idle_face(NULL));

    expect_eaten(sample(true), T0 + 11 * SEC);
    TEST_ASSERT_FALSE(power_idle_face(NULL));
    expect_eaten(sample(true), T0 + 11 * SEC + 10 * MS);
    expect_eaten(sample(false), T0 + 11 * SEC + 20 * MS);
    expect_write_count(0);
    power_info_t info = info_at(T0 + 11 * SEC + 20 * MS);
    TEST_ASSERT_EQUAL_UINT32(1, info.face_wakes);
    expect_counters(&info, 3, 0, 0, 0);

    expect_passes(sample(true), T0 + 12 * SEC);
    expect_passes(sample(false), T0 + 12 * SEC + 10 * MS);
    expect_passes(sample(false), T0 + 22 * SEC - 1);
    TEST_ASSERT_FALSE(power_idle_face(NULL));
    uint32_t entry = 0;
    expect_passes(sample(false), T0 + 22 * SEC);
    TEST_ASSERT_TRUE(power_idle_face(&entry));
    TEST_ASSERT_EQUAL_UINT32(2, entry);
    expect_write_count(0);
}

TEST_CASE("turning photo idle mode off takes the idle face down and the v1 timeout darkens the screen again", "[power]")
{
    fresh();
    TEST_ASSERT_EQUAL(ESP_OK, power_set_idle_timeout_s(5, T0));
    TEST_ASSERT_EQUAL(ESP_OK, power_set_photo_idle(true, 10, T0));
    expect_passes(sample(false), T0 + 10 * SEC);
    TEST_ASSERT_TRUE(power_idle_face(NULL));

    TEST_ASSERT_EQUAL(ESP_OK, power_set_photo_idle(false, 0, T0 + 11 * SEC));
    TEST_ASSERT_FALSE(power_idle_face(NULL));
    TEST_ASSERT_FALSE(info_at(T0 + 11 * SEC).photo_idle);
    expect_goes_dark_at(T0 + 11 * SEC + 10 * MS, T0 + 16 * SEC);
}

TEST_CASE("photo idle mode takes 10 to 86400 s and leaves the gate alone when it refuses", "[power]")
{
    fresh();
    TEST_ASSERT_EQUAL(ESP_ERR_INVALID_ARG, power_set_photo_idle(true, 9, T0));
    TEST_ASSERT_EQUAL(ESP_ERR_INVALID_ARG, power_set_photo_idle(true, 86401, T0));
    TEST_ASSERT_FALSE(info_at(T0).photo_idle);
    TEST_ASSERT_EQUAL(ESP_OK, power_set_photo_idle(true, 10, T0));
    TEST_ASSERT_EQUAL(ESP_OK, power_set_photo_idle(true, 86400, T0));
    TEST_ASSERT_EQUAL_UINT32(86400, info_at(T0).photo_timeout_s);
    TEST_ASSERT_EQUAL(ESP_OK, power_set_photo_idle(false, 0, T0));
    TEST_ASSERT_FALSE(info_at(T0).photo_idle);
}

TEST_CASE("a dark screen in photo idle mode shows no face, and a touch lights it and starts the idle clock", "[power]")
{
    fresh();
    const int64_t t = go_dark(T0, 5);
    TEST_ASSERT_EQUAL(ESP_OK, power_set_photo_idle(true, 10, t));
    expect_passes(sample(false), t + 20 * SEC);
    TEST_ASSERT_FALSE(power_idle_face(NULL));
    expect_write_count(0);

    eaten_real_tap(t + 21 * SEC);
    TEST_ASSERT_TRUE(physical_lit());
    writes_clear();
    expect_passes(sample(false), t + 31 * SEC - 1);
    TEST_ASSERT_FALSE(power_idle_face(NULL));
    expect_passes(sample(false), t + 31 * SEC);
    TEST_ASSERT_TRUE(power_idle_face(NULL));
    expect_write_count(0);
}

TEST_CASE("a wake lights a dark screen on the next read, eats nothing and starts the idle clock over", "[power]")
{
    fresh();
    const int64_t t = go_dark(T0, 5);
    TEST_ASSERT_EQUAL(ESP_OK, power_wake());
    expect_passes(sample(false), t + SEC);
    expect_write_count(1);
    expect_write(0, LIT, true);
    power_info_t info = info_at(t + SEC);
    TEST_ASSERT_EQUAL(POWER_ACTIVE, info.state);
    expect_counters(&info, 0, 1, 1, 0);

    /* The wake was a touch for the idle clock: the screen stays lit for the whole timeout from it. */
    writes_clear();
    expect_goes_dark_at(t + SEC + 10 * MS, t + 6 * SEC);

    /* A wake that lands on the same read as a tap on the dark screen leaves the tap to wake it, eaten, with one write.
     * A wake on a lit screen writes nothing, and a tap after it passes. */
    writes_clear();
    expect_passes(sample(false), t + 7 * SEC);
    writes_clear();
    TEST_ASSERT_EQUAL(ESP_OK, power_wake());
    expect_eaten(sample(true), t + 8 * SEC);
    expect_eaten(sample(false), t + 8 * SEC + 10 * MS);
    expect_write_count(1);
    writes_clear();
    TEST_ASSERT_EQUAL(ESP_OK, power_wake());
    expect_passes(sample(false), t + 9 * SEC);
    expect_write_count(0);
    expect_passes(sample(true), t + 9 * SEC + 10 * MS);
    expect_passes(sample(false), t + 9 * SEC + 20 * MS);
}

TEST_CASE("a wake takes the idle face down without eating the next touch", "[power]")
{
    fresh();
    TEST_ASSERT_EQUAL(ESP_OK, power_set_photo_idle(true, 10, T0));
    expect_passes(sample(false), T0 + 10 * SEC);
    TEST_ASSERT_TRUE(power_idle_face(NULL));
    TEST_ASSERT_EQUAL(ESP_OK, power_wake());
    expect_passes(sample(false), T0 + 11 * SEC);
    TEST_ASSERT_FALSE(power_idle_face(NULL));
    expect_write_count(0);
    expect_passes(sample(true), T0 + 12 * SEC);
    expect_passes(sample(false), T0 + 12 * SEC + 10 * MS);
}
