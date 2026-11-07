/* The host build's stand-in for ESP-IDF's mbedtls/ecp.h wrapper, which exposes TF-PSA-Crypto's private ECP API. */
#pragma once

#define MBEDTLS_DECLARE_PRIVATE_IDENTIFIERS
#include "mbedtls/private/ecp.h"
