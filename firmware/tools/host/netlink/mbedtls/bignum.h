/* The host build's stand-in for ESP-IDF's mbedtls/bignum.h wrapper, which exposes TF-PSA-Crypto's private bignum API. */
#pragma once

#define MBEDTLS_DECLARE_PRIVATE_IDENTIFIERS
#include "mbedtls/private/bignum.h"
