#pragma once
#include "chat.h"
#include "chat-auto-parser.h"
#include "chat-peg-parser.h"
#include "stream_buffer.h"
#include <algorithm>
#include <set>

// The accepted format is determined from the embedded template, never its name.
class air_tool_parser {
  common_chat_parser_params calls;
  common_chat_parser_params text;
  std::vector<common_peg_arena> text_guards;
  bool tagged_arguments = false;

  static bool complete(const common_peg_parse_context &ctx,
                       const common_peg_parse_result &result) {
    if (!result.success() || result.end != ctx.input.size() ||
        !result.invalid_utf8.empty()) return false;
    bool valid = true;
    ctx.ast.visit(result, [&](const common_peg_ast_node &node) {
      if (node.is_partial || !node.invalid_utf8.empty()) valid = false;
    });
    return valid;
  }
  static common_chat_msg mapped(const common_peg_parse_context &ctx,
                                const common_peg_parse_result &result,
                                bool tagged_arguments) {
    common_chat_msg message;
    common_chat_peg_mapper mapper(message);
    mapper.from_ast(ctx.ast, result);
    if (!message.reasoning_content.empty())
      throw failure(9, "reasoning output is unsupported in tool mode");
    std::vector<std::string> arguments;
    ctx.ast.visit(result, [&](const common_peg_ast_node &node) {
      if (node.tag == common_chat_peg_builder::TOOL_ARGS)
        arguments.emplace_back(node.text);
    });
    if (arguments.size() != message.tool_calls.size())
      throw failure(9, "tool arguments lack original JSON boundaries");
    // Tagged parameter formats expose typed AST values. Rebuild JSON from
    // those complete original spans, without the partial mapper's repairs.
    std::vector<std::string> tagged;
    std::string argument_name;
    std::set<std::string> names;
    bool open = false;
    bool has_value = false;
    ctx.ast.visit(result, [&](const common_peg_ast_node &node) {
      if (node.tag == common_chat_peg_builder::TOOL_OPEN) {
        if (open) throw failure(9, "nested tool boundary");
        tagged.emplace_back("{");
        names.clear();
        argument_name.clear();
        open = true;
        has_value = false;
      } else if (node.tag == common_chat_peg_builder::TOOL_ARG_NAME) {
        if (!open || !argument_name.empty() || node.text.empty())
          throw failure(9, "invalid tool argument boundary");
        argument_name = std::string(node.text);
        if (!names.insert(argument_name).second) throw failure(9, "duplicate tool argument name");
      } else if (node.tag == common_chat_peg_builder::TOOL_ARG_VALUE ||
                 node.tag == common_chat_peg_builder::TOOL_ARG_STRING_VALUE) {
        if (!open || argument_name.empty()) throw failure(9, "tool value has no argument name");
        std::string value(node.text);
        if (node.tag == common_chat_peg_builder::TOOL_ARG_STRING_VALUE) {
          value = common_json(value).dump();
        } else if (common_json::parse_no_throw(value).is_discarded()) {
          throw failure(9, "tool value is not complete JSON");
        }
        auto &json = tagged.back();
        if (has_value) json += ",";
        json += common_json(argument_name).dump() + ":" + value;
        if (json.size() > 16384) throw failure(11, "normalized tool arguments exceed limit");
        has_value = true;
        argument_name.clear();
      } else if (node.tag == common_chat_peg_builder::TOOL_CLOSE) {
        if (!open || !argument_name.empty()) throw failure(9, "incomplete tool argument");
        tagged.back() += "}";
        open = false;
      }
    });
    if (open || tagged.size() != arguments.size()) throw failure(9, "incomplete tool boundaries");
    for (size_t i = 0; i < arguments.size(); ++i) {
      const auto &raw = arguments[i];
      if (common_json::parse_no_throw(raw).is_object()) {
        const auto &normalized = message.tool_calls[i].arguments;
        const auto end = raw.find_last_not_of(" \t\r\n");
        if (end == std::string::npos || normalized != raw.substr(0, end + 1))
          throw failure(9, "tool arguments require unsupported normalization");
        message.tool_calls[i].arguments = raw;
      } else {
        if (!tagged_arguments) throw failure(9, "tool arguments are not a JSON object");
        // No model-specific strings are inspected: only upstream typed nodes.
        message.tool_calls[i].arguments = std::move(tagged[i]);
      }
    }
    return message;
  }
public:
  air_tool_parser(const common_chat_template &tmpl,
                  const common_chat_params &call_params,
                  const common_chat_params &text_params) : calls(call_params), text(text_params) {
    autoparser::autoparser analysis;
    analysis.analyze_template(tmpl);
    const auto &format = analysis.tools.format;
    tagged_arguments = format.mode == autoparser::tool_format::TAG_WITH_TAGGED;
    if ((format.mode != autoparser::tool_format::JSON_NATIVE &&
         format.mode != autoparser::tool_format::TAG_WITH_JSON &&
         format.mode != autoparser::tool_format::TAG_WITH_TAGGED) ||
        call_params.format != COMMON_CHAT_FORMAT_PEG_NATIVE ||
        text_params.format != COMMON_CHAT_FORMAT_PEG_NATIVE ||
        call_params.parser.empty() || text_params.parser.empty() ||
        call_params.generation_prompt != text_params.generation_prompt)
      throw failure(4, "template has no supported strict JSON tool parser");
    std::vector<std::string> markers;
    for (const auto &marker : {format.section_start, format.per_call_start,
                               format.section_end, format.per_call_end}) {
      if (!marker.empty() && std::find(markers.begin(), markers.end(), marker) == markers.end())
        markers.push_back(marker);
    }
    if (format.section_start.empty() && format.per_call_start.empty())
      throw failure(4, "template has no unambiguous tool boundary");
    calls.parser.load(call_params.parser);
    text.parser.load(text_params.parser);
    for (const auto &marker : markers) {
      text_guards.push_back(build_chat_peg_parser([&](common_chat_peg_builder &p) {
        return p.until(marker) + p.end();
      }));
    }
  }
  common_chat_msg parse(const std::string &raw) const {
    common_peg_parse_context call_ctx(calls.generation_prompt + raw, COMMON_PEG_PARSE_FLAG_NONE);
    const auto call_result = calls.parser.parse(call_ctx);
    if (complete(call_ctx, call_result)) {
      auto result = mapped(call_ctx, call_result, tagged_arguments);
      if (!result.tool_calls.empty()) return result;
    }
    // A complete or partially emitted tool marker can never fall back to text.
    for (const auto &guard : text_guards) {
      common_peg_parse_context guard_ctx(raw, COMMON_PEG_PARSE_FLAG_NONE);
      const auto guard_result = guard.parse(guard_ctx);
      if (!complete(guard_ctx, guard_result))
        throw failure(9, "incomplete or invalid tool output");
    }
    common_peg_parse_context text_ctx(text.generation_prompt + raw, COMMON_PEG_PARSE_FLAG_NONE);
    const auto text_result = text.parser.parse(text_ctx);
    if (!complete(text_ctx, text_result)) throw failure(9, "invalid assistant output framing");
    auto result = mapped(text_ctx, text_result, false);
    if (!result.tool_calls.empty()) throw failure(9, "invalid text parser result");
    return result;
  }
};
