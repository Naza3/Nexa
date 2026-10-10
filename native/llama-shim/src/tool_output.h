#pragma once
#include "chat.h"
#include "stream_buffer.h"
#include <cstddef>
#include <string>
#include <vector>

constexpr size_t air_tool_output_bytes = 65536;
constexpr size_t air_tool_max_calls = 16;
// capacity() excludes the terminator. Allow SSO/allocator rounding separately
// from payload bytes: MSVC rounds char capacity to (requested | 15), including
// in shrink_to_fit(); neither reserve nor copy construction promises exactness.
constexpr size_t air_tool_string_overhead = 32;
constexpr size_t air_tool_raw_storage = air_tool_output_bytes + air_tool_string_overhead;
constexpr size_t air_tool_normalized_strings = 1 + 3 * air_tool_max_calls;
constexpr size_t air_tool_normalized_string_storage =
    air_tool_output_bytes + air_tool_normalized_strings * air_tool_string_overhead;
constexpr size_t air_tool_normalized_metadata = 4096;

inline void air_check_tool_string_capacity(const std::string &value, size_t payload_limit) {
  if (payload_limit > air_tool_output_bytes || value.size() > payload_limit ||
      value.capacity() >= payload_limit + air_tool_string_overhead)
    throw failure(11, "tool output string capacity exceeds budget");
}
inline void air_reserve_tool_output(std::string &value) {
  value.reserve(air_tool_output_bytes);
  air_check_tool_string_capacity(value, air_tool_output_bytes);
}
inline void air_append_tool_output(std::string &value, const std::string &chunk) {
  if (value.size() > air_tool_output_bytes || chunk.size() > air_tool_output_bytes - value.size())
    throw failure(11, "tool output byte limit exceeded");
  value += chunk;
  air_check_tool_string_capacity(value, air_tool_output_bytes);
}

// Retain only the published fields. The upstream message (including role,
// reasoning/content_parts, model IDs and any spare capacities) dies before any
// callback can create the Rust/IPC/actor output copies.
struct air_tool_output {
  std::string content;
  std::vector<common_chat_tool_call> tool_calls;

  size_t string_storage() const {
    size_t total = content.capacity() + 1;
    for (const auto &call : tool_calls)
      total += call.name.capacity() + 1 + call.arguments.capacity() + 1 + call.id.capacity() + 1;
    return total;
  }
  size_t metadata_storage() const {
    return sizeof(*this) + tool_calls.capacity() * sizeof(common_chat_tool_call);
  }
};
static_assert(sizeof(air_tool_output) + air_tool_max_calls * sizeof(common_chat_tool_call) <=
              air_tool_normalized_metadata, "tool output metadata exceeds reservation");

inline air_tool_output air_normalize_tool_output(common_chat_msg source) {
  if (source.content.size() > 8192 || source.tool_calls.size() > air_tool_max_calls)
    throw failure(11, "normalized tool output limit exceeded");
  auto compact = [](const std::string &value) {
    std::string result(value.data(), value.size());
    air_check_tool_string_capacity(result, result.size());
    return result;
  };
  air_tool_output result;
  result.content = compact(source.content);
  result.tool_calls.reserve(source.tool_calls.size());
  if (result.tool_calls.capacity() > air_tool_max_calls)
    throw failure(11, "normalized tool call capacity exceeds budget");
  size_t total = result.content.size();
  for (const auto &call : source.tool_calls) {
    if (call.name.empty() || call.name.size() > 64 || call.arguments.size() > 16384 ||
        call.name.size() + call.arguments.size() > air_tool_output_bytes - total)
      throw failure(11, "normalized tool call limit exceeded");
    total += call.name.size() + call.arguments.size();
    result.tool_calls.push_back({compact(call.name), compact(call.arguments), {}});
  }
  // Check the actual retained capacities after all moves, including empty IDs
  // and SSO storage. Never charge len as if it were allocation capacity.
  air_check_tool_string_capacity(result.content, result.content.size());
  for (const auto &call : result.tool_calls) {
    air_check_tool_string_capacity(call.name, call.name.size());
    air_check_tool_string_capacity(call.arguments, call.arguments.size());
    air_check_tool_string_capacity(call.id, 0);
  }
  if (result.string_storage() > air_tool_normalized_string_storage ||
      result.metadata_storage() > air_tool_normalized_metadata)
    throw failure(11, "normalized tool output capacity exceeds budget");
  return result;
}
