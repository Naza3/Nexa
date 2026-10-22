#include "air_llama.h"
#include "chat.h"
#include "log.h"
#include "../src/tool_template.h"

#include <cassert>
#include <cstdlib>
#include <fstream>
#include <iostream>
#include <iterator>

// Diagnostic only: a full PEG match is not a tool/schema acceptance decision.
struct observation {
  bool strict_complete;
  bool mapped;
  size_t calls;
  bool arguments_match;
};
static observation observe(const common_chat_parser_params &params,
                           const std::string &raw) {
  const std::string effective = params.generation_prompt + raw;
  common_peg_parse_context ctx(effective, COMMON_PEG_PARSE_FLAG_NONE);
  const auto result = params.parser.parse(ctx);
  observation found{result.success() && result.end == effective.size(), false, 0, false};
  try {
    const auto parsed = common_chat_parse(raw, false, params);
    found.mapped = true;
    found.calls = parsed.tool_calls.size();
    found.arguments_match = found.calls == 1 &&
        common_json::parse_no_throw(parsed.tool_calls[0].arguments) ==
        common_json{{"code", "B7"}};
  } catch (...) {
    // Parser exceptions are deliberately not printed: they may contain input.
  }
  return found;
}
static std::string read_file(const char *path) {
  std::ifstream file(path);
  assert(file.good());
  return std::string(std::istreambuf_iterator<char>(file), {});
}
static void tagged_template_probe() {
  const auto *override_path = std::getenv("NEXA_TEST_TOOL_TEMPLATE");
  const auto source = read_file(override_path ? override_path : AIR_TOOL_TAGGED_TEMPLATE);
  auto templates = common_chat_templates_init(nullptr, source, "<|endoftext|>", "<|im_end|>");
  common_chat_templates_inputs input;
  input.messages = {{"user", "Look up B7"}};
  input.tools = {{"lookup_test_color", "A test lookup", R"({"type":"object","properties":{"code":{"type":"string"}},"required":["code"]})"}};
  input.enable_thinking = false;
  input.reasoning_format = COMMON_REASONING_FORMAT_AUTO;
  input.chat_template_kwargs["enable_thinking"] = "false";
  const auto calls = common_chat_templates_apply(templates.get(), input);
  input.tool_choice = COMMON_CHAT_TOOL_CHOICE_NONE;
  const auto text = common_chat_templates_apply(templates.get(), input);
  common_chat_template tmpl(source, "<|endoftext|>", "<|im_end|>");
  air_tool_parser parser(tmpl, calls, text);
  const std::string valid = "<tool_call>\n<function=lookup_test_color>\n<parameter=code>\nB7\n</parameter>\n</function>\n</tool_call>";
  const auto parsed = parser.parse(valid);
  assert(parsed.tool_calls.size() == 1);
  assert(parsed.tool_calls[0].name == "lookup_test_color");
  assert(common_json::parse(parsed.tool_calls[0].arguments) == common_json({{"code", "B7"}}));
  assert(parser.parse("Hello, 蓝色 🌈").content == "Hello, 蓝色 🌈");
  // Typed AST conversion must retain original JSON values and literal strings.
  input.tools = {{"write_test", "Synthetic typed values", R"({"type":"object","properties":{"text":{"type":"string"},"data":{"type":"object"},"count":{"type":"integer"}},"required":["text","data","count"]})"}};
  input.tool_choice = COMMON_CHAT_TOOL_CHOICE_AUTO;
  const auto typed_calls = common_chat_templates_apply(templates.get(), input);
  input.tool_choice = COMMON_CHAT_TOOL_CHOICE_NONE;
  const auto typed_text = common_chat_templates_apply(templates.get(), input);
  air_tool_parser typed(tmpl, typed_calls, typed_text);
  const std::string typed_raw = "<tool_call>\n<function=write_test>\n<parameter=text>\nline \"one\"\n蓝色\n</parameter>\n<parameter=data>\n{\"nested\":[true,null,3]}\n</parameter>\n<parameter=count>\n2\n</parameter>\n</function>\n</tool_call>";
  const auto typed_result = typed.parse(typed_raw);
  assert(typed_result.tool_calls.size() == 1);
  const auto typed_args = common_json::parse(typed_result.tool_calls[0].arguments);
  assert(typed_args.at("text") == "line \"one\"\n蓝色");
  assert(typed_args.at("data") == common_json::parse(R"({"nested":[true,null,3]})"));
  assert(typed_args.at("count") == 2);

  for (const auto &bad : {valid.substr(0, valid.find("</parameter>")),
                          valid.substr(0, valid.find("</tool_call>")),
                          valid + "\n<tool_call>", std::string("Hello <tool_")}) {
    bool rejected = false;
    try { (void)parser.parse(bad); } catch (const failure &) { rejected = true; }
    assert(rejected);
  }
}
static void run() {
  tagged_template_probe();
  const auto fixture = common_json::parse(read_file(AIR_TOOL_FIXTURE));
  assert(fixture.at("origin") == "synthetic-design");
  const auto source = read_file(AIR_TOOL_TEMPLATE);
  auto templates = common_chat_templates_init(nullptr, source, "<|endoftext|>", "<|im_end|>");
  common_chat_templates_inputs input;
  input.messages = common_chat_msgs_parse_oaicompat(fixture.at("first_request").at("messages"));
  input.tools = common_chat_tools_parse_oaicompat(fixture.at("first_request").at("tools"));
  input.enable_thinking = false;
  input.reasoning_format = COMMON_REASONING_FORMAT_NONE;
  input.chat_template_kwargs["enable_thinking"] = "false";
  // The production candidate extracts empty prefills through upstream reasoning tags.
  auto strict_input = input;
  strict_input.reasoning_format = COMMON_REASONING_FORMAT_AUTO;
  const auto strict_calls = common_chat_templates_apply(templates.get(), strict_input);
  strict_input.tool_choice = COMMON_CHAT_TOOL_CHOICE_NONE;
  const auto strict_text = common_chat_templates_apply(templates.get(), strict_input);
  common_chat_template strict_template(source, "<|endoftext|>", "<|im_end|>");
  air_tool_parser strict_parser(strict_template, strict_calls, strict_text);
  auto normal = strict_parser.parse("Hello, 蓝色 🌈");
  assert(normal.content == "Hello, 蓝色 🌈" && normal.tool_calls.empty());
  assert(normal.reasoning_content.empty());
  const auto applied = common_chat_templates_apply(templates.get(), input);
  assert(!applied.parser.empty());
  assert(applied.format != COMMON_CHAT_FORMAT_CONTENT_ONLY);
  assert(applied.prompt.find("lookup_test_color") != std::string::npos);
  assert(applied.prompt.find("additionalProperties") != std::string::npos);

  common_chat_parser_params params(applied);
  assert(params.parser.empty());
  const std::string valid = "<tool_call>\n{\"name\":\"lookup_test_color\",\"arguments\":{\"code\":\"B7\"}}\n</tool_call>";
  const auto strict_valid = strict_parser.parse(valid);
  assert(strict_valid.tool_calls.size() == 1 && strict_valid.content.empty());
  assert(strict_valid.tool_calls[0].arguments == "{\"code\":\"B7\"}");
  for (const auto &bad : {valid.substr(0, valid.find("</tool_call>")),
                          valid + "\n<tool_call>", std::string("Hello <tool_")}) {
    bool rejected = false;
    try { (void)strict_parser.parse(bad); } catch (const failure &) { rejected = true; }
    assert(rejected);
  }
  // An uninitialised parser silently maps tool syntax to ordinary content.
  const auto fallback = common_chat_parse(valid, false, params);
  assert(fallback.tool_calls.empty());
  assert(fallback.content.find("<tool_call>") != std::string::npos);
  params.parser.load(applied.parser);
  assert(!params.parser.empty());
  const auto parsed = common_chat_parse(valid, false, params);
  assert(parsed.tool_calls.size() == 1);
  assert(parsed.tool_calls[0].name == "lookup_test_color");
  assert(common_json::parse(parsed.tool_calls[0].arguments) == common_json({{"code", "B7"}}));
  const auto text = common_chat_parse("Hello, 蓝色 🌈", false, params);
  // With the explicit NONE format, this pinned parser exposes the empty
  // thinking prefix. Characterize it; do not strip or accept it as production.
  assert(text.tool_calls.empty() && text.reasoning_content.empty());
  assert(text.content == "<think>\n\n</think>\n\nHello, 蓝色 🌈");

  input.messages = common_chat_msgs_parse_oaicompat(fixture.at("second_request").at("messages"));
  assert(input.messages.size() == 4);
  assert(input.messages[2].tool_calls.size() == 1);
  assert(input.messages[2].tool_calls[0].id == input.messages[3].tool_call_id);
  assert(input.messages[3].role == "tool");
  const auto second = common_chat_templates_apply(templates.get(), input);
  assert(second.prompt.find("<tool_call>") != std::string::npos);
  assert(second.prompt.find("<tool_response>\n" + input.messages[3].content + "\n</tool_response>") != std::string::npos);
  assert(second.prompt.find("fixture-b7-v1") != std::string::npos);
  assert(second.prompt.find("蓝色") != std::string::npos);

  struct test_case {
    const char *name;
    std::string raw;
    observation expected;
  };
  // These are locked-upstream observations, NOT an acceptance allowlist.
  const std::vector<test_case> cases = {
      {"valid", valid, {true, true, 1, true}},
      {"missing_close_tag", valid.substr(0, valid.find("</tool_call>")), {false, true, 1, true}},
      {"truncated_arguments", "<tool_call>\n{\"name\":\"lookup_test_color\",\"arguments\":{\"code\":\"B7\"", {false, true, 1, false}},
      {"bad_json", "<tool_call>\n{\"name\":\"lookup_test_color\",\"arguments\":{\"code\":}}\n</tool_call>", {false, false, 0, false}},
      {"trailing_structure", valid + "\n<tool_call>", {false, true, 1, true}},
      {"unknown_tool", "<tool_call>\n{\"name\":\"unknown_tool\",\"arguments\":{\"code\":\"B7\"}}\n</tool_call>", {false, false, 0, false}},
      {"two_calls", valid + "\n" + valid, {true, true, 2, false}},
      {"extra_property", "<tool_call>\n{\"name\":\"lookup_test_color\",\"arguments\":{\"code\":\"B7\",\"extra\":true}}\n</tool_call>", {true, true, 1, false}},
      {"duplicate_key", "<tool_call>\n{\"name\":\"lookup_test_color\",\"arguments\":{\"code\":\"A1\",\"code\":\"B7\"}}\n</tool_call>", {true, true, 1, true}},
      {"invalid_enum", "<tool_call>\n{\"name\":\"lookup_test_color\",\"arguments\":{\"code\":\"Z0\"}}\n</tool_call>", {true, true, 1, false}},
      {"utf8_text", "Hello, 蓝色 🌈", {false, true, 0, false}},
      {"canary_error", "<tool_call>\n{\"name\":\"lookup_test_color\",\"arguments\":{\"code\":,\"canary\":\"NEXA_TOOL_PRIVATE_CANARY_26f074\"}}\n</tool_call>", {false, false, 0, false}},
      {"canary_text", "NEXA_TOOL_PRIVATE_CANARY_26f074", {false, true, 0, false}},
  };
  assert(cases.size() == 13);
  for (const auto &item : cases) {
    const auto found = observe(params, item.raw);
    std::cout << item.name << ": strict_complete=" << found.strict_complete
              << " final_mapped=" << found.mapped << " calls=" << found.calls
              << " expected_arguments=" << found.arguments_match << '\n';
    assert(found.strict_complete == item.expected.strict_complete);
    assert(found.mapped == item.expected.mapped);
    assert(found.calls == item.expected.calls);
    assert(found.arguments_match == item.expected.arguments_match);
  }
  std::cout << "synthetic template/parser diagnostics only; no tool acceptance or inference proved\n";
}
int main() {
  // Exercise the current production log setup before using common/template code.
  common_log_set_verbosity_thold(LOG_LEVEL_TRACE);
  jinja::enable_debug(true);
  air_engine *engine = nullptr;
  air_error error{};
  assert(air_engine_create(&engine, &error) == 0 && engine);
  assert(common_log_get_verbosity_thold() == -1 && !g_jinja_debug);
  try { run(); }
  catch (...) {
    air_engine_destroy(engine);
    std::cerr << "tool parser diagnostic failed (details suppressed)\n";
    return 1;
  }
  air_engine_destroy(engine);
}
