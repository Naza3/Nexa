#ifndef NEXA_MNN_V1_H
#define NEXA_MNN_V1_H
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
#define NEXA_MNN_ABI_VERSION 1u
#define NEXA_MNN_MAX_INPUT_BYTES 1048576u
/* All pointers are borrowed for the call. Handles remain caller-owned until a
 * successful destroy. Model/prepared calls, including destruction, are confined
 * to their creation thread. Only cancel_request may run concurrently. Callers
 * must keep the cancellation object alive through every concurrent operation.
 * Callbacks must not throw, block indefinitely, or reenter model operations.
 * Freed handles are invalid; dangling-pointer detection is not promised. */
typedef struct nexa_mnn_v1_model nexa_mnn_v1_model;
typedef struct nexa_mnn_v1_prepared nexa_mnn_v1_prepared;
typedef struct nexa_mnn_v1_cancel nexa_mnn_v1_cancel;
typedef struct { const uint8_t *data; uint64_t len; } nexa_mnn_v1_bytes;
enum { NEXA_MNN_OK=0, NEXA_MNN_INVALID=1, NEXA_MNN_ABI=2,
 NEXA_MNN_WRONG_THREAD=3, NEXA_MNN_BUSY=4, NEXA_MNN_CANCELLED=5,
 NEXA_MNN_NATIVE=6, NEXA_MNN_BUDGET=7, NEXA_MNN_CONSUMED=8,
 NEXA_MNN_CALLBACK=9, NEXA_MNN_IDENTITY=10 };
enum { NEXA_MNN_STOP_EOS=1, NEXA_MNN_STOP_LENGTH=2,
 NEXA_MNN_STOP_USER=3, NEXA_MNN_STOP_CANCEL=4, NEXA_MNN_STOP_ERROR=5 };
enum { NEXA_MNN_ROLE_SYSTEM=1, NEXA_MNN_ROLE_USER=2, NEXA_MNN_ROLE_ASSISTANT=3 };
enum { NEXA_MNN_PHASE_LOAD=1, NEXA_MNN_PHASE_TEMPLATE=2,
 NEXA_MNN_PHASE_TOKENIZE=3, NEXA_MNN_PHASE_PREFILL=4, NEXA_MNN_PHASE_DECODE=5 };
/* Numeric, owner-thread progress only. The callback can signal a control thread
 * to cancel. Count is phase-specific (load checkpoint / completed input tokens /
 * accepted output tokens), not a wall-clock latency promise. */
typedef void (*nexa_mnn_v1_progress)(void *user, uint32_t phase, uint64_t count);
/* 0 accepts a borrowed UTF-8 chunk (1..4096 bytes); 1 cancels; 2 fails.
 * No other return values are accepted. No text is retained after this call. */
typedef int32_t (*nexa_mnn_v1_text)(void *user, nexa_mnn_v1_bytes text);
typedef struct { uint32_t struct_size, abi_version; int32_t code;
 uint32_t message_len; uint8_t message[512]; } nexa_mnn_v1_error;
typedef struct { uint32_t struct_size, abi_version; uint8_t upstream_commit[40];
 uint8_t patch_sha256[64]; uint8_t policy_sha256[64]; uint32_t silent_logs, reserved;
} nexa_mnn_v1_build;
typedef struct { uint32_t struct_size, abi_version;
 nexa_mnn_v1_bytes runtime_config_path, artifact_sha256, policy_sha256,
 expected_upstream_commit, expected_patch_sha256;
 uint32_t logical_context, threads, prefill_chunk, reserved;
 nexa_mnn_v1_progress progress; void *progress_user;
} nexa_mnn_v1_load_options;
typedef struct { uint32_t struct_size, abi_version; int32_t role; uint32_t reserved;
 nexa_mnn_v1_bytes content; } nexa_mnn_v1_message;
typedef struct { uint32_t struct_size, abi_version;
 const nexa_mnn_v1_message *messages; uint64_t message_count;
 const nexa_mnn_v1_bytes *stops; uint64_t stop_count;
 uint32_t max_tokens; float temperature, top_p; uint32_t seed;
 /* UINT32_MAX resolves fresh entropy, all other seeds including zero are exact. */
 nexa_mnn_v1_progress progress; void *progress_user; uint64_t reserved;
} nexa_mnn_v1_request;
typedef struct { uint32_t struct_size, abi_version; uint64_t prompt_tokens;
 uint32_t resolved_seed, reserved; } nexa_mnn_v1_prepared_info;
typedef struct { uint32_t struct_size, abi_version; uint64_t prompt_tokens,
 completion_tokens; int32_t stop_reason; uint32_t resolved_seed;
} nexa_mnn_v1_result;
/* Output structs must have exact sizeof and ABI=1 initialized before calling.
 * On failure load/prepare clear *out. Errors contain fixed private text only.
 * Prepared is consumed by every generate attempt that starts on its owner,
 * including cancellation; destroy it before another prepare or model destroy.
 * Invalid arguments, ABI, wrong-thread and busy errors do not consume Prepared.
 * Error may be null; malformed error headers are never written. Input message
 * and stop bytes are copied during prepare; progress/user are never retained.
 * Every progress callback is borrowed only during its corresponding call. */
int32_t nexa_mnn_v1_build_info(nexa_mnn_v1_build *out);
int32_t nexa_mnn_v1_cancel_create(nexa_mnn_v1_cancel **out);
void nexa_mnn_v1_cancel_request(nexa_mnn_v1_cancel *flag);
void nexa_mnn_v1_cancel_destroy(nexa_mnn_v1_cancel *flag);
int32_t nexa_mnn_v1_load(const nexa_mnn_v1_load_options *, nexa_mnn_v1_cancel *, nexa_mnn_v1_model **out, nexa_mnn_v1_error *);
int32_t nexa_mnn_v1_prepare(nexa_mnn_v1_model *, const nexa_mnn_v1_request *, nexa_mnn_v1_cancel *, nexa_mnn_v1_prepared **out, nexa_mnn_v1_prepared_info *, nexa_mnn_v1_error *);
int32_t nexa_mnn_v1_generate(nexa_mnn_v1_prepared *, nexa_mnn_v1_cancel *, nexa_mnn_v1_text, void *, nexa_mnn_v1_progress, void *, nexa_mnn_v1_result *, nexa_mnn_v1_error *);
int32_t nexa_mnn_v1_prepared_destroy(nexa_mnn_v1_prepared *, nexa_mnn_v1_error *);
int32_t nexa_mnn_v1_model_destroy(nexa_mnn_v1_model *, nexa_mnn_v1_error *);
#ifdef __cplusplus
}
#endif
#endif
