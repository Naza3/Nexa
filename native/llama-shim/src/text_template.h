#pragma once
// A deliberately narrow text continuation contract over the model's actual
// Jinja. No fallback templates, role rewriting, tool grammar or output parser.
#include "chat.h"
#include "chat-auto-parser.h"
#include "stream_buffer.h"
#include <functional>
#include <memory>

static bool air_text_output_token_supported(llama_token_attr attributes) {
  return !(attributes & (LLAMA_TOKEN_ATTR_CONTROL | LLAMA_TOKEN_ATTR_UNKNOWN));
}

class air_text_template {
  common_chat_template tmpl;
  bool add_bos;
  bool add_eos;
  bool system_supported = false;
  std::function<bool(const std::string &)> suffix_supported;

  static const std::string &validated_source(const std::string &source,
                                              const std::string &bos,
                                              const std::string &eos) {
    if (source.empty() || source.size() > 1048576 || source == "chatml" ||
        (bos.empty() && source.find("bos_token") != std::string::npos) ||
        (eos.empty() && source.find("eos_token") != std::string::npos))
      throw failure(4, "missing embedded template or required vocabulary token");
    return source;
  }
  std::string apply(const common_json &messages, bool generation) const {
    autoparser::generation_params params;
    params.messages = messages;
    params.add_generation_prompt = generation;
    params.enable_thinking = false;
    params.reasoning_format = COMMON_REASONING_FORMAT_NONE;
    params.add_bos = add_bos;
    params.add_eos = add_eos;
    auto result = common_chat_template_direct_apply(tmpl, params);
    if (result.size() > 4194304)
      throw failure(4, "formatted prompt exceeds text adapter limit");
    return result;
  }
  static size_t unique(const std::string &text, const std::string &needle) {
    auto pos = text.find(needle);
    if (pos == std::string::npos || text.find(needle, pos + needle.size()) != std::string::npos)
      throw failure(4, "template does not preserve plain message content");
    return pos;
  }
  std::string checked(const common_json &messages) const {
    // This check is repeated for the real history, not just an optimistic
    // template capability probe. No appended sentinel is sent to inference.
    const auto serialized = messages.dump();
    std::string sentinel;
    for (size_t attempt = 0; attempt < 128; ++attempt) {
      auto candidate = "NEXA_TEXT_PROBE_7bd30a6f_ASST_" + std::to_string(attempt);
      if (serialized.find(candidate) == std::string::npos &&
          tmpl.source().find(candidate) == std::string::npos) {
        sentinel = std::move(candidate);
        break;
      }
    }
    if (sentinel.empty()) throw failure(4, "template probe collision limit");
    auto prompt = apply(messages, true);
    auto complete = messages;
    complete.push_back({{"role", "assistant"}, {"content", sentinel}});
    auto rendered = apply(complete, false);
    auto pos = unique(rendered, sentinel);
    if (rendered.substr(0, pos) != prompt ||
        !suffix_supported(rendered.substr(pos + sentinel.size())))
      throw failure(4, "template requires unsupported output framing or reasoning");
    return prompt;
  }
public:
  air_text_template(const std::string &source, const std::string &bos,
                    const std::string &eos, bool add_bos_, bool add_eos_,
                    std::function<bool(const std::string &)> suffix)
      : tmpl(validated_source(source, bos, eos), bos, eos), add_bos(add_bos_), add_eos(add_eos_),
        suffix_supported(std::move(suffix)) {
    const std::string user = "NEXA_TEXT_PROBE_5ce70d2a_USER";
    const std::string answer = "NEXA_TEXT_PROBE_162cfb80_HISTORY";
    const std::string next = "NEXA_TEXT_PROBE_38f4be09_NEXT";
    common_json messages = common_json::array({{{"role", "user"}, {"content", user}}});
    unique(checked(messages), user);
    messages.push_back({{"role", "assistant"}, {"content", answer}});
    messages.push_back({{"role", "user"}, {"content", next}});
    auto history = checked(messages);
    auto first = unique(history, user);
    auto second = unique(history, answer);
    auto third = unique(history, next);
    if (!(first < second && second < third))
      throw failure(4, "template does not preserve message order");
    // Unsupported system roles remain an explicit request error, never merged
    // into the first user message or silently discarded by common polyfills.
    try {
      const std::string system = "NEXA_TEXT_PROBE_9fb2d14e_SYSTEM";
      auto with_system = common_json::array({common_json{{"role", "system"}, {"content", system}}});
      with_system.insert(messages);
      auto rendered = checked(with_system);
      system_supported = tmpl.original_caps().supports_system_role &&
                         unique(rendered, system) < unique(rendered, user);
    } catch (...) {
      system_supported = false;
    }
  }
  std::string render(const std::vector<common_chat_msg> &messages) const {
    common_json input = common_json::array();
    for (const auto &message : messages) {
      if (message.role == "system" && !system_supported)
        throw failure(4, "template cannot preserve a system message");
      input.push_back({{"role", message.role}, {"content", message.content}});
    }
    return checked(input);
  }
};
