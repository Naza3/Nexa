#include "llm/llm.hpp"
#include "nexa_mnn.h"
#include "sampler.hpp"
#include "ujson.hpp"
#include <MNN/expr/ExprCreator.hpp>
#include <cassert>
#include <chrono>
#include <cstring>
#include <fstream>
#include <iostream>
#include <limits>
#include <sstream>
#include <thread>
using MNN::Transformer::Llm;
using Json = ujson::json;
template <class T> T init() {
  T t{};
  t.struct_size = sizeof(T);
  t.abi_version = 1;
  return t;
}
nexa_mnn_v1_bytes bytes(const std::string &s) {
  return {reinterpret_cast<const uint8_t *>(s.data()), s.size()};
}
struct Cancel {
  nexa_mnn_v1_cancel *c = nullptr;
  Cancel() { assert(!nexa_mnn_v1_cancel_create(&c)); }
  ~Cancel() { nexa_mnn_v1_cancel_destroy(c); }
};
struct Phase {
  nexa_mnn_v1_cancel *c;
  uint32_t phase;
  uint64_t count;
  bool reached = false;
  std::chrono::steady_clock::time_point requested;
  static void call(void *ptr, uint32_t phase, uint64_t count) {
    auto &p = *static_cast<Phase *>(ptr);
    if (!p.reached && phase == p.phase && count >= p.count) {
      p.reached = true;
      std::thread([&] {
        p.requested = std::chrono::steady_clock::now();
        nexa_mnn_v1_cancel_request(p.c);
      }).join();
    }
  }
};
struct Output {
  std::string value;
  int rc = 0;
  std::string marker;
  nexa_mnn_v1_cancel *cancel_after_marker = nullptr;
  nexa_mnn_v1_model *reenter = nullptr;
  bool tried = false;
  static int32_t text(void *ptr, nexa_mnn_v1_bytes b) {
    auto &o = *static_cast<Output *>(ptr);
    assert(b.len <= 4096);
    o.value.append(reinterpret_cast<const char *>(b.data), b.len);
    if (o.reenter && !o.tried) {
      o.tried = true;
      assert(nexa_mnn_v1_model_destroy(o.reenter, nullptr) == NEXA_MNN_BUSY);
    }
    if (!o.marker.empty() && o.value.find(o.marker) == std::string::npos)
      return 0;
    if (o.cancel_after_marker) {
      std::thread([&] {
        nexa_mnn_v1_cancel_request(o.cancel_after_marker);
      }).join();
      return 0;
    }
    return o.rc;
  }
};
struct Request {
  std::vector<std::string> content;
  std::vector<nexa_mnn_v1_message> messages;
  std::vector<std::string> stop;
  std::vector<nexa_mnn_v1_bytes> stops;
  nexa_mnn_v1_request q = init<nexa_mnn_v1_request>();
  Request(std::vector<std::string> c) : content(std::move(c)) {
    for (size_t i = 0; i < content.size(); ++i) {
      auto m = init<nexa_mnn_v1_message>();
      m.role = content.size() == 1 ? 2 : i == 0 ? 1 : i % 2 ? 2 : 3;
      m.content = bytes(content[i]);
      messages.push_back(m);
    }
    q.messages = messages.data();
    q.message_count = messages.size();
    q.max_tokens = 12;
    q.temperature = 0;
    q.top_p = 1;
    q.seed = 0;
  }
  void setstop(std::string s) {
    stop = {std::move(s)};
    stops = {bytes(stop[0])};
    q.stops = stops.data();
    q.stop_count = 1;
  }
};
struct Generated {
  std::string text;
  nexa_mnn_v1_result result;
};
Generated generate(nexa_mnn_v1_model *model, Request &request,
                   int32_t expected = 0, uint32_t cancel_phase = 0,
                   uint64_t cancel_count = 0, int callback = 0,
                   std::string marker = "", bool cancel_after_marker = false) {
  Cancel c;
  auto info = init<nexa_mnn_v1_prepared_info>();
  nexa_mnn_v1_prepared *p = nullptr;
  assert(nexa_mnn_v1_prepare(model, &request.q, c.c, &p, &info, nullptr) == 0);
  assert(nexa_mnn_v1_model_destroy(model, nullptr) == NEXA_MNN_BUSY);
  nexa_mnn_v1_prepared *extra = nullptr;
  auto i2 = init<nexa_mnn_v1_prepared_info>();
  assert(nexa_mnn_v1_prepare(model, &request.q, c.c, &extra, &i2, nullptr) ==
         NEXA_MNN_BUSY);
  assert(!extra);
  auto result = init<nexa_mnn_v1_result>();
  Output output;
  output.rc = callback;
  output.marker = std::move(marker);
  output.cancel_after_marker = cancel_after_marker ? c.c : nullptr;
  output.reenter = model;
  Phase phase{c.c, cancel_phase, cancel_count};
  assert(nexa_mnn_v1_generate(p, c.c, nullptr, nullptr, nullptr, nullptr,
                              &result, nullptr) == NEXA_MNN_INVALID);
  std::thread wrong([&] {
    assert(nexa_mnn_v1_prepared_destroy(p, nullptr) == NEXA_MNN_WRONG_THREAD);
    auto r = init<nexa_mnn_v1_result>();
    assert(nexa_mnn_v1_generate(p, c.c, Output::text, &output, nullptr, nullptr,
                                &r, nullptr) == NEXA_MNN_WRONG_THREAD);
  });
  wrong.join();
  auto rc = nexa_mnn_v1_generate(p, c.c, Output::text, &output,
                                 cancel_phase ? Phase::call : nullptr, &phase,
                                 &result, nullptr);
  if (rc != expected) {
    std::cerr << "unexpected_code " << rc << " expected " << expected << "\n";
    std::abort();
  }
  assert(result.prompt_tokens == info.prompt_tokens);
  assert(result.completion_tokens <= request.q.max_tokens);
  if (cancel_phase) {
    assert(phase.reached);
    assert(result.stop_reason == NEXA_MNN_STOP_CANCEL);
    std::cout << "cancel_phase " << cancel_phase << " safe_return_ms "
              << std::chrono::duration<double, std::milli>(
                     std::chrono::steady_clock::now() - phase.requested)
                     .count()
              << "\n";
  }
  assert(nexa_mnn_v1_generate(p, c.c, Output::text, &output, nullptr, nullptr,
                              &result, nullptr) == NEXA_MNN_CONSUMED);
  assert(!nexa_mnn_v1_prepared_destroy(p, nullptr));
  return {output.value, result};
}
void synthetic_sampler() {
  using namespace MNN::Transformer;
  using namespace MNN::Express;
  auto c = std::make_shared<LlmContext>();
  auto config = std::make_shared<LlmConfig>();
  config->config_ = Json::parse(
      "{\"sampler_type\":\"topP\",\"topP\":0.8,\"temperature\":0.7}");
  auto x = _Input({1, 4}, NCHW, halide_type_of<float>());
  float values[] = {0, 1, 2, 3};
  std::memcpy(x->writeMap<float>(), values, sizeof(values));
  Sampler a(c, config), b(c, config);
  a.nexaSeedV1(0);
  b.nexaSeedV1(0);
  for (int i = 0; i < 100; ++i) {
    int first = a.sample(x);
    assert(first == b.sample(x));
    assert(first >= 2 && first <= 3);
  }
  config->config_["sampler_type"] = "greedy";
  Sampler greedy(c, config);
  assert(greedy.sample(x) == 3);
  config->config_["sampler_type"] = "topP";
  config->config_["topP"] = 0.01;
  Sampler top(c, config);
  for (int i = 0; i < 20; ++i)
    assert(top.sample(x) == 3);
}
int main(int argc, char **argv) {
  auto b = init<nexa_mnn_v1_build>();
  assert(nexa_mnn_v1_build_info(&b) == 0);
  assert(b.silent_logs == 1);
  assert(nexa_mnn_v1_build_info(nullptr) == NEXA_MNN_ABI);
  auto bad = b;
  bad.struct_size--;
  assert(nexa_mnn_v1_build_info(&bad) == NEXA_MNN_ABI);
  assert(nexa_mnn_v1_cancel_create(nullptr) == NEXA_MNN_INVALID);
  assert(nexa_mnn_v1_model_destroy(nullptr, nullptr) == NEXA_MNN_INVALID);
  auto malformed = init<nexa_mnn_v1_error>();
  malformed.struct_size = 0;
  malformed.code = 777;
  assert(nexa_mnn_v1_model_destroy(nullptr, &malformed) == NEXA_MNN_ABI);
  assert(malformed.code == 777);
  synthetic_sampler();
  std::cout << "abi_sampler_pass\n";
  if (argc < 2)
    return 0;
  std::string path = argv[1],
              commit(reinterpret_cast<char *>(b.upstream_commit), 40),
              patch(reinterpret_cast<char *>(b.patch_sha256), 64),
              policy(reinterpret_cast<char *>(b.policy_sha256), 64),
              artifact(64, 'a');
  auto load = init<nexa_mnn_v1_load_options>();
  load.runtime_config_path = bytes(path);
  load.expected_upstream_commit = bytes(commit);
  load.expected_patch_sha256 = bytes(patch);
  load.policy_sha256 = bytes(policy);
  load.artifact_sha256 = bytes(artifact);
  load.logical_context = 2048;
  load.threads = 2;
  load.prefill_chunk = 32;
  if (argc == 3 && std::string(argv[2]) == "privacy-success") {
    {
      Cancel cancelled_load;
      Phase phase{cancelled_load.c, 1, 3};
      load.progress = Phase::call;
      load.progress_user = &phase;
      nexa_mnn_v1_model *m = nullptr;
      assert(nexa_mnn_v1_load(&load, cancelled_load.c, &m, nullptr) ==
             NEXA_MNN_CANCELLED);
      assert(phase.reached && !m);
    }
    load.progress = nullptr;
    load.progress_user = nullptr;
    Cancel c;
    nexa_mnn_v1_model *m = nullptr;
    assert(!nexa_mnn_v1_load(&load, c.c, &m, nullptr));
    Request en({"NEXA_PROMPT_EN_CANARY: Repeat exactly this word and nothing "
                "else: NEXA_OUTPUT_EN_CANARY"});
    en.q.max_tokens = 64;
    auto en_result = generate(m, en);
    assert(en_result.text.find("NEXA_OUTPUT_EN_CANARY") != std::string::npos);
    Request zh({"NEXA_PROMPT_ZH_"
                "CANARY。请逐字原样输出以下字符串，不要添加其他内容：NEXA_"
                "OUTPUT_ZH_CANARY"});
    zh.q.max_tokens = 64;
    auto zh_result = generate(m, zh);
    assert(zh_result.text.find("NEXA_OUTPUT_ZH_CANARY") != std::string::npos);
    Request multi({"You repeat requested words exactly.",
                   "Remember NEXA_OUTPUT_MULTI_CANARY.",
                   "I remember NEXA_OUTPUT_MULTI_CANARY.",
                   "NEXA_PROMPT_MULTI_CANARY: Repeat the remembered word "
                   "exactly, without commentary."});
    multi.q.max_tokens = 64;
    auto multi_result = generate(m, multi);
    assert(multi_result.text.find("NEXA_OUTPUT_MULTI_CANARY") !=
           std::string::npos);
    Request prefill({std::string(300, 'x') + " NEXA_PROMPT_PREFILL_CANARY"});
    generate(m, prefill, NEXA_MNN_CANCELLED, 4, 32);
    Request decode({"Repeat exactly NEXA_OUTPUT_DECODE_CANARY. "
                    "NEXA_PROMPT_DECODE_CANARY means do not add commentary."});
    decode.q.max_tokens = 64;
    auto cancelled = generate(m, decode, NEXA_MNN_CANCELLED, 0, 0, 0,
                              "NEXA_OUTPUT_DECODE_CANARY", true);
    assert(cancelled.text.find("NEXA_OUTPUT_DECODE_CANARY") !=
           std::string::npos);
    Request backpressure(
        {"Repeat exactly NEXA_OUTPUT_CALLBACK_CANARY. "
         "NEXA_PROMPT_CALLBACK_CANARY means do not add commentary."});
    backpressure.q.max_tokens = 64;
    auto failed = generate(m, backpressure, NEXA_MNN_CALLBACK, 0, 0, 2,
                           "NEXA_OUTPUT_CALLBACK_CANARY");
    assert(failed.text.find("NEXA_OUTPUT_CALLBACK_CANARY") !=
           std::string::npos);
    assert(generate(m, en).text == en_result.text);
    assert(!nexa_mnn_v1_model_destroy(m, nullptr));
    std::cout << "privacy_success_matrix_pass\n";
    return 0;
  }
  if (argc == 3 && (std::string(argv[2]) == "load-only" ||
                    std::string(argv[2]) == "prepare-only")) {
    Cancel c;
    nexa_mnn_v1_model *m = nullptr;
    auto error = init<nexa_mnn_v1_error>();
    bool host_logged = false;
    load.progress = [](void *user, uint32_t, uint64_t) {
      auto &logged = *static_cast<bool *>(user);
      if (!logged) {
        logged = true;
        std::thread([] { std::cout << "host_parallel_marker\n"; }).join();
      }
    };
    load.progress_user = &host_logged;
    auto rc = nexa_mnn_v1_load(&load, c.c, &m, &error);
    if (rc == 0 && std::string(argv[2]) == "prepare-only") {
      Request q({"NEXA_PROMPT_CANARY"});
      nexa_mnn_v1_prepared *prepared = nullptr;
      auto info = init<nexa_mnn_v1_prepared_info>();
      rc = nexa_mnn_v1_prepare(m, &q.q, c.c, &prepared, &info, &error);
      if (prepared)
        nexa_mnn_v1_prepared_destroy(prepared, nullptr);
    }
    assert(host_logged);
    std::cout << "load_result " << rc << "\n";
    if (m)
      nexa_mnn_v1_model_destroy(m, nullptr);
    return rc == 0 ? 1 : 0;
  }
  for (uint64_t checkpoint : {0, 2, 3, 4, 5, 6}) {
    Cancel c;
    Phase phase{c.c, 1, checkpoint};
    load.progress = Phase::call;
    load.progress_user = &phase;
    nexa_mnn_v1_model *m = nullptr;
    auto loadrc = nexa_mnn_v1_load(&load, c.c, &m, nullptr);
    if (loadrc != NEXA_MNN_CANCELLED) {
      std::cerr << "checkpoint " << checkpoint << " code " << loadrc
                << " reached " << phase.reached << "\n";
      std::abort();
    }
    assert(!m && phase.reached);
    std::cout << "cancel_load_checkpoint " << checkpoint << " safe_return_ms "
              << std::chrono::duration<double, std::milli>(
                     std::chrono::steady_clock::now() - phase.requested)
                     .count()
              << "\n";
  }
  load.progress = nullptr;
  load.progress_user = nullptr;
  Cancel c;
  nexa_mnn_v1_model *model = nullptr;
  auto error = init<nexa_mnn_v1_error>();
  auto rc = nexa_mnn_v1_load(&load, c.c, &model, &error);
  if (rc) {
    std::cerr << "load_code " << rc << "\n";
    return 1;
  }
  std::thread([&] {
    assert(nexa_mnn_v1_model_destroy(model, nullptr) == NEXA_MNN_WRONG_THREAD);
  }).join();
  {
    Request invalid({"ABI edge"});
    auto info = init<nexa_mnn_v1_prepared_info>();
    nexa_mnn_v1_prepared *prepared = nullptr;
    auto reject = [&] {
      prepared = reinterpret_cast<nexa_mnn_v1_prepared *>(uintptr_t(1));
      assert(nexa_mnn_v1_prepare(model, &invalid.q, c.c, &prepared, &info,
                                 nullptr) != 0);
      assert(prepared == nullptr);
    };
    invalid.q.temperature = std::numeric_limits<float>::quiet_NaN();
    reject();
    invalid.q.temperature = 0;
    invalid.q.top_p = 0;
    reject();
    invalid.q.top_p = 1;
    invalid.q.reserved = 1;
    reject();
    invalid.q.reserved = 0;
    invalid.q.max_tokens = 0;
    reject();
    invalid.q.max_tokens = 12;
    invalid.messages[0].role = 99;
    reject();
    invalid.messages[0].role = NEXA_MNN_ROLE_USER;
    invalid.messages[0].content = {nullptr, 1};
    reject();
    invalid.messages[0].content = {nullptr, UINT64_MAX};
    reject();
    invalid.messages[0].content = bytes(invalid.content[0]);
    invalid.q.message_count = UINT64_MAX;
    reject();
    invalid.q.message_count = 1;
    invalid.q.stop_count = UINT64_MAX;
    reject();
    invalid.q.stop_count = 0;
    invalid.setstop(std::string("\xff", 1));
    reject();
    invalid.q.stop_count = 0;
    std::thread([&] {
      assert(nexa_mnn_v1_prepare(model, &invalid.q, c.c, &prepared, &info,
                                 nullptr) == NEXA_MNN_WRONG_THREAD);
    }).join();
  }
  Request en({"Reply with one short sentence about the sky."});
  auto first = generate(model, en);
  assert(!first.text.empty());
  assert(first.result.completion_tokens > 0);
  Request zh({"请用一句话描述天空。"});
  auto chinese = generate(model, zh);
  assert(!chinese.text.empty());
  Request multi({"Answer concisely.", "Remember the word bamboo.",
                 "I remember bamboo.", "What word did I ask you to remember?"});
  assert(!generate(model, multi).text.empty());
  Request empty({""});
  generate(model, empty);
  Request eos({"Reply only with OK."});
  eos.q.max_tokens = 64;
  auto eos_result = generate(model, eos);
  assert(eos_result.result.stop_reason == NEXA_MNN_STOP_EOS);
  Request one({"Count from one."});
  one.q.max_tokens = 1;
  auto maxone = generate(model, one);
  assert(maxone.result.completion_tokens == 1);
  Request stopped({"Reply with one short sentence about the sky."});
  stopped.setstop(first.text.substr(0, 1));
  auto stopresult = generate(model, stopped);
  assert(stopresult.result.stop_reason == NEXA_MNN_STOP_USER);
  assert(stopresult.text.empty());
  en.q.temperature = .7f;
  en.q.top_p = .9f;
  en.q.seed = 42;
  auto a = generate(model, en);
  en.q.temperature = 1.5f;
  en.q.top_p = .5f;
  en.q.seed = 999;
  generate(model, en);
  en.q.temperature = .7f;
  en.q.top_p = .9f;
  en.q.seed = 42;
  auto again = generate(model, en);
  assert(a.text == again.text &&
         a.result.completion_tokens == again.result.completion_tokens);
  for (uint32_t phase : {2, 3}) {
    Cancel pc;
    Phase p{pc.c, phase, 0};
    en.q.progress = Phase::call;
    en.q.progress_user = &p;
    nexa_mnn_v1_prepared *prepared = nullptr;
    auto info = init<nexa_mnn_v1_prepared_info>();
    assert(nexa_mnn_v1_prepare(model, &en.q, pc.c, &prepared, &info, nullptr) ==
           NEXA_MNN_CANCELLED);
    assert(p.reached && !prepared);
    std::cout << "cancel_prepare_phase " << phase << " safe_return_ms "
              << std::chrono::duration<double, std::milli>(
                     std::chrono::steady_clock::now() - p.requested)
                     .count()
              << "\n";
    en.q.progress = nullptr;
    en.q.progress_user = nullptr;
    generate(model, en);
  }
  Request longprompt({std::string(300, 'x') + " Describe the sky."});
  generate(model, longprompt, NEXA_MNN_CANCELLED, 4, 32);
  generate(model, en);
  generate(model, en, NEXA_MNN_CANCELLED, 5, 1);
  generate(model, en);
  generate(model, en, NEXA_MNN_CALLBACK, 0, 0, 2);
  generate(model, en);
  // Budget equality uses the exact same full template token count, never
  // characters.
  Request budget({"Say yes."});
  auto normal = generate(model, budget);
  budget.q.max_tokens = 2048 - normal.result.prompt_tokens + 1;
  nexa_mnn_v1_prepared *p = nullptr;
  auto info = init<nexa_mnn_v1_prepared_info>();
  assert(nexa_mnn_v1_prepare(model, &budget.q, c.c, &p, &info, nullptr) ==
         NEXA_MNN_BUDGET);
  budget.q.max_tokens--;
  assert(!nexa_mnn_v1_prepare(model, &budget.q, c.c, &p, &info, nullptr));
  assert(!nexa_mnn_v1_prepared_destroy(p, nullptr));
  assert(!nexa_mnn_v1_model_destroy(model, nullptr));
  // Independent unhooked exact-upstream greedy path: same native rendered input
  // and tokenization, full generated bytes and accepted-token usage must match.
  std::unique_ptr<Llm> baseline(Llm::createLLM(path));
  assert(baseline->load());
  assert(baseline->set_config("{\"jinja\":{\"context\":{\"enable_thinking\":"
                              "false}},\"sampler_type\":\"greedy\"}"));
  MNN::Transformer::ChatMessages msg = {
      {"user", "Reply with one short sentence about the sky."}};
  auto rendered = baseline->apply_chat_template(msg);
  auto tokens = baseline->tokenizer_encode(rendered);
  std::ostringstream output;
  baseline->generate_init(&output, "");
  baseline->generate(tokens, 12);
  assert(output.str() == first.text);
  assert(tokens.size() == first.result.prompt_tokens);
  assert(baseline->getContext()->output_tokens.size() ==
         first.result.completion_tokens);
  auto expected_tokens = baseline->getContext()->output_tokens;
  baseline->reset();
  baseline->generate_init(nullptr, "");
  assert(baseline->nexaResetSamplerV1(0, 1, 0));
  std::vector<int> accepted;
  MNN::Transformer::NexaRequestHooksV1 hooks;
  hooks.user = &accepted;
  hooks.token = [](void *user, int token, bool) noexcept {
    static_cast<std::vector<int> *>(user)->push_back(token);
    return 0;
  };
  baseline->nexaSetHooksV1(&hooks);
  baseline->generate(tokens, 12);
  baseline->nexaSetHooksV1(nullptr);
  assert(accepted == expected_tokens);
  assert(baseline->getContext()->generate_str.empty());
  if (baseline->getContext()->status ==
      MNN::Transformer::LlmStatus::MAX_TOKENS_FINISHED)
    assert(baseline->getContext()->all_seq_len ==
           int(tokens.size() + accepted.size() - 1));
  if (argc == 3) {
    Json report = Json::object();
    report["shim_text"] = first.text;
    report["shim_prompt_count"] = first.result.prompt_tokens;
    report["shim_completion_count"] = first.result.completion_tokens;
    report["rendered_prompt"] = rendered;
    report["prompt_tokens"] = Json::array();
    for (int t : tokens)
      report["prompt_tokens"].push_back(t);
    report["output_tokens"] = Json::array();
    for (int t : baseline->getContext()->output_tokens)
      report["output_tokens"].push_back(t);
    std::ofstream(argv[2]) << report.dump();
  }
  std::cout << "real_request_pass\n";
}
