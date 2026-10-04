#include "../src/stream_buffer.h"
#include <cstdlib>
#include <iostream>
static void require(bool ok) {
  if (!ok)
    std::abort();
}
int main() {
  for (const std::string &text :
       {std::string("中文🙂abc"), std::string("ascii"), std::string("é𝄞")}) {
    for (size_t split = 0; split <= text.size(); ++split) {
      air_stream_buffer b({});
      std::string out;
      auto emit = [&](const std::string &s) {
        require(utf8_prefix(s) == s.size());
        require(s.size() <= 4096);
        out += s;
      };
      b.push(text.substr(0, split));
      b.flush(false, emit);
      b.push(text.substr(split));
      b.flush(true, emit);
      require(out == text);
    }
  }
  for (const std::string &stop : {std::string("STOP"), std::string("结束🙂")}) {
    const auto text = std::string("hello中") + stop + "never";
    for (bool final : {false, true}) {
      for (size_t split = 0; split <= text.size(); split++) {
        air_stream_buffer b({stop});
        std::string out;
        auto emit = [&](const std::string &s) {
          require(utf8_prefix(s) == s.size());
          out += s;
        };
        b.push(text.substr(0, split));
        b.flush(false, emit);
        if (!b.stopped()) {
          b.push(text.substr(split));
          b.flush(final, emit);
        }
        require(b.stopped());
        require(out == "hello中");
      }
    }
  }
  // A matched stop is a terminal boundary, even during a non-final flush.
  // Cover every truncated length of 2/3/4-byte UTF-8 and every piece boundary.
  for (const std::string &character :
       {std::string("é"), std::string("中"), std::string("🙂")}) {
    for (size_t length = 1; length < character.size(); ++length) {
      const auto incomplete = character.substr(0, length);
      {
        // Non-terminal partial UTF-8 remains buffered. Cancellation can discard
        // the buffer without finalizing; this helper has no cancellation API.
        air_stream_buffer b({"STOP"});
        std::string out;
        b.push(std::string("hello中") + incomplete);
        b.flush(false, [&](const std::string &s) { out += s; });
        require(out == "hello中");
        require(!b.stopped());
      }
      for (const std::string &prefix : {std::string(), std::string("hello中")}) {
        for (const std::string &stop :
             {std::string("STOP"), std::string("结束🙂")}) {
          const auto text = prefix + incomplete + stop + "never";
          for (bool final : {false, true}) {
            for (size_t split = 0; split <= text.size(); ++split) {
              air_stream_buffer b({stop});
              std::string out;
              auto emit = [&](const std::string &s) {
                require(utf8_prefix(s) == s.size());
                out += s;
              };
              bool caught = false;
              try {
                b.push(text.substr(0, split));
                b.flush(false, emit);
                if (!b.stopped()) {
                  b.push(text.substr(split));
                  b.flush(final, emit);
                }
              } catch (const failure &e) {
                require(e.code == 6);
                caught = true;
              }
              require(caught);
              require(prefix.compare(0, out.size(), out) == 0);
            }
          }
        }
      }
    }
  }
  {
    air_stream_buffer b({"STOP"});
    std::string out;
    auto emit = [&](const std::string &s) { out += s; };
    b.push("xST");
    b.flush(false, emit);
    require(out == "x");
    b.flush(true, emit);
    require(out == "xST");
  }
  {
    air_stream_buffer b({"aba", "ab"});
    std::string out;
    b.push("xxaba");
    b.flush(false, [&](const std::string &s) { out += s; });
    require(out == "xx");
    require(b.stopped());
  }
  {
    air_stream_buffer b({});
    std::string input;
    for (int i = 0; i < 5000; i++)
      input += "中";
    std::string out;
    b.push(input);
    b.flush(true, [&](const std::string &s) {
      require(s.size() <= 4096);
      require(utf8_prefix(s) == s.size());
      out += s;
    });
    require(out == input);
  }
  for (const std::string &bad :
       {std::string("\xff"), std::string("\xed\xa0\x80"),
        std::string("\xc0\x80"), std::string("\xf4\x90\x80\x80"),
        std::string("\xe4\xb8")}) {
    bool caught = false;
    try {
      air_stream_buffer b({});
      b.push(bad);
      b.flush(true, [](const std::string &) {});
    } catch (const failure &) {
      caught = true;
    }
    require(caught);
  }
  std::cout << "stream buffer tests passed\n";
}
