#include "../src/tool_output.h"
#include <cassert>
#include <iostream>
#include <utility>

template <class F> static void limit_exceeded(F operation) {
  bool rejected = false;
  try { operation(); }
  catch (const failure &error) { assert(error.code == 11); rejected = true; }
  assert(rejected);
}

static std::string arguments(size_t bytes) {
  assert(bytes >= 8);
  return "{\"x\":\"" + std::string(bytes - 8, 'x') + "\"}";
}
static common_chat_msg maximum_message(bool with_content) {
  common_chat_msg message;
  if (with_content) message.content.assign(8192, 'x');
  // Both layouts use all 16 calls and exactly 64 KiB of published payload.
  // Every name/arguments length is a multiple of 16: MSVC rounding plus empty
  // ID/content SSO made the previous capacity <= payload-limit guard reject it.
  for (size_t i = 0; i < air_tool_max_calls; ++i)
    message.tool_calls.push_back({std::string(64, 'n'), arguments(with_content ? 3520 : 4032), "discarded"});
  return message;
}
static void verify(const air_tool_output &message, size_t expected_payload) {
  size_t payload = message.content.size();
  assert(message.content.size() <= 8192);
  assert(message.tool_calls.size() <= air_tool_max_calls);
  assert(message.tool_calls.capacity() <= air_tool_max_calls);
  air_check_tool_string_capacity(message.content, message.content.size());
  for (const auto &call : message.tool_calls) {
    assert(call.id.empty());
    assert(!call.name.empty() && call.name.size() <= 64);
    assert(call.arguments.size() <= 16384);
    air_check_tool_string_capacity(call.name, call.name.size());
    air_check_tool_string_capacity(call.arguments, call.arguments.size());
    air_check_tool_string_capacity(call.id, 0);
    payload += call.name.size() + call.arguments.size();
  }
  assert(payload == expected_payload);
  assert(message.string_storage() <= air_tool_normalized_string_storage);
  assert(message.metadata_storage() <= air_tool_normalized_metadata);
}

int main() {
  std::string raw;
  air_reserve_tool_output(raw);
  assert(raw.capacity() >= air_tool_output_bytes);
  assert(raw.capacity() + 1 <= air_tool_raw_storage);
  std::cout << "raw_capacity=" << raw.capacity() << " raw_storage_limit=" << air_tool_raw_storage << '\n';
  for (size_t bytes : {size_t(1), size_t(15), size_t(16), size_t(4096), size_t(61408)})
    air_append_tool_output(raw, std::string(bytes, 'x'));
  assert(raw.size() == air_tool_output_bytes);
  limit_exceeded([&] { air_append_tool_output(raw, "x"); });
  assert(raw.size() == air_tool_output_bytes);

  // Reproduce the rounded retained capacity on non-MSVC hosts too. A payload
  // at the public limit remains legal with 15 spare bytes, but not unbounded
  // reserve space. Both checks inspect capacity after shrinking only the size.
  std::string rounded(air_tool_output_bytes + 15, 'x');
  rounded.resize(air_tool_output_bytes);
  assert(rounded.capacity() > air_tool_output_bytes);
  air_check_tool_string_capacity(rounded, air_tool_output_bytes);
  std::string excessive(air_tool_raw_storage, 'x');
  excessive.resize(air_tool_output_bytes);
  limit_exceeded([&] { air_check_tool_string_capacity(excessive, air_tool_output_bytes); });

  // Check actual host STL capacities at SSO/allocation boundaries, including
  // fresh construction and shrink_to_fit. Neither is assumed to be exact.
  for (size_t bytes : {size_t(0), size_t(15), size_t(16), size_t(31), size_t(32),
                       size_t(64), size_t(8192), size_t(16384), size_t(65536)}) {
    std::string value(bytes, 'x');
    air_check_tool_string_capacity(value, bytes);
    value.reserve(2 * air_tool_output_bytes);
    limit_exceeded([&] { air_check_tool_string_capacity(value, bytes); });
    value.shrink_to_fit();
    // shrink_to_fit is non-binding; production uses a fresh, checked copy.
    std::string compact(value.data(), value.size());
    air_check_tool_string_capacity(compact, bytes);
  }

  for (bool with_content : {false, true}) {
    auto source = maximum_message(with_content);
    source.content.reserve(2 * air_tool_output_bytes);
    source.tool_calls.reserve(128);
    for (auto &call : source.tool_calls) {
      call.name.reserve(2 * air_tool_output_bytes);
      call.arguments.reserve(2 * air_tool_output_bytes);
      call.id.assign(8192, 'i');
    }
    // These upstream-only fields must not survive the normalization boundary.
    source.role.assign(8192, 'r');
    source.reasoning_content.reserve(2 * air_tool_output_bytes);
    source.tool_name.assign(8192, 'n');
    source.tool_call_id.assign(8192, 'i');
    source.content_parts.push_back({"text", std::string(8192, 'p')});
    const auto normalized = air_normalize_tool_output(std::move(source));
    verify(normalized, air_tool_output_bytes);
    assert(normalized.content.size() == (with_content ? 8192 : 0));
    std::cout << "normalized_string_storage=" << normalized.string_storage()
              << " normalized_metadata=" << normalized.metadata_storage() << '\n';
  }
  {
    common_chat_msg source;
    source.tool_calls.push_back({std::string(64, 'n'), arguments(16384), "discarded"});
    verify(air_normalize_tool_output(std::move(source)), 64 + 16384);
  }
  for (int which = 0; which < 5; ++which) {
    auto source = maximum_message(false);
    if (which == 0) source.content = "x"; // 64 KiB aggregate + 1
    if (which == 1) source.content.assign(8193, 'x');
    if (which == 2) source.tool_calls[0].name += 'n';
    if (which == 3) source.tool_calls[0].arguments = arguments(16385);
    if (which == 4) source.tool_calls.push_back({"name", "{}", {}});
    limit_exceeded([&] { (void)air_normalize_tool_output(std::move(source)); });
  }
  {
    air_stream_buffer stream({}, air_tool_output_bytes, air_tool_raw_storage);
    assert(stream.retained_capacity() + 1 <= air_tool_raw_storage);
    stream.push(raw);
    limit_exceeded([&] { stream.push("x"); });
    std::string emitted;
    air_reserve_tool_output(emitted);
    stream.flush(true, [&](const std::string &chunk) { air_append_tool_output(emitted, chunk); });
    assert(emitted == raw);
    // erase() retains capacity; release() is required before publication.
    assert(stream.retained_capacity() >= air_tool_output_bytes);
    stream.release();
    assert(stream.retained_capacity() < air_tool_string_overhead);
  }
  std::cout << "tool output capacity and exact payload boundary tests passed\n";
}
