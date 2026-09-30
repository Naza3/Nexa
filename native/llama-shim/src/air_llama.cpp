#include "air_llama.h"
#include "chat.h"
#include "llama.h"
#include <algorithm>
#include <atomic>
#include <cstdlib>
#include <cstring>
#include <limits>
#include <memory>
#include <stdexcept>
#include <string>
#include <thread>
#include <vector>

#include "stream_buffer.h"
struct air_cancel {
  std::atomic<bool> value{false};
};
struct air_engine {
  std::thread::id owner{std::this_thread::get_id()};
  bool loaded = false;
};
struct air_model {
  air_engine *engine = nullptr;
  llama_model *model = nullptr;
  llama_context *context = nullptr;
  common_chat_templates_ptr templates;
  uint32_t batch = 0;
  uint32_t context_limit = 0;
  ~air_model() {
    templates.reset();
    if (context)
      llama_free(context);
    if (model)
      llama_model_free(model);
    if (engine)
      engine->loaded = false;
  }
};
struct air_prepared {
  air_model *model;
  std::vector<llama_token> tokens;
  air_generate_options options;
  std::vector<std::string> stops;
};
static std::atomic<bool> engine_active{false};
static bool cancelled(const air_cancel *c) {
  return c && c->value.load(std::memory_order_relaxed);
}
static void check_cancel(const air_cancel *c) {
  if (cancelled(c))
    throw failure(2, "cancelled");
}
static void owner(air_engine *e) {
  if (!e)
    throw failure(1, "null engine");
  if (e->owner != std::this_thread::get_id())
    throw failure(7, "wrong inference thread");
}
static void owner(air_model *m) {
  if (!m)
    throw failure(1, "null model");
  owner(m->engine);
}
static air_buffer buffer(const std::string &s) {
  if (s.empty())
    return {nullptr, 0};
  auto *p = static_cast<uint8_t *>(std::malloc(s.size()));
  if (!p)
    throw std::bad_alloc();
  std::memcpy(p, s.data(), s.size());
  return {p, s.size()};
}
static void error_set(air_error *e, int32_t code,
                      const char *message) noexcept {
  if (!e)
    return;
  e->code = code;
  try {
    e->message = buffer(message);
  } catch (...) {
    e->message = {nullptr, 0};
  }
}
template <class F> static int32_t guarded(air_error *e, F f) noexcept {
  if (e)
    *e = {0, {nullptr, 0}};
  try {
    f();
    return 0;
  } catch (const failure &x) {
    error_set(e, x.code, x.what());
    return x.code;
  } catch (...) {
    error_set(e, 6, "native operation failed");
    return 6;
  }
}
static std::string string(air_string s) {
  if (s.len > 1048576 || (!s.data && s.len))
    throw failure(1, "invalid string length");
  std::string result(s.len ? reinterpret_cast<const char *>(s.data) : "",
                     static_cast<size_t>(s.len));
  if (utf8_prefix(result) != result.size())
    throw failure(1, "incomplete UTF-8 input");
  return result;
}
static bool abort_callback(void *p) {
  return cancelled(static_cast<const air_cancel *>(p));
}
static bool progress_callback(float, void *p) {
  return !cancelled(static_cast<const air_cancel *>(p));
}
static void quiet_log(ggml_log_level, const char *, void *) {
} // Native messages can contain full local paths.

extern "C" int32_t air_engine_create(air_engine **out, air_error *error) {
  if (out)
    *out = nullptr;
  return guarded(error, [&] {
    if (!out)
      throw failure(1, "null output");
    auto e = std::make_unique<air_engine>();
    bool expected = false;
    if (!engine_active.compare_exchange_strong(expected, true))
      throw failure(1, "engine already exists");
    llama_log_set(quiet_log, nullptr);
    try {
      llama_backend_init();
    } catch (...) {
      engine_active = false;
      throw;
    }
    *out = e.release();
  });
}
extern "C" void air_engine_destroy(air_engine *e) {
  if (!e || e->owner != std::this_thread::get_id() || e->loaded)
    return;
  llama_backend_free();
  delete e;
  engine_active = false;
}
extern "C" int32_t air_model_load(air_engine *e, air_string path,
                                  air_load_options options,
                                  const air_cancel *cancel, air_model **out,
                                  air_error *error) {
  if (out)
    *out = nullptr;
  return guarded(error, [&] {
    owner(e);
    if (!out || e->loaded || options.context_size < 32 ||
        options.context_size > 131072 || options.threads < 1 ||
        options.threads > 256 || options.batch_size < 1 ||
        options.batch_size > 4096)
      throw failure(1, "invalid load options");
    check_cancel(cancel);
    auto p = string(path);
    if (p.empty() || p.find('\0') != std::string::npos)
      throw failure(1, "invalid model path");
    auto m = std::make_unique<air_model>();
    m->engine = e;
    auto mp = llama_model_default_params();
    mp.n_gpu_layers = 0;
    mp.progress_callback = progress_callback;
    mp.progress_callback_user_data = const_cast<air_cancel *>(cancel);
    m->model = llama_model_load_from_file(p.c_str(), mp);
    check_cancel(cancel);
    if (!m->model)
      throw failure(6, "model load failed");
    char arch[64]{};
    if (llama_model_meta_val_str(m->model, "general.architecture", arch,
                                 sizeof(arch)) < 0 ||
        std::string(arch) != "qwen3")
      throw failure(3, "only Qwen3 is currently implemented; acceptance "
                       "remains model-specific");
    const char *tmpl = llama_model_chat_template(m->model, nullptr);
    if (!tmpl || !*tmpl)
      throw failure(4, "missing chat template");
    try {
      m->templates = common_chat_templates_init(m->model, "");
      if (!common_chat_templates_support_enable_thinking(m->templates.get()))
        throw failure(4, "template cannot disable thinking");
    } catch (const failure &) {
      throw;
    } catch (...) {
      throw failure(4, "unsupported chat template");
    }
    auto cp = llama_context_default_params();
    cp.n_ctx = options.context_size;
    cp.n_batch = std::min(options.batch_size, options.context_size);
    cp.n_ubatch = cp.n_batch;
    cp.n_threads = options.threads;
    cp.n_threads_batch = options.threads;
    cp.abort_callback = abort_callback;
    cp.abort_callback_data = const_cast<air_cancel *>(cancel);
    m->context = llama_init_from_model(m->model, cp);
    check_cancel(cancel);
    if (!m->context)
      throw failure(6, "context creation failed");
    llama_set_abort_callback(m->context, nullptr, nullptr);
    m->batch = cp.n_batch;
    // Upstream rounds KV capacity to 256; preserve the caller's logical budget.
    m->context_limit = std::min(options.context_size, llama_n_ctx(m->context));
    e->loaded = true;
    *out = m.release();
  });
}
extern "C" void air_model_unload(air_model *m) {
  if (m && m->engine->owner == std::this_thread::get_id())
    delete m;
}
extern "C" int32_t air_prepare(air_model *m, const air_message *messages,
                               uint64_t count, air_generate_options options,
                               const air_string *stops, uint64_t stop_count,
                               const air_cancel *cancel, air_prepared **out,
                               uint32_t *prompt_tokens, air_error *error) {
  if (out)
    *out = nullptr;
  if (prompt_tokens)
    *prompt_tokens = 0;
  return guarded(error, [&] {
    owner(m);
    if (!out || !prompt_tokens || !messages || count < 1 || count > 128 ||
        stop_count > 4 || (stop_count && !stops) || options.max_tokens < 1 ||
        options.max_tokens > 4096 ||
        !(options.temperature >= 0 && options.temperature <= 2) ||
        !(options.top_p > 0 && options.top_p <= 1))
      throw failure(1, "invalid generation options");
    check_cancel(cancel);
    common_chat_templates_inputs input;
    input.enable_thinking = false;
    input.use_jinja = true;
    input.add_generation_prompt = true;
    input.chat_template_kwargs["enable_thinking"] = "false";
    size_t bytes = 0;
    std::string expected = "user";
    for (uint64_t i = 0; i < count; i++) {
      common_chat_msg msg;
      msg.role = string(messages[i].role);
      msg.content = string(messages[i].content);
      bytes += msg.content.size();
      if (bytes > 1048576)
        throw failure(1, "messages exceed byte limit");
      if (i == 0 && msg.role == "system") {
      } else {
        if (msg.role != expected)
          throw failure(1, "invalid message order");
        expected = expected == "user" ? "assistant" : "user";
      }
      input.messages.push_back(std::move(msg));
    }
    if (input.messages.back().role != "user")
      throw failure(1, "last message must be user");
    auto p = std::make_unique<air_prepared>();
    p->model = m;
    p->options = options;
    for (uint64_t i = 0; i < stop_count; i++) {
      auto s = string(stops[i]);
      if (s.empty() || s.size() > 128)
        throw failure(1, "invalid stop");
      p->stops.push_back(s);
    }
    std::string prompt;
    try {
      auto applied = common_chat_templates_apply(m->templates.get(), input);
      prompt = applied.prompt;
    } catch (...) {
      throw failure(4, "template application failed");
    }
    check_cancel(cancel);
    if (prompt.size() > 4194304)
      throw failure(1, "formatted prompt too large");
    auto vocab = llama_model_get_vocab(m->model);
    int32_t n = llama_tokenize(vocab, prompt.data(),
                               static_cast<int32_t>(prompt.size()), nullptr, 0,
                               true, true);
    if (n >= 0)
      throw failure(6, "empty prompt tokenization");
    p->tokens.resize(static_cast<size_t>(-n));
    n = llama_tokenize(vocab, prompt.data(),
                       static_cast<int32_t>(prompt.size()), p->tokens.data(),
                       static_cast<int32_t>(p->tokens.size()), true, true);
    if (n <= 0)
      throw failure(6, "tokenization failed");
    p->tokens.resize(n);
    if (static_cast<uint64_t>(n) + options.max_tokens > m->context_limit)
      throw failure(5, "context_length_exceeded");
    check_cancel(cancel);
    *prompt_tokens = n;
    *out = p.release();
  });
}
extern "C" void air_prepared_free(air_prepared *p) {
  if (p && p->model->engine->owner == std::this_thread::get_id())
    delete p;
}

extern "C" int32_t air_generate(air_prepared *raw, const air_cancel *cancel,
                                air_text_callback callback, void *user,
                                air_usage *usage, air_error *error) {
  std::unique_ptr<air_prepared> p(raw);
  if (usage)
    *usage = {0, 0, 3};
  int32_t status = guarded(error, [&] {
    if (!p || !callback || !usage)
      throw failure(1, "invalid generation arguments");
    owner(p->model);
    auto ctx = p->model->context;
    auto vocab = llama_model_get_vocab(p->model->model);
    usage->prompt_tokens = static_cast<uint32_t>(p->tokens.size());
    struct cleanup {
      llama_context *ctx;
      ~cleanup() {
        llama_set_abort_callback(ctx, nullptr, nullptr);
        llama_memory_clear(llama_get_memory(ctx), true);
      }
    } clean{ctx};
    llama_memory_clear(llama_get_memory(ctx), true);
    llama_set_abort_callback(ctx, abort_callback,
                             const_cast<air_cancel *>(cancel));
    check_cancel(cancel);
    std::unique_ptr<llama_sampler, decltype(&llama_sampler_free)> sampler(
        llama_sampler_chain_init(llama_sampler_chain_default_params()),
        llama_sampler_free);
    if (!sampler)
      throw failure(6, "sampler allocation failed");
    if (p->options.temperature == 0)
      llama_sampler_chain_add(sampler.get(), llama_sampler_init_greedy());
    else {
      llama_sampler_chain_add(sampler.get(),
                              llama_sampler_init_top_p(p->options.top_p, 1));
      llama_sampler_chain_add(sampler.get(),
                              llama_sampler_init_temp(p->options.temperature));
      llama_sampler_chain_add(sampler.get(),
                              llama_sampler_init_dist(p->options.seed));
    }
    for (size_t i = 0; i < p->tokens.size(); i += p->model->batch) {
      check_cancel(cancel);
      auto batch = llama_batch_get_one(
          p->tokens.data() + i, static_cast<int32_t>(std::min<size_t>(
                                    p->model->batch, p->tokens.size() - i)));
      int r = llama_decode(ctx, batch);
      check_cancel(cancel);
      if (r)
        throw failure(6, "prefill failed");
    }
    air_stream_buffer stream(p->stops);
    auto emit = [&](bool final) {
      stream.flush(final, [&](const std::string &chunk) {
        check_cancel(cancel);
        air_string text{reinterpret_cast<const uint8_t *>(chunk.data()),
                        chunk.size()};
        if (callback(user, text) != 0)
          throw failure(8, "consumer stopped");
      });
    };
    usage->finish_reason = 1;
    for (uint32_t i = 0; i < p->options.max_tokens; i++) {
      check_cancel(cancel);
      auto token = llama_sampler_sample(sampler.get(), ctx, -1);
      usage->completion_tokens++;
      if (llama_vocab_is_eog(vocab, token)) {
        usage->finish_reason = 0;
        break;
      }
      char small[256];
      int32_t n =
          llama_token_to_piece(vocab, token, small, sizeof(small), 0, false);
      std::string piece;
      if (n < 0) {
        piece.resize(-n);
        n = llama_token_to_piece(vocab, token, piece.data(),
                                 static_cast<int32_t>(piece.size()), 0, false);
        if (n < 0)
          throw failure(6, "token decoding failed");
        piece.resize(n);
      } else
        piece.assign(small, n);
      stream.push(piece);
      emit(false);
      if (stream.stopped()) {
        usage->finish_reason = 0;
        break;
      }
      if (i + 1 < p->options.max_tokens) {
        int r = llama_decode(ctx, llama_batch_get_one(&token, 1));
        check_cancel(cancel);
        if (r)
          throw failure(6, "decode failed");
      }
    }
    emit(true);
    check_cancel(cancel);
  });
  if (usage && status)
    usage->finish_reason = (status == 2 || status == 8) ? 2 : 3;
  return status;
}
extern "C" int32_t air_cancel_create(air_cancel **out, air_error *error) {
  if (out)
    *out = nullptr;
  return guarded(error, [&] {
    if (!out)
      throw failure(1, "null output");
    *out = new air_cancel();
  });
}
extern "C" void air_cancel_set(air_cancel *c) {
  if (c)
    c->value.store(true, std::memory_order_relaxed);
}
extern "C" void air_cancel_destroy(air_cancel *c) { delete c; }
extern "C" void air_buffer_free(air_buffer b) { std::free(b.data); }
extern "C" int32_t air_get_build_info(air_buffer *out, air_error *error) {
  if (out)
    *out = {nullptr, 0};
  return guarded(error, [&] {
    if (!out)
      throw failure(1, "null output");
    *out = buffer("{\"shim_version\":1,\"backend\":\"cpu\",\"llama_commit\":"
                  "\"" AIR_LLAMA_COMMIT "\"}");
  });
}
extern "C" int32_t air_model_template(air_model *m, air_buffer *out,
                                      air_error *error) {
  if (out)
    *out = {nullptr, 0};
  return guarded(error, [&] {
    owner(m);
    if (!out)
      throw failure(1, "null output");
    *out = buffer(llama_model_chat_template(m->model, nullptr));
  });
}
