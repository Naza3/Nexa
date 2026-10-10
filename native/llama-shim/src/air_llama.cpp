#include "air_llama.h"
#include "chat.h"
#include "llama.h"
#include "mtmd.h"
#include "mtmd-helper.h"
#include "stb/stb_image.h"
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
#include "text_template.h"
#include "ocr_template.h"
#include "tool_template.h"
#include "tool_output.h"
#include <set>
#include <map>
#include "gguf.h"
#include "log.h"
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
  std::unique_ptr<air_text_template> templates;
  std::unique_ptr<air_ocr_template> ocr_template;
  mtmd_context *projector = nullptr;
  const air_cancel *vision_cancel = nullptr;
  uint32_t batch = 0;
  uint32_t context_limit = 0;
  ~air_model() {
    templates.reset();
    ocr_template.reset();
    if (projector) mtmd_free(projector);
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
  std::unique_ptr<mtmd_input_chunks, decltype(&mtmd_input_chunks_free)> chunks{nullptr, mtmd_input_chunks_free};
  uint32_t prompt_tokens = 0;
  air_generate_options options;
  std::vector<std::string> stops;
  std::unique_ptr<air_tool_parser> tool_parser;
  std::set<llama_token> preserved_tokens;
  uint32_t tool_choice = 0;
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
static thread_local bool unsupported_architecture = false;
static void quiet_log(ggml_log_level, const char *message, void *) {
  // Classify only known static engine errors; never retain native text/paths.
  if (message && (std::strstr(message, "unknown model architecture:") ||
                  std::strstr(message, "unsupported model architecture:")))
    unsupported_architecture = true;
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
    // Configure global diagnostics only after acquiring the single-engine
    // owner and before any native/template activity. Dependency defaults or
    // earlier direct-library users must not enable prompt/template logging.
    llama_log_set(quiet_log, nullptr);
    mtmd_helper_log_set(quiet_log, nullptr);
    common_log_set_verbosity_thold(-1);
    jinja::enable_debug(false);
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
static int32_t model_load(air_engine *e, air_string path, air_string projector_path,
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
    // Defense in depth for direct adapter users too: never let llama open
    // sibling shards outside the protected single-file closure.
    gguf_init_params gp{true, nullptr};
    std::unique_ptr<gguf_context, decltype(&gguf_free)> metadata(
        gguf_init_from_file(p.c_str(), gp), gguf_free);
    if (!metadata)
      throw failure(3, "invalid GGUF model metadata");
    for (const char *key : {"split.count", "split.no"}) {
      auto index = gguf_find_key(metadata.get(), key);
      if (index < 0) continue;
      auto type = gguf_get_kv_type(metadata.get(), index);
      uint32_t value;
      if (type == GGUF_TYPE_UINT16) value = gguf_get_val_u16(metadata.get(), index);
      else if (type == GGUF_TYPE_UINT32) value = gguf_get_val_u32(metadata.get(), index);
      else throw failure(3, "invalid GGUF split metadata");
      if ((std::strcmp(key, "split.count") == 0 && value > 1) ||
          (std::strcmp(key, "split.no") == 0 && value != 0))
        throw failure(3, "multi-file GGUF is not supported");
    }
    metadata.reset();
    check_cancel(cancel);
    unsupported_architecture = false;
    m->model = llama_model_load_from_file(p.c_str(), mp);
    check_cancel(cancel);
    if (!m->model) {
      if (unsupported_architecture)
        throw failure(3, "architecture is unsupported by the locked engine");
      throw failure(6, "model load failed; verify format and available resources");
    }
    if (llama_model_has_encoder(m->model) || !llama_model_has_decoder(m->model) ||
        llama_model_is_diffusion(m->model))
      throw failure(3, "model requires an unsupported execution method");
    if (options.context_size > static_cast<uint32_t>(llama_model_n_ctx_train(m->model)))
      throw failure(5, "requested context exceeds model training context");
    const char *tmpl = llama_model_chat_template(m->model, nullptr);
    if (!tmpl || !*tmpl)
      throw failure(4, "missing embedded chat template; no fallback is provided");
    try {
      const auto *vocab = llama_model_get_vocab(m->model);
      const auto bos_id = llama_vocab_bos(vocab);
      const auto eos_id = llama_vocab_eos(vocab);
      auto bos = bos_id == LLAMA_TOKEN_NULL ? std::string() : common_token_to_piece(vocab, bos_id, true);
      auto eos = eos_id == LLAMA_TOKEN_NULL ? std::string() : common_token_to_piece(vocab, eos_id, true);
      if (projector_path.len) {
        m->ocr_template = std::make_unique<air_ocr_template>(tmpl, bos, eos);
      } else m->templates = std::make_unique<air_text_template>(
          tmpl, bos, eos, llama_vocab_get_add_bos(vocab), llama_vocab_get_add_eos(vocab),
          [vocab, eos_id](const std::string &suffix) {
            if (suffix.empty())
              return llama_vocab_get_add_eos(vocab) && eos_id != LLAMA_TOKEN_NULL && llama_vocab_is_eog(vocab, eos_id);
            auto tokens = common_tokenize(vocab, suffix, false, true);
            if (tokens.empty() || !llama_vocab_is_eog(vocab, tokens.front())) return false;
            std::string trailing;
            for (size_t i = 1; i < tokens.size(); ++i)
              trailing += common_token_to_piece(vocab, tokens[i], true);
            return trailing.find_first_not_of(" \t\r\n") == std::string::npos;
          });
    } catch (const failure &) { throw; }
      catch (...) { throw failure(4, "embedded template is unsupported by the text adapter"); }
    check_cancel(cancel);
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
    if (!llama_get_causal_attn(m->context))
      throw failure(3, "non-causal models are unsupported by the text decoder");
    llama_set_abort_callback(m->context, nullptr, nullptr);
    m->batch = cp.n_batch;
    // Upstream rounds KV capacity to 256; preserve the caller's logical budget.
    m->context_limit = std::min(options.context_size, llama_n_ctx(m->context));
    if (projector_path.len) {
      auto pp = string(projector_path);
      if (pp.empty() || pp.find('\0') != std::string::npos)
        throw failure(1, "invalid projector path");
      auto vp = mtmd_context_params_default();
      vp.use_gpu = false;
      vp.print_timings = false;
      vp.warmup = false;
      vp.n_threads = options.threads;
      // mtmd may retain its parameters. Keep user_data model-owned instead of
      // storing a borrowed load flag that can be dropped after this call.
      m->vision_cancel = cancel;
      vp.progress_callback = [](float, void *data) {
        return !cancelled(static_cast<air_model *>(data)->vision_cancel);
      };
      vp.progress_callback_user_data = m.get();
      vp.cb_eval = [](ggml_tensor *, bool ask, void *data) {
        auto *model = static_cast<air_model *>(data);
        return ask || !cancelled(model->vision_cancel);
      };
      vp.cb_eval_user_data = m.get();
      m->projector = mtmd_init_from_file(pp.c_str(), m->model, vp);
      check_cancel(cancel);
      m->vision_cancel = nullptr;
      if (!m->projector || !mtmd_support_vision(m->projector) || mtmd_support_audio(m->projector))
        throw failure(3, "projector does not support the image-only execution contract");
    }
    e->loaded = true;
    *out = m.release();
  });
}
extern "C" int32_t air_model_load(air_engine *e, air_string path,
    air_load_options options, const air_cancel *cancel, air_model **out, air_error *error) {
  return model_load(e, path, {nullptr, 0}, options, cancel, out, error);
}
extern "C" int32_t air_model_load_with_projector(air_engine *e, air_string path,
    air_string projector_path, air_load_options options, const air_cancel *cancel,
    air_model **out, air_error *error) {
  if (!projector_path.len) {
    if (out) *out = nullptr;
    return guarded(error, [] { throw failure(1, "projector path is required"); });
  }
  return model_load(e, path, projector_path, options, cancel, out, error);
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
    if (!m->templates) throw failure(4, "this OCR model requires one image and a prompt");
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
      prompt = m->templates->render(input.messages);
    } catch (const failure &) { throw;
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
    p->prompt_tokens = n;
    *prompt_tokens = n;
    *out = p.release();
  });
}
extern "C" int32_t air_prepare_chat_v3(air_model *m,
    const air_message_v3 *messages, uint64_t count,
    const air_tool_v3 *tools, uint64_t tool_count,
    uint32_t choice, air_string choice_name, uint32_t parallel_tool_calls,
    air_generate_options options, const air_cancel *cancel,
    air_prepared **out, uint32_t *prompt_tokens, air_error *error) {
  if (out) *out = nullptr;
  if (prompt_tokens) *prompt_tokens = 0;
  return guarded(error, [&] {
    owner(m);
    if (!out || !prompt_tokens || !messages || count < 1 || count > 128 ||
        tool_count > 64 || (tool_count && !tools) || choice > 3 || parallel_tool_calls > 1 ||
        options.max_tokens < 1 || options.max_tokens > 4096 ||
        !(options.temperature >= 0 && options.temperature <= 2) ||
        !(options.top_p > 0 && options.top_p <= 1) || m->projector)
      throw failure(1, "invalid tool generation options");
    check_cancel(cancel);
    const auto selected_name = string(choice_name);
    if ((choice == 3) != !selected_name.empty()) throw failure(1, "invalid named tool choice");
    common_chat_templates_inputs input;
    input.enable_thinking = false;
    input.reasoning_format = COMMON_REASONING_FORMAT_AUTO;
    input.chat_template_kwargs["enable_thinking"] = "false";
    input.parallel_tool_calls = parallel_tool_calls != 0;
    input.tool_choice = choice == 0 ? COMMON_CHAT_TOOL_CHOICE_NONE :
                        choice == 1 ? COMMON_CHAT_TOOL_CHOICE_AUTO : COMMON_CHAT_TOOL_CHOICE_REQUIRED;
    size_t bytes = 0;
    auto bounded = [&](air_string value) {
      auto result = string(value);
      if (result.size() > 1048576 - bytes) throw failure(1, "tool request exceeds byte limit");
      bytes += result.size();
      return result;
    };
    std::map<std::string, std::string> history_names;
    for (uint64_t i = 0; i < count; ++i) {
      const auto &source = messages[i];
      if (source.has_content > 1 || source.call_count > 16 || (source.call_count && !source.calls))
        throw failure(1, "invalid tool history layout");
      common_chat_msg message;
      message.role = bounded(source.role);
      message.content = bounded(source.content);
      message.tool_call_id = bounded(source.tool_call_id);
      if (message.role != "system" && message.role != "user" &&
          message.role != "assistant" && message.role != "tool")
        throw failure(1, "invalid chat role");
      if (!source.has_content && (!message.content.empty() || message.role != "assistant" || !source.call_count))
        throw failure(1, "invalid nullable content");
      for (uint64_t j = 0; j < source.call_count; ++j) {
        common_chat_tool_call call;
        call.id = bounded(source.calls[j].id);
        call.name = bounded(source.calls[j].name);
        call.arguments = bounded(source.calls[j].arguments);
        if (call.id.empty() || call.id.size() > 128 || call.name.empty() || call.name.size() > 64 ||
            call.arguments.size() > 16384 || !common_json::parse_no_throw(call.arguments).is_object() ||
            !history_names.emplace(call.id, call.name).second)
          throw failure(1, "invalid historical tool call");
        message.tool_calls.push_back(std::move(call));
      }
      if (message.role == "tool") {
        const auto found = history_names.find(message.tool_call_id);
        if (found == history_names.end()) throw failure(1, "tool result has no preceding call");
        message.tool_name = found->second;
      }
      input.messages.push_back(std::move(message));
    }
    for (uint64_t i = 0; i < tool_count; ++i) {
      common_chat_tool tool;
      tool.name = bounded(tools[i].name);
      tool.description = bounded(tools[i].description);
      tool.parameters = bounded(tools[i].parameters_json);
      if (tool.name.empty() || tool.name.size() > 64 || tool.description.size() > 16384 ||
          tool.parameters.size() > 65536 || !common_json::parse_no_throw(tool.parameters).is_object())
        throw failure(1, "invalid tool definition");
      if (choice != 3 || tool.name == selected_name) input.tools.push_back(std::move(tool));
    }
    if (choice != 0 && input.tools.empty()) throw failure(1, "tool choice has no matching definition");
    const auto *vocab = llama_model_get_vocab(m->model);
    auto selected_source = llama_model_chat_template(m->model, "tool_use");
    if (!selected_source || !*selected_source) selected_source = llama_model_chat_template(m->model, nullptr);
    if (!selected_source || !*selected_source || std::strlen(selected_source) > 1048576 ||
        std::strcmp(selected_source, "chatml") == 0)
      throw failure(4, "missing embedded tool template; no fallback is provided");
    auto prepared = std::make_unique<air_prepared>();
    prepared->model = m;
    prepared->options = options;
    prepared->tool_choice = choice;
    common_chat_params applied;
    try {
    auto templates = common_chat_templates_init(m->model, selected_source);
    const auto caps = common_chat_templates_get_caps(templates.get());
    if (!caps.at("supports_tools") || !caps.at("supports_tool_calls"))
      throw failure(4, "embedded template does not support tool definitions and history");
    if (!caps.at("supports_system_role") && input.messages.front().role == "system")
      throw failure(4, "embedded template cannot preserve system role");
    applied = common_chat_templates_apply(templates.get(), input);
    input.tool_choice = COMMON_CHAT_TOOL_CHOICE_NONE;
    const auto text = common_chat_templates_apply(templates.get(), input);
    const auto bos_id = llama_vocab_bos(vocab), eos_id = llama_vocab_eos(vocab);
    common_chat_template template_info(selected_source,
        bos_id == LLAMA_TOKEN_NULL ? "" : common_token_to_piece(vocab, bos_id, true),
        eos_id == LLAMA_TOKEN_NULL ? "" : common_token_to_piece(vocab, eos_id, true));
    prepared->tool_parser = std::make_unique<air_tool_parser>(template_info, applied, text);
    } catch (const failure &) { throw; }
      catch (const std::bad_alloc &) { throw; }
      catch (...) { throw failure(4, "embedded tool template application failed"); }
    for (const auto &stop : applied.additional_stops) {
      if (stop.empty() || stop.size() > 1024 || prepared->stops.size() >= 32)
        throw failure(4, "unsupported template stop boundary");
      prepared->stops.push_back(stop);
    }
    for (const auto &token_text : applied.preserved_tokens) {
      if (token_text.size() > 1024) throw failure(4, "oversized template special token");
      auto tokens = common_tokenize(vocab, token_text, false, true);
      for (const auto token : tokens) {
        if (llama_vocab_get_attr(vocab, token) & LLAMA_TOKEN_ATTR_CONTROL)
          prepared->preserved_tokens.insert(token);
      }
    }
    const auto &prompt = applied.prompt;
    if (prompt.empty() || prompt.size() > 4194304) throw failure(1, "formatted prompt too large");
    int32_t n = llama_tokenize(vocab, prompt.data(), static_cast<int32_t>(prompt.size()), nullptr, 0, true, true);
    if (n >= 0) throw failure(6, "empty prompt tokenization");
    prepared->tokens.resize(static_cast<size_t>(-n));
    n = llama_tokenize(vocab, prompt.data(), static_cast<int32_t>(prompt.size()), prepared->tokens.data(),
                       static_cast<int32_t>(prepared->tokens.size()), true, true);
    if (n <= 0) throw failure(6, "tokenization failed");
    prepared->tokens.resize(n);
    if (static_cast<uint64_t>(n) + options.max_tokens > m->context_limit)
      throw failure(5, "context_length_exceeded");
    check_cancel(cancel);
    prepared->prompt_tokens = n;
    *prompt_tokens = n;
    *out = prepared.release();
  });
}
extern "C" int32_t air_prepare_image(air_model *m, air_string input,
    const uint8_t *image, uint64_t image_len, uint32_t image_after_text,
    air_generate_options options, const air_string *stops, uint64_t stop_count,
    const air_cancel *cancel, air_prepared **out, uint32_t *prompt_tokens,
    air_error *error) {
  if (out) *out = nullptr;
  if (prompt_tokens) *prompt_tokens = 0;
  return guarded(error, [&] {
    owner(m);
    if (!out || !prompt_tokens || !image || image_len < 3 || image_len > 4194304 ||
        image_after_text > 1 || stop_count > 4 || (stop_count && !stops) ||
        options.max_tokens < 1 || options.max_tokens > 4096 ||
        !(options.temperature >= 0 && options.temperature <= 2) ||
        !(options.top_p > 0 && options.top_p <= 1))
      throw failure(1, "invalid image generation arguments");
    check_cancel(cancel);
    if (!m->projector || !m->ocr_template)
      throw failure(3, "image input requires a compatible loaded projector");
    auto text = string(input);
    const std::string marker = mtmd_get_marker(m->projector);
    if (text.find_first_not_of(" \t\r\n") == std::string::npos ||
        text.find(marker) != std::string::npos)
      throw failure(1, "image prompt is empty or contains a reserved media marker");
    const uint8_t png[] = {137, 80, 78, 71, 13, 10, 26, 10};
    const bool is_png = image_len >= 24 && std::memcmp(image, png, sizeof(png)) == 0;
    const bool is_jpeg = image[0] == 255 && image[1] == 216 && image[2] == 255;
    int width = 0, height = 0, channels = 0;
    if ((!is_png && !is_jpeg) ||
        !stbi_info_from_memory(image, static_cast<int>(image_len), &width, &height, &channels) ||
        width < 1 || height < 1 || width > 8192 || height > 8192 ||
        static_cast<uint64_t>(width) * height > 16777216)
      throw failure(1, "invalid or oversized PNG/JPEG image");
    auto decoded = mtmd_helper_bitmap_init_from_buf(m->projector, image, image_len,
                                                   false, mtmd_helper_init_opt_default());
    std::unique_ptr<mtmd_bitmap, decltype(&mtmd_bitmap_free)> bitmap(decoded.bitmap, mtmd_bitmap_free);
    std::unique_ptr<mtmd_helper_video, decltype(&mtmd_helper_video_free)> video(decoded.video_ctx, mtmd_helper_video_free);
    check_cancel(cancel);
    if (!bitmap || video || mtmd_bitmap_is_audio(bitmap.get()))
      throw failure(1, "PNG/JPEG image decoding failed");
    auto p = std::make_unique<air_prepared>();
    p->model = m;
    p->options = options;
    for (uint64_t i = 0; i < stop_count; ++i) {
      auto stop = string(stops[i]);
      if (stop.empty() || stop.size() > 128) throw failure(1, "invalid stop");
      p->stops.push_back(std::move(stop));
    }
    auto prompt = m->ocr_template->render(image_after_text ? text + marker : marker + text);
    if (prompt.size() > 4194304) throw failure(1, "formatted prompt too large");
    p->chunks.reset(mtmd_input_chunks_init());
    if (!p->chunks) throw failure(6, "image chunk allocation failed");
    mtmd_input_text formatted{prompt.data(), prompt.size(), true, true};
    const mtmd_bitmap *bitmaps[] = {bitmap.get()};
    int result = mtmd_tokenize(m->projector, p->chunks.get(), &formatted, bitmaps, 1);
    check_cancel(cancel);
    if (result) throw failure(1, "image tokenization failed");
    const auto tokens = mtmd_helper_get_n_tokens(p->chunks.get());
    const auto positions = mtmd_helper_get_n_pos(p->chunks.get());
    if (!tokens || positions < 1) throw failure(6, "empty multimodal prompt");
    if (tokens + options.max_tokens > m->context_limit ||
        static_cast<uint64_t>(positions) + options.max_tokens > m->context_limit)
      throw failure(5, "context_length_exceeded");
    p->prompt_tokens = static_cast<uint32_t>(tokens);
    *prompt_tokens = p->prompt_tokens;
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
  return air_generate_observed(raw, cancel, callback, user, nullptr, nullptr,
                               usage, error);
}
static int32_t generate_impl(
    air_prepared *raw, const air_cancel *cancel, air_text_callback callback,
    air_chat_callback_v3 chat_callback, void *user,
    air_progress_callback progress, void *progress_user,
    air_usage *usage, air_error *error) {
  std::unique_ptr<air_prepared> p(raw);
  if (usage)
    *usage = {0, 0, 3};
  int32_t status = guarded(error, [&] {
    if (!p || !usage || (p->tool_parser ? !chat_callback : !callback) ||
        (chat_callback && !p->tool_parser))
      throw failure(1, "invalid generation arguments");
    owner(p->model);
    auto ctx = p->model->context;
    auto vocab = llama_model_get_vocab(p->model->model);
    usage->prompt_tokens = p->prompt_tokens;
    struct cleanup {
      llama_context *ctx;
      air_model *model;
      ~cleanup() {
        model->vision_cancel = nullptr;
        llama_set_abort_callback(ctx, nullptr, nullptr);
        llama_memory_clear(llama_get_memory(ctx), true);
      }
    } clean{ctx, p->model};
    p->model->vision_cancel = cancel;
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
    auto observe = [&](uint32_t phase, uint32_t completed) {
      if (progress && progress(progress_user, phase, completed,
                               usage->prompt_tokens) != 0)
        throw failure(8, "progress consumer stopped");
      check_cancel(cancel);
    };
    observe(0, 0);
    llama_pos next_position = 0;
    std::unique_ptr<llama_batch_ext, decltype(&llama_batch_ext_free)> multimodal_batch(
        p->chunks ? llama_batch_ext_init(ctx) : nullptr, llama_batch_ext_free);
    if (p->chunks && !multimodal_batch) throw failure(6, "batch allocation failed");
    uint32_t completed = 0;
    auto decode_text = [&](const llama_token *tokens, size_t count, bool final, bool prefill) {
      for (size_t i = 0; i < count; i += p->model->batch) {
        check_cancel(cancel);
        llama_batch_ext_clear(multimodal_batch.get());
        size_t end = std::min(count, i + p->model->batch);
        for (size_t j = i; j < end; ++j) {
          int32_t index = llama_batch_ext_add_token(multimodal_batch.get(), 0, tokens[j]);
          if (index < 0 || !llama_batch_ext_set_pos(multimodal_batch.get(), index, &next_position))
            throw failure(6, "multimodal text batch failed");
          ++next_position;
          if (final && j + 1 == count &&
              !llama_batch_ext_set_output_logits(multimodal_batch.get(), index, true))
            throw failure(6, "multimodal logits batch failed");
        }
        int r = llama_process(ctx, LLAMA_PROCESS_TYPE_DECODE, multimodal_batch.get());
        check_cancel(cancel);
        if (r) throw failure(6, "multimodal text prefill failed");
        if (prefill) {
          completed += static_cast<uint32_t>(end - i);
          observe(1, completed);
        }
      }
    };
    if (p->chunks) {
      const auto count = mtmd_input_chunks_size(p->chunks.get());
      for (size_t i = 0; i < count; ++i) {
        check_cancel(cancel);
        auto *chunk = mtmd_input_chunks_get(p->chunks.get(), i);
        auto type = mtmd_input_chunk_get_type(chunk);
        if (type == MTMD_INPUT_CHUNK_TYPE_TEXT) {
          size_t n = 0;
          auto *tokens = mtmd_input_chunk_get_tokens_text(chunk, &n);
          decode_text(tokens, n, i + 1 == count, true);
        } else if (type == MTMD_INPUT_CHUNK_TYPE_IMAGE) {
          int r = mtmd_encode_chunk(p->model->projector, chunk);
          // Upstream cb_eval stops graph work without necessarily returning an
          // error: never consume embeddings before checking our cancellation flag.
          check_cancel(cancel);
          if (r) throw failure(6, "image encoding failed");
          struct image_progress_state {
            air_progress_callback callback;
            void *user;
            uint32_t *completed;
            uint32_t total;
            const air_cancel *cancel;
          } state{progress, progress_user, &completed, usage->prompt_tokens, cancel};
          auto on_batch = [](const mtmd_helper_embd_batch *batch, void *data) -> int32_t {
            auto &state = *static_cast<image_progress_state *>(data);
            *state.completed += static_cast<uint32_t>(batch->n_tokens);
            if (state.callback && state.callback(state.user, 1, *state.completed, state.total))
              return 8;
            return cancelled(state.cancel) ? 2 : 0;
          };
          r = mtmd_helper_decode_image_chunk(p->model->projector, ctx, chunk,
              mtmd_get_output_embd(p->model->projector), next_position, 0,
              p->model->batch, &next_position, on_batch, &state);
          check_cancel(cancel);
          if (r == 8) throw failure(8, "progress consumer stopped");
          if (r) throw failure(6, "image prefill failed");
        } else throw failure(3, "unsupported multimodal chunk");
      }
    } else {
      for (size_t i = 0; i < p->tokens.size(); i += p->model->batch) {
        check_cancel(cancel);
        auto batch = llama_batch_get_one(
            p->tokens.data() + i, static_cast<int32_t>(std::min<size_t>(
                                      p->model->batch, p->tokens.size() - i)));
        int r = llama_decode(ctx, batch);
        check_cancel(cancel);
        if (r)
          throw failure(6, "prefill failed");
        observe(1, static_cast<uint32_t>(std::min<size_t>(
                       i + p->model->batch, p->tokens.size())));
      }
    }
    observe(2, usage->prompt_tokens);
    air_stream_buffer stream(p->stops, p->tool_parser ? air_tool_output_bytes : 0,
                             p->tool_parser ? air_tool_raw_storage : 0);
    std::string raw_output;
    size_t raw_piece_bytes = 0;
    if (p->tool_parser) {
      air_reserve_tool_output(raw_output);
    }
    auto emit = [&](bool final) {
      stream.flush(final, [&](const std::string &chunk) {
        check_cancel(cancel);
        if (p->tool_parser) {
          air_append_tool_output(raw_output, chunk);
          return;
        }
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
      const bool preserve = p->tool_parser && p->preserved_tokens.count(token);
      if (!preserve && !air_text_output_token_supported(llama_vocab_get_attr(vocab, token)))
        throw failure(4, "model emitted unsupported output token type");
      char small[256];
      int32_t n =
          llama_token_to_piece(vocab, token, small, sizeof(small), 0, preserve);
      std::string piece;
      if (n < 0) {
        if (p->tool_parser && static_cast<uint64_t>(-static_cast<int64_t>(n)) > air_tool_output_bytes - raw_piece_bytes)
          throw failure(11, "tool token exceeds remaining output budget");
        piece.resize(-n);
        n = llama_token_to_piece(vocab, token, piece.data(),
                                 static_cast<int32_t>(piece.size()), 0, preserve);
        if (n < 0)
          throw failure(6, "token decoding failed");
        piece.resize(n);
      } else
        piece.assign(small, n);
      if (p->tool_parser) {
        if (piece.size() > air_tool_output_bytes - raw_piece_bytes) throw failure(11, "tool output byte limit exceeded");
        air_check_tool_string_capacity(piece, air_tool_output_bytes);
        raw_piece_bytes += piece.size();
      }
      stream.push(piece);
      emit(false);
      if (stream.stopped()) {
        usage->finish_reason = 0;
        break;
      }
      if (i + 1 < p->options.max_tokens) {
        int r = 0;
        if (p->chunks) decode_text(&token, 1, true, false);
        else r = llama_decode(ctx, llama_batch_get_one(&token, 1));
        check_cancel(cancel);
        if (r)
          throw failure(6, "decode failed");
      }
    }
    emit(true);
    // pending may retain a full token's capacity after erase(). Drop it before
    // parsing/publishing, when normalized and downstream output copies coexist.
    stream.release();
    check_cancel(cancel);
    if (p->tool_parser) {
      if (usage->finish_reason != 0) throw failure(10, "tool generation ended at the token limit");
      air_tool_output message;
      try { message = air_normalize_tool_output(p->tool_parser->parse(raw_output)); }
      catch (const failure &) { throw; }
      catch (const std::bad_alloc &) { throw; }
      catch (...) { throw failure(9, "tool output parsing failed"); }
      if ((p->tool_choice == 0 && !message.tool_calls.empty()) ||
          (p->tool_choice >= 2 && message.tool_calls.empty()))
        throw failure(9, "generated result violates tool choice");
      auto borrowed = [](const std::string &value) {
        return air_string{reinterpret_cast<const uint8_t *>(value.data()), value.size()};
      };
      if (!message.content.empty() && chat_callback(user, 0, 0, {nullptr, 0}, borrowed(message.content)))
        throw failure(8, "chat consumer stopped");
      for (size_t i = 0; i < message.tool_calls.size(); ++i) {
        check_cancel(cancel);
        const auto &call = message.tool_calls[i];
        if (chat_callback(user, 1, static_cast<uint32_t>(i), borrowed(call.name), borrowed(call.arguments)))
          throw failure(8, "chat consumer stopped");
      }
      usage->finish_reason = message.tool_calls.empty() ? 0 : 4;
      check_cancel(cancel);
    }
  });
  if (usage && status)
    usage->finish_reason = (status == 2 || status == 8) ? 2 : 3;
  return status;
}
extern "C" int32_t air_generate_observed(
    air_prepared *raw, const air_cancel *cancel, air_text_callback callback,
    void *user, air_progress_callback progress, void *progress_user,
    air_usage *usage, air_error *error) {
  return generate_impl(raw, cancel, callback, nullptr, user, progress, progress_user, usage, error);
}
extern "C" int32_t air_generate_chat_v3(
    air_prepared *raw, const air_cancel *cancel, air_chat_callback_v3 callback,
    void *user, air_progress_callback progress, void *progress_user,
    air_usage *usage, air_error *error) {
  return generate_impl(raw, cancel, nullptr, callback, user, progress, progress_user, usage, error);
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
    *out = buffer("{\"shim_version\":5,\"backend\":\"cpu\",\"llama_commit\":"
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
