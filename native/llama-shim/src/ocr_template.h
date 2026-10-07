#pragma once
#include "text_template.h"

// GLM's OCR template has no assistant-history EOS. Validate only the supported
// single-user continuation against its actual Jinja, without weakening text chat.
class air_ocr_template {
  common_chat_template tmpl;
  std::string apply(const std::string &content) const {
    autoparser::generation_params params;
    params.messages = common_json::array({{{"role", "user"}, {"content", content}}});
    params.add_generation_prompt = true;
    params.enable_thinking = false;
    params.reasoning_format = COMMON_REASONING_FORMAT_NONE;
    params.add_bos = false;
    params.add_eos = false;
    return common_chat_template_direct_apply(tmpl, params);
  }
public:
  air_ocr_template(const std::string &source, const std::string &bos,
                   const std::string &eos) : tmpl(source, bos, eos) {
    if (source.empty() || source.size() > 1048576)
      throw failure(4, "invalid OCR template");
    render("NEXA_OCR_TEMPLATE_PROBE");
  }
  std::string render(const std::string &content) const {
    auto prompt = apply(content);
    // This is a validation oracle, never a replacement or fallback template.
    const auto expected = "[gMASK]<sop><|user|>\n" + content + "<|assistant|>";
    if (prompt != expected)
      throw failure(4, "embedded template does not support the single-image OCR contract");
    return prompt;
  }
};
