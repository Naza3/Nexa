// Derived from native/llama-shim/src/stream_buffer.h at Nexa c1114ee.
// Independent copy preserves the Windows implementation and UTF-8/stop goldens.
#pragma once
#include <algorithm>
#include <stdexcept>
#include <string>
#include <vector>
struct failure : std::runtime_error {
  int32_t code;
  failure(int32_t c, const char *m) : std::runtime_error(m), code(c) {}
};
// Returns the complete UTF-8 prefix. Invalid byte sequences fail, incomplete
// suffix waits.
static size_t utf8_prefix(const std::string &s) {
  size_t i = 0;
  while (i < s.size()) {
    const auto c = static_cast<unsigned char>(s[i]);
    size_t n = c < 0x80 ? 1
                        : (c >= 0xc2 && c <= 0xdf
                               ? 2
                               : (c >= 0xe0 && c <= 0xef
                                      ? 3
                                      : (c >= 0xf0 && c <= 0xf4 ? 4 : 0)));
    if (!n)
      throw failure(6, "invalid UTF-8 from tokenizer");
    if (i + n > s.size())
      break;
    for (size_t j = 1; j < n; j++)
      if ((static_cast<unsigned char>(s[i + j]) & 0xc0) != 0x80)
        throw failure(6, "invalid UTF-8 from tokenizer");
    if (n >= 3) {
      auto b = static_cast<unsigned char>(s[i + 1]);
      if ((c == 0xe0 && b < 0xa0) || (c == 0xed && b >= 0xa0) ||
          (c == 0xf0 && b < 0x90) || (c == 0xf4 && b >= 0x90))
        throw failure(6, "invalid UTF-8 from tokenizer");
    }
    i += n;
  }
  return i;
}

class air_stream_buffer {
  std::string pending;
  std::vector<std::string> stops;
  bool did_stop = false;

public:
  explicit air_stream_buffer(std::vector<std::string> s)
      : stops(std::move(s)) {}
  bool stopped() const { return did_stop; }
  void push(const std::string &piece) {
    if (piece.size() > 1048576)
      throw failure(6, "token piece exceeds byte limit");
    pending += piece;
  }
  template <class Emit> void flush(bool final, Emit emit) {
    size_t ready = pending.size();
    size_t pos = std::string::npos;
    for (const auto &s : stops)
      pos = std::min(pos, pending.find(s));
    if (pos != std::string::npos) {
      ready = pos;
      did_stop = true;
    } else if (!final) {
      for (const auto &s : stops)
        for (size_t n = 1; n < s.size() && n <= pending.size(); n++)
          if (pending.compare(pending.size() - n, n, s, 0, n) == 0)
            ready = std::min(ready, pending.size() - n);
    }
    auto prefix = pending.substr(0, ready);
    size_t valid = utf8_prefix(prefix);
    if ((final || did_stop) && valid != prefix.size())
      throw failure(6, "incomplete generated UTF-8");
    ready = valid;
    size_t offset = 0;
    while (offset < ready) {
      size_t n = std::min<size_t>(4096, ready - offset);
      while (n && offset + n < ready &&
             (static_cast<unsigned char>(pending[offset + n]) & 0xc0) == 0x80)
        --n;
      emit(pending.substr(offset, n));
      offset += n;
    }
    pending.erase(0, ready);
    if (did_stop)
      pending.clear();
  }
};
