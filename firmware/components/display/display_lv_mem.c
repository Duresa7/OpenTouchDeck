#include "esp_heap_caps.h"
#include "lvgl.h"

/*
 * LVGL's allocator when the build selects CONFIG_LV_USE_CUSTOM_MALLOC, which only the radio build does
 * (firmware/radio/radio.defaults). The normal build uses the C library's malloc.
 *
 * Every LVGL object, style and string goes to PSRAM first, and to internal RAM only when PSRAM is full. In the radio
 * build malloc keeps blocks under 256 bytes internal, which is nearly every LVGL object, and the Wi-Fi station needs
 * that internal RAM at boot (#313). Only the LVGL task, and app_main under the adapter lock, touch LVGL's memory, and
 * no task runs while flash is written, so it never has to be readable with the cache off.
 */

#define LV_MEM_PREFERRED (MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT)
#define LV_MEM_FALLBACK (MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT)

void *lv_malloc_core(size_t size);
void *lv_realloc_core(void *p, size_t new_size);
void lv_free_core(void *p);
void lv_mem_monitor_core(lv_mem_monitor_t *mon_p);
lv_result_t lv_mem_test_core(void);

void lv_mem_init(void)
{
}

void lv_mem_deinit(void)
{
}

lv_mem_pool_t lv_mem_add_pool(void *mem, size_t bytes)
{
    LV_UNUSED(mem);
    LV_UNUSED(bytes);
    return NULL;
}

void lv_mem_remove_pool(lv_mem_pool_t pool)
{
    LV_UNUSED(pool);
}

void *lv_malloc_core(size_t size)
{
    return heap_caps_malloc_prefer(size, 2, LV_MEM_PREFERRED, LV_MEM_FALLBACK);
}

void *lv_realloc_core(void *p, size_t new_size)
{
    return heap_caps_realloc_prefer(p, new_size, 2, LV_MEM_PREFERRED, LV_MEM_FALLBACK);
}

void lv_free_core(void *p)
{
    heap_caps_free(p);
}

void lv_mem_monitor_core(lv_mem_monitor_t *mon_p)
{
    LV_UNUSED(mon_p);
}

lv_result_t lv_mem_test_core(void)
{
    return LV_RESULT_OK;
}
