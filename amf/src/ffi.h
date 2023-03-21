#ifndef AMF_FFI_H
#define AMF_FFI_H

#include "../../common/src/callback.h"

void* amf_new_encoder(int32_t device, int32_t format, int32_t codecID, int32_t width, int32_t height);

int amf_encode(void *e, uint8_t *data[MAX_DATA_NUM], int32_t linesize[MAX_DATA_NUM], EncodeCallback callback, void* obj);

int amf_destroy_encoder(void *enc);

void* amf_new_decoder(int32_t device, int32_t format, int32_t codecID, int32_t iGpu);

int amf_decode(void *decoder, uint8_t *data, int32_t length, DecodeCallback callback, void *obj);

int amf_destroy_decoder(void *decoder);

#endif // AMF_FFI_H