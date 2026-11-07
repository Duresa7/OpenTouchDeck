#include "td_device.h"
#include "unity.h"
#include <string.h>
TEST_CASE("Device names enforce UTF-8 byte and whitespace rules", "[td_device]")
{
    TEST_ASSERT_TRUE(td_device_name_valid("Open TouchDeck"));
    TEST_ASSERT_TRUE(td_device_name_valid("  Desk  "));
    TEST_ASSERT_FALSE(td_device_name_valid(""));
    TEST_ASSERT_FALSE(td_device_name_valid(" \xe2\x80\x83"));
    TEST_ASSERT_FALSE(td_device_name_valid("bad\nname"));
    TEST_ASSERT_FALSE(td_device_name_valid("\xc0\xaf"));
    TEST_ASSERT_FALSE(td_device_name_valid("\xed\xa0\x80"));
    TEST_ASSERT_FALSE(td_device_name_valid("\xf4\x90\x80\x80"));
    char name[50];
    memset(name, '"', 48); name[48] = 0;
    TEST_ASSERT_TRUE(td_device_name_valid(name));
    name[48] = 'x'; name[49] = 0;
    TEST_ASSERT_FALSE(td_device_name_valid(name));
}

TEST_CASE("Device names refuse format characters and Hangul fillers", "[td_device]")
{
    TEST_ASSERT_FALSE(td_device_name_valid("Desk\xe2\x80\xaePC"));      /* U+202E right-to-left override */
    TEST_ASSERT_FALSE(td_device_name_valid("\xef\xbb\xbf" "Desk"));        /* U+FEFF first */
    TEST_ASSERT_FALSE(td_device_name_valid("a\xe2\x80\x8d" "b"));       /* U+200D */
    TEST_ASSERT_FALSE(td_device_name_valid("Desk\xf3\xa0\x80\x81"));    /* U+E0001 */
    TEST_ASSERT_FALSE(td_device_name_valid("\xe3\x85\xa4"));            /* U+3164 alone */
    TEST_ASSERT_FALSE(td_device_name_valid("\xef\xbe\xa0"));            /* U+FFA0 alone */
    TEST_ASSERT_FALSE(td_device_name_valid("a\xe1\x85\x9f" "b"));       /* U+115F */
    TEST_ASSERT_FALSE(td_device_name_valid("a\xe1\x85\xa0" "b"));       /* U+1160 */
    TEST_ASSERT_TRUE(td_device_name_valid("Desk\xe2\x81\xa5PC"));       /* U+2065 is unassigned */
    TEST_ASSERT_TRUE(td_device_name_valid("\xed\x95\x9c\xea\xb8\x80")); /* Hangul letters */
}
