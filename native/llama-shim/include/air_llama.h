#ifndef AIR_LLAMA_H
#define AIR_LLAMA_H
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
/* ABI v2 layouts are unchanged; build_info behavior identity is shim_version 4.
 * v1 entry points and layouts remain compatible. All strings are pointer + UTF-8 byte length, never NUL-terminated.
 * Borrowed inputs must remain alive for the call. Output buffers belong to
 * shim; release with air_buffer_free. Error outputs are reset on every fallible
 * call. Engine/model/prepared are thread-affine, cancel alone supports
 * concurrent set. Destroy in prepared -> model -> engine order. Callbacks must not unwind or block indefinitely. Text callbacks may wait
 * for a bounded output budget only with cancellation-interruptible waits and
 * no additional native locks held. Progress callbacks must return promptly. Only one engine per process is supported. No function logs
 * request content.
 */
typedef struct air_engine air_engine;
typedef struct air_model air_model;
typedef struct air_prepared air_prepared;
typedef struct air_cancel air_cancel;
typedef struct {
  const uint8_t *data;
  uint64_t len;
} air_string;
typedef struct {
  uint8_t *data;
  uint64_t len;
} air_buffer;
typedef struct {
  int32_t code;
  air_buffer message;
} air_error;
typedef struct {
  air_string role;
  air_string content;
} air_message;
typedef struct {
  uint32_t context_size;
  uint32_t threads;
  uint32_t batch_size;
} air_load_options;
typedef struct {
  uint32_t max_tokens;
  float temperature;
  float top_p;
  uint32_t seed;
} air_generate_options;
typedef struct {
  uint32_t prompt_tokens;
  uint32_t completion_tokens;
  int32_t finish_reason;
} air_usage;
/* Codes: 0 success, 1 invalid argument, 2 cancelled, 3 unsupported model,
 * 4 unsupported template, 5 context limit, 6 native failure, 7 wrong thread,
 * 8 consumer stopped. Finish: 0 stop, 1 length, 2 cancelled, 3 failed. */
typedef int32_t (*air_text_callback)(void *user, air_string text);
int32_t air_engine_create(air_engine **out, air_error *error);
void air_engine_destroy(air_engine *engine);
int32_t air_model_load(air_engine *engine, air_string path,
                       air_load_options options, const air_cancel *cancel,
                       air_model **out, air_error *error);
/* Additive OCR entries; existing ABI layouts remain unchanged. Projector and
 * image storage are borrowed only for the call. Images are PNG/JPEG, <=4 MiB,
 * <=8192 per dimension and <=16,777,216 pixels. image_after_text is 0 or 1.
 * OCR accepts exactly one user prompt and one image, with no chat history. */
int32_t air_model_load_with_projector(air_engine *engine, air_string path,
                       air_string projector_path, air_load_options options,
                       const air_cancel *cancel, air_model **out, air_error *error);
int32_t air_prepare_image(air_model *model, air_string prompt,
                    const uint8_t *image, uint64_t image_len,
                    uint32_t image_after_text, air_generate_options options,
                    const air_string *stops, uint64_t stop_count,
                    const air_cancel *cancel, air_prepared **out,
                    uint32_t *prompt_tokens, air_error *error);
void air_model_unload(air_model *model);
int32_t air_prepare(air_model *model, const air_message *messages,
                    uint64_t count, air_generate_options options,
                    const air_string *stops, uint64_t stop_count,
                    const air_cancel *cancel, air_prepared **out,
                    uint32_t *prompt_tokens, air_error *error);
void air_prepared_free(air_prepared *prepared);
/* Consumes prepared on every call, including failure. Callback text is valid
 * UTF-8,
 * <=4096 bytes, borrowed until callback returns. Nonzero callback result stops
 * generation. Returns exactly once with final usage; no terminal callback is
 * emitted. */
int32_t air_generate(air_prepared *prepared, const air_cancel *cancel,
                     air_text_callback callback, void *user, air_usage *usage,
                     air_error *error);
/* Additive ABI v2 observation. phase: 0 prefill entered (completed=0),
 * 1 a prefill batch has successfully completed, 2 decode entered (completed=total).
 * completed_prompt_tokens counts only successful prefill batches. These are
 * synchronous numeric observations, not a terminal event. The callback must not
 * unwind/block; nonzero stops with code 8. A null callback disables observation.
 * Consumes prepared exactly like air_generate, including on failure. */
typedef int32_t (*air_progress_callback)(void *user, uint32_t phase,
                                        uint32_t completed_prompt_tokens,
                                        uint32_t total_prompt_tokens);
int32_t air_generate_observed(air_prepared *prepared, const air_cancel *cancel,
                              air_text_callback callback, void *user,
                              air_progress_callback progress, void *progress_user,
                              air_usage *usage, air_error *error);
int32_t air_cancel_create(air_cancel **out, air_error *error);
void air_cancel_set(air_cancel *cancel);
void air_cancel_destroy(air_cancel *cancel);
void air_buffer_free(air_buffer buffer);
int32_t air_get_build_info(air_buffer *out, air_error *error);
/* Borrow-free copy of original GGUF chat template, for reproducible SHA-256. */
int32_t air_model_template(air_model *model, air_buffer *out, air_error *error);
#ifdef __cplusplus
}
#endif
#endif
