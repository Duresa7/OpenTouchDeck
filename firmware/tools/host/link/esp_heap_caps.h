#pragma once
#include <stddef.h>

/* The host heap has no PSRAM. The link test counts what the transfer allocates and frees. */
#define MALLOC_CAP_SPIRAM 1
#define MALLOC_CAP_8BIT 2

void *heap_caps_malloc(size_t size, unsigned caps);
void heap_caps_free(void *ptr);
