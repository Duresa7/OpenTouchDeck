#pragma once

#include "freertos/FreeRTOS.h"

typedef struct host_stream *StreamBufferHandle_t;
typedef struct {
    int unused;
} StaticStreamBuffer_t;

StreamBufferHandle_t xStreamBufferCreate(size_t size, size_t trigger);
StreamBufferHandle_t xStreamBufferCreateStatic(size_t size, size_t trigger, uint8_t *storage,
                                               StaticStreamBuffer_t *buffer);
size_t xStreamBufferSend(StreamBufferHandle_t stream, const void *data, size_t len, TickType_t wait);
size_t xStreamBufferReceive(StreamBufferHandle_t stream, void *data, size_t len, TickType_t wait);
size_t xStreamBufferSpacesAvailable(StreamBufferHandle_t stream);
BaseType_t xStreamBufferIsEmpty(StreamBufferHandle_t stream);
void vStreamBufferDelete(StreamBufferHandle_t stream);
