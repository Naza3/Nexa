#include "identity.h"
#include "nexa_mnn.h"
#if !defined(NEXA_MNN_SILENT_LOGS) || NEXA_MNN_SILENT_LOGS != 1
#error Private shim requires silent MNN logging
#endif
#include "llm/llm.hpp"
#include "stream_buffer.h"
#include "ujson.hpp"
#include <atomic>
#include <cmath>
#include <cstring>
#include <fstream>
#include <memory>
#include <random>
#include <set>
#include <thread>
using MNN::Transformer::Llm;
using MNN::Transformer::LlmStatus;
using MNN::Transformer::NexaRequestHooksV1;
using Json = ujson::json;
struct nexa_mnn_v1_cancel {
  std::atomic<bool> requested{false};
};
struct nexa_mnn_v1_model {
  std::thread::id owner = std::this_thread::get_id();
  std::unique_ptr<Llm> llm;
  uint32_t context = 0;
  bool busy = false, prepared = false, faulted = false;
};
struct nexa_mnn_v1_prepared {
  nexa_mnn_v1_model *model;
  std::vector<int> tokens;
  std::vector<std::string> stops;
  uint32_t max_tokens = 0, seed = 0;
  bool consumed = false;
};
namespace {
void require(bool b, int32_t c = NEXA_MNN_INVALID) {
  if (!b)
    throw failure(c, "request rejected");
}
template <class T> bool valid(const T *v) {
  return v && v->struct_size == sizeof(T) && v->abi_version == 1;
}
template <class T> void header(const T *v) { require(valid(v), NEXA_MNN_ABI); }
void err(nexa_mnn_v1_error *e, int32_t c) noexcept {
  if (!valid(e))
    return;
  const char *s = c == 0    ? ""
                  : c == 5  ? "operation cancelled"
                  : c == 7  ? "logical token budget exceeded"
                  : c == 3  ? "owner thread required"
                  : c == 4  ? "model operation active"
                  : c == 8  ? "prepared request already consumed"
                  : c == 9  ? "output callback failed"
                  : c == 10 ? "native identity mismatch"
                            : "native request rejected";
  e->code = c;
  e->message_len = uint32_t(std::strlen(s));
  std::memset(e->message, 0, sizeof(e->message));
  std::memcpy(e->message, s, e->message_len);
}
template <class F> int32_t boundary(nexa_mnn_v1_error *e, F f) noexcept {
  if (e && !valid(e))
    return NEXA_MNN_ABI;
  try {
    f();
    err(e, 0);
    return 0;
  } catch (const failure &x) {
    err(e, x.code);
    return x.code;
  } catch (...) {
    err(e, 6);
    return 6;
  }
}
std::string bytes(nexa_mnn_v1_bytes b, size_t max = 1048576,
                  bool empty = true) {
  require(b.len <= max && (b.data || b.len == 0) && (empty || b.len));
  std::string s;
  if (b.len)
    s.assign(reinterpret_cast<const char *>(b.data), size_t(b.len));
  require(utf8_prefix(s) == s.size());
  return s;
}
void owner(nexa_mnn_v1_model *m) {
  require(m);
  require(m->owner == std::this_thread::get_id(), 3);
}
struct Busy {
  nexa_mnn_v1_model *m;
  explicit Busy(nexa_mnn_v1_model *p) : m(p) {
    owner(m);
    require(!m->busy, 4);
    m->busy = true;
  }
  ~Busy() { m->busy = false; }
};
bool cancelled(nexa_mnn_v1_cancel *c) {
  return c && c->requested.load(std::memory_order_acquire);
}
struct Hooks {
  nexa_mnn_v1_cancel *cancel;
  nexa_mnn_v1_progress progress;
  void *progress_user;
  nexa_mnn_v1_prepared *prepared = nullptr;
  nexa_mnn_v1_text text = nullptr;
  void *text_user = nullptr;
  std::unique_ptr<air_stream_buffer> stream;
  bool user_stop = false;
  int32_t failure_code = 0;
  void checkpoint(uint32_t phase, uint64_t count) {
    if (progress)
      progress(progress_user, phase, count);
    require(!cancelled(cancel), 5);
  }
  void emit(const std::string &s) {
    require(!cancelled(cancel), 5);
    int32_t rc = text(text_user,
                      {reinterpret_cast<const uint8_t *>(s.data()), s.size()});
    require(rc == 0, rc == 1 ? 5 : 9);
    require(!cancelled(cancel), 5);
  }
  static bool cancel_fn(void *p) noexcept {
    return cancelled(static_cast<Hooks *>(p)->cancel);
  }
  static void progress_fn(void *p, uint32_t phase, uint32_t count) noexcept {
    auto h = static_cast<Hooks *>(p);
    if (h->progress)
      h->progress(h->progress_user, phase, count);
  }
  static int token_fn(void *p, int id, bool terminal) noexcept {
    auto &h = *static_cast<Hooks *>(p);
    try {
      require(!cancelled(h.cancel), 5);
      if (!terminal) {
        h.stream->push(h.prepared->model->llm->tokenizer_decode(id));
        h.stream->flush(false, [&](const std::string &s) { h.emit(s); });
      }
      h.user_stop = h.stream->stopped();
      return h.user_stop ? 1 : 0;
    } catch (const failure &e) {
      h.failure_code = e.code;
      return e.code == 5 ? 2 : 3;
    } catch (...) {
      h.failure_code = 6;
      return 3;
    }
  }
};
struct HookLease {
  Llm *llm;
  HookLease(Llm *l, Hooks &h, bool tokens = false) : llm(l) {
    NexaRequestHooksV1 x{&h, Hooks::cancel_fn, Hooks::progress_fn,
                         tokens ? Hooks::token_fn : nullptr};
    llm->nexaSetHooksV1(&x);
  }
  ~HookLease() { llm->nexaSetHooksV1(nullptr); }
};
void json_shape(const rapidjson::Value &v, unsigned depth = 0) {
  require(depth <= 64);
  if (v.IsObject()) {
    std::set<std::string> names;
    for (auto i = v.MemberBegin(); i != v.MemberEnd(); ++i) {
      require(names
                  .insert(std::string(i->name.GetString(),
                                      i->name.GetStringLength()))
                  .second);
      json_shape(i->value, depth + 1);
    }
  } else if (v.IsArray())
    for (auto &child : v.GetArray())
      json_shape(child, depth + 1);
}
void check_config(const Json &j, const nexa_mnn_v1_load_options &o) {
  require(j.is_object());
  static const std::set<std::string> keys = {"base_dir",
                                             "llm_model",
                                             "llm_weight",
                                             "llm_config",
                                             "tokenizer_file",
                                             "context_file",
                                             "backend_type",
                                             "thread_num",
                                             "precision",
                                             "memory",
                                             "sampler_type",
                                             "async",
                                             "chunk",
                                             "max_all_tokens",
                                             "max_new_tokens",
                                             "reuse_kv",
                                             "prompt_cache",
                                             "use_mmap",
                                             "use_cached_mmap",
                                             "kvcache_mmap",
                                             "speculative_type",
                                             "hidden_size",
                                             "layer_nums",
                                             "attention_mask",
                                             "key_value_shape",
                                             "bos",
                                             "system_prompt_template",
                                             "user_prompt_template",
                                             "assistant_prompt_template",
                                             "is_visual",
                                             "jinja",
                                             "tie_embeddings"};
  auto dumped = j.dump();
  rapidjson::Document shape;
  shape.Parse(dumped.c_str());
  require(shape.IsObject());
  for (auto i = shape.MemberBegin(); i != shape.MemberEnd(); ++i)
    require(keys.count(i->name.GetString()) != 0);
  require(j.value("hidden_size", 0) == 1024 && j.value("layer_nums", 0) == 28 &&
          j.value("attention_mask", "") == "float");
  require(j["tie_embeddings"].dump() == "[275780066,431362530,19447808,8,64]");
  require(j["key_value_shape"].dump() == "[2,1,0,8,128]");
  require(j.value("sampler_type", "") == "greedy");
  require(j.value("backend_type", "") == "cpu" &&
          j.value("thread_num", 0) == int(o.threads));
  require(j.value("precision", "") == "high" && j.value("memory", "") == "low");
  require(j.value("chunk", 0) == int(o.prefill_chunk) &&
          !j.contains("chunk_limits"));
  require(!j.value("async", true) &&
          j.value("max_all_tokens", 0) == int(o.logical_context));
  for (auto k : {"reuse_kv", "prompt_cache", "use_mmap", "use_cached_mmap",
                 "kvcache_mmap", "is_visual", "is_audio", "has_talker",
                 "has_ple", "hidden_states"})
    require(!j.value(k, false));
  require(j.value("speculative_type", "").empty());
  require(j["jinja"]["chat_template"].is_string() &&
          !j["jinja"]["chat_template"].get<std::string>().empty());
}
} // namespace
extern "C" {
int32_t nexa_mnn_v1_build_info(nexa_mnn_v1_build *out) {
  return boundary(nullptr, [&] {
    header(out);
    std::memcpy(out->upstream_commit, NEXA_MNN_COMMIT, 40);
    std::memcpy(out->patch_sha256, NEXA_MNN_PATCH, 64);
    std::memcpy(out->policy_sha256, NEXA_MNN_POLICY, 64);
    out->silent_logs = 1;
    out->reserved = 0;
  });
}
int32_t nexa_mnn_v1_cancel_create(nexa_mnn_v1_cancel **out) {
  if (out)
    *out = nullptr;
  return boundary(nullptr, [&] {
    require(out);
    *out = new nexa_mnn_v1_cancel;
  });
}
void nexa_mnn_v1_cancel_request(nexa_mnn_v1_cancel *c) {
  if (c)
    c->requested.store(true, std::memory_order_release);
}
void nexa_mnn_v1_cancel_destroy(nexa_mnn_v1_cancel *c) { delete c; }
int32_t nexa_mnn_v1_load(const nexa_mnn_v1_load_options *o,
                         nexa_mnn_v1_cancel *c, nexa_mnn_v1_model **out,
                         nexa_mnn_v1_error *error) {
  if (out)
    *out = nullptr;
  return boundary(error, [&] {
    header(o);
    require(out && c);
    require(o->reserved == 0 && o->threads >= 1 && o->threads <= 2 &&
            o->logical_context >= 1 && o->logical_context <= 2048 &&
            o->prefill_chunk >= 1 && o->prefill_chunk <= 128);
    require(bytes(o->expected_upstream_commit, 40, false) == NEXA_MNN_COMMIT &&
                bytes(o->expected_patch_sha256, 64, false) == NEXA_MNN_PATCH &&
                bytes(o->policy_sha256, 64, false) == NEXA_MNN_POLICY,
            10);
    auto artifact = bytes(o->artifact_sha256, 64, false);
    require(artifact.size() == 64 &&
                artifact.find_first_not_of("0123456789abcdef") ==
                    std::string::npos,
            10);
    auto path = bytes(o->runtime_config_path, 4096, false);
    require(path.find('\0') == std::string::npos);
    Hooks h{c, o->progress, o->progress_user};
    h.checkpoint(1, 0);
    std::ifstream file(path, std::ios::binary);
    require(bool(file), 6);
    std::string raw(1048577, '\0');
    file.read(raw.data(), raw.size());
    raw.resize(size_t(file.gcount()));
    require(raw.size() <= 1048576, 6);
    rapidjson::Document checked;
    checked.Parse<rapidjson::kParseValidateEncodingFlag>(raw.data(),
                                                         raw.size());
    require(!checked.HasParseError() && checked.IsObject(), 6);
    json_shape(checked);
    auto input = Json::parse(raw);
    check_config(input, *o);
    auto m = std::make_unique<nexa_mnn_v1_model>();
    m->context = o->logical_context;
    m->llm.reset(Llm::createLLM(path));
    require(bool(m->llm), 6);
    h.checkpoint(1, 0);
    check_config(Json::parse(m->llm->dump_config()), *o);
    HookLease hooks(m->llm.get(), h);
    bool loaded = m->llm->load();
    require(loaded, cancelled(c) ? 5 : 6);
    h.checkpoint(1, 7);
    Json fixed = Json::object();
    fixed["jinja"] = input["jinja"];
    fixed["jinja"]["context"] = Json::object();
    fixed["jinja"]["context"]["enable_thinking"] = false;
    require(m->llm->set_config(fixed.dump()), 6);
    check_config(Json::parse(m->llm->dump_config()), *o);
    h.checkpoint(1, 8);
    *out = m.release();
  });
}
int32_t nexa_mnn_v1_prepare(nexa_mnn_v1_model *m, const nexa_mnn_v1_request *q,
                            nexa_mnn_v1_cancel *c, nexa_mnn_v1_prepared **out,
                            nexa_mnn_v1_prepared_info *info,
                            nexa_mnn_v1_error *error) {
  if (out)
    *out = nullptr;
  return boundary(error, [&] {
    header(q);
    header(info);
    info->prompt_tokens = 0;
    info->resolved_seed = 0;
    info->reserved = 0;
    require(out && c);
    Busy busy(m);
    require(!m->prepared, 4);
    require(!m->faulted, 6);
    require(q->reserved == 0 && q->messages && q->message_count > 0 &&
            q->message_count <= 4096 && q->stop_count <= 4 &&
            (q->stops || !q->stop_count));
    require(q->max_tokens > 0 && q->max_tokens <= m->context &&
            std::isfinite(q->temperature) && q->temperature >= 0 &&
            q->temperature <= 2 && std::isfinite(q->top_p) && q->top_p > 0 &&
            q->top_p <= 1);
    auto p = std::make_unique<nexa_mnn_v1_prepared>();
    p->model = m;
    p->max_tokens = q->max_tokens;
    p->seed = q->seed;
    MNN::Transformer::ChatMessages messages;
    uint64_t total = 0;
    for (uint64_t i = 0; i < q->message_count; ++i) {
      auto &msg = q->messages[i];
      header(&msg);
      require(!msg.reserved && msg.role >= 1 && msg.role <= 3);
      require(msg.content.len <= 1048576 - total);
      total += msg.content.len;
      messages.emplace_back(msg.role == 1   ? "system"
                            : msg.role == 2 ? "user"
                                            : "assistant",
                            bytes(msg.content));
    }
    for (uint64_t i = 0; i < q->stop_count; ++i)
      p->stops.push_back(bytes(q->stops[i], 128, false));
    Hooks h{c, q->progress, q->progress_user};
    h.checkpoint(2, 0);
    HookLease hooks(m->llm.get(), h);
    m->llm->reset();
    m->llm->generate_init(nullptr, "");
    if (p->seed == UINT32_MAX)
      p->seed = std::random_device{}();
    require(m->llm->nexaResetSamplerV1(q->temperature, q->top_p, p->seed), 6);
    h.checkpoint(2, 1);
    auto rendered = m->llm->apply_chat_template(messages);
    require(!rendered.empty() && rendered.size() <= 1048576, 6);
    h.checkpoint(2, 2);
    h.checkpoint(3, 0);
    p->tokens = m->llm->tokenizer_encode(rendered);
    h.checkpoint(3, p->tokens.size());
    require(!p->tokens.empty(), 6);
    require(p->tokens.size() <= m->context &&
                p->max_tokens <= m->context - p->tokens.size(),
            7);
    info->prompt_tokens = p->tokens.size();
    info->resolved_seed = p->seed;
    info->reserved = 0;
    m->prepared = true;
    *out = p.release();
  });
}
int32_t nexa_mnn_v1_generate(nexa_mnn_v1_prepared *p, nexa_mnn_v1_cancel *c,
                             nexa_mnn_v1_text text, void *user,
                             nexa_mnn_v1_progress progress, void *progress_user,
                             nexa_mnn_v1_result *result,
                             nexa_mnn_v1_error *error) {
  return boundary(error, [&] {
    header(result);
    require(p && c && text);
    Busy busy(p->model);
    require(!p->consumed, 8);
    p->consumed = true;
    result->prompt_tokens = p->tokens.size();
    result->completion_tokens = 0;
    result->resolved_seed = p->seed;
    result->stop_reason = 5;
    Hooks h{c, progress, progress_user};
    h.prepared = p;
    h.text = text;
    h.text_user = user;
    h.stream = std::make_unique<air_stream_buffer>(p->stops);
    HookLease hooks(p->model->llm.get(), h, true);
    try {
      h.checkpoint(4, 0);
      p->model->llm->generate(p->tokens, p->max_tokens);
      result->completion_tokens =
          p->model->llm->getContext()->output_tokens.size();
      if (h.failure_code)
        throw failure(h.failure_code, "generation stopped");
      require(!cancelled(c), 5);
      auto status = p->model->llm->getContext()->status;
      require(status == LlmStatus::NORMAL_FINISHED ||
                  status == LlmStatus::MAX_TOKENS_FINISHED,
              status == LlmStatus::USER_CANCEL ? 5 : 6);
      if (!h.user_stop)
        h.stream->flush(true, [&](const std::string &s) { h.emit(s); });
      result->stop_reason = h.user_stop                            ? 3
                            : status == LlmStatus::NORMAL_FINISHED ? 1
                                                                   : 2;
    } catch (const failure &e) {
      result->completion_tokens =
          p->model->llm->getContext()->output_tokens.size();
      result->stop_reason = e.code == 5 ? 4 : 5;
      if (e.code == 6)
        p->model->faulted = true;
      throw;
    } catch (...) {
      result->completion_tokens =
          p->model->llm->getContext()->output_tokens.size();
      p->model->faulted = true;
      throw;
    }
  });
}
int32_t nexa_mnn_v1_prepared_destroy(nexa_mnn_v1_prepared *p,
                                     nexa_mnn_v1_error *e) {
  return boundary(e, [&] {
    require(p);
    owner(p->model);
    require(!p->model->busy, 4);
    p->model->prepared = false;
    delete p;
  });
}
int32_t nexa_mnn_v1_model_destroy(nexa_mnn_v1_model *m, nexa_mnn_v1_error *e) {
  return boundary(e, [&] {
    owner(m);
    require(!m->busy && !m->prepared, 4);
    delete m;
  });
}
}
