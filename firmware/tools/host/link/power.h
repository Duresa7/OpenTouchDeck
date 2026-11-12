#pragma once
#include <stdbool.h>
#include <stdint.h>

/* The one power function the link's transfer wiring (td_link_transfer.c) calls. */
bool power_idle_face(uint32_t *entry);
