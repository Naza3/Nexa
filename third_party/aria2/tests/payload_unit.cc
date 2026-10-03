// Exercises the actual disk writer and IOFile implementation; no model copies.
#include "NexaPayloadLimit.h"
#include "DefaultDiskWriter.h"
#include "BufferedFile.h"
#include "Exception.h"
#include "console.h"
#include <cassert>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <iostream>
#include <limits>
#include <string>
using namespace aria2;
namespace {
unsigned cases = 0;
void env(const char* value) {
#ifdef _WIN32
  assert(_putenv_s("NEXA_PAYLOAD_MAX_BYTES", value ? value : "") == 0);
#else
  if (value) assert(setenv("NEXA_PAYLOAD_MAX_BYTES", value, 1) == 0);
  else assert(unsetenv("NEXA_PAYLOAD_MAX_BYTES") == 0);
#endif
}
template<class Fn> void rejected(Fn fn, const char* marker) {
  bool blocked = false;
  try { fn(); } catch (const Exception& e) { blocked = std::string(e.what()).find(marker) != std::string::npos; }
  assert(blocked); ++cases;
}
std::string line(const std::string& input, bool getsn = false) {
  FILE* fp = std::tmpfile(); assert(fp);
  assert(std::fwrite(input.data(), 1, input.size(), fp) == input.size());
  std::rewind(fp);
  BufferedFile file(fp);
  if (getsn) { char buf[32]; char* p = file.getsn(buf, sizeof(buf)); return p ? p : ""; }
  return file.getLine();
}
}
int main(int argc, char** argv) {
  global::initConsole(false);
  assert(argc == 2); const std::string filename = argv[1];
  for (const char* value : {"", "0", "3", "04", "+4", "-4", " 4", "4 ", "4\n", "4x", "17179869185", "18446744073709551615", "18446744073709551616"})
    rejected([&] { nexa::parsePayloadMaxBytes(value); }, "invalid payload byte limit");
  rejected([] { nexa::parsePayloadMaxBytes(nullptr); }, "invalid payload byte limit");
  for (const char* value : {"4", "64", "17179869184"}) { assert(nexa::parsePayloadMaxBytes(value) >= 4); ++cases; }
  env(nullptr);
  rejected([&] { DefaultDiskWriter writer(filename); }, "invalid payload byte limit");
  env("64");
  const unsigned char data[128] = {};
  {
    DefaultDiskWriter writer(filename);
    env("128"); // Existing instance retains immutable 64-byte bound.
    rejected([&] { writer.initAndOpenFile(65); }, "payload byte limit exceeded");
    writer.initAndOpenFile(64);
    assert(writer.size() == 0); ++cases;
    writer.writeData(data, 64, 0);
    assert(writer.size() == 64); ++cases;
    writer.enableMmap();
    writer.writeData(data, 4, 60); // same check precedes mmap path
    assert(writer.size() == 64); ++cases;
    for (auto offset : {INT64_C(-1), INT64_C(65), std::numeric_limits<int64_t>::max()})
      rejected([&] { writer.writeData(data, 1, offset); }, "payload byte limit exceeded");
    rejected([&] { writer.writeData(data, 2, 63); }, "payload byte limit exceeded");
    rejected([&] { writer.writeData(data, std::numeric_limits<size_t>::max(), 1); }, "payload byte limit exceeded");
    rejected([&] { writer.truncate(65); }, "payload byte limit exceeded");
    rejected([&] { writer.truncate(-1); }, "payload byte limit exceeded");
    for (bool sparse : {false, true}) {
      rejected([&] { writer.allocate(63, 2, sparse); }, "payload byte limit exceeded");
      rejected([&] { writer.allocate(0, -1, sparse); }, "payload byte limit exceeded");
      rejected([&] { writer.allocate(std::numeric_limits<int64_t>::max(), 1, sparse); }, "payload byte limit exceeded");
    }
    assert(writer.size() == 64); ++cases;
    writer.closeFile();
  }
  env("64");
  {
    DefaultDiskWriter writer(filename);
    rejected([&] { writer.openFile(65); }, "payload byte limit exceeded");
    rejected([&] { writer.openExistingFile(65); }, "payload byte limit exceeded");
    writer.openExistingFile(64);
    writer.truncate(4); assert(writer.size() == 4); ++cases;
    writer.allocate(0, 64, true); assert(writer.size() == 64); ++cases;
    writer.closeFile();
  }
  // Metadata remains a separate IOFile path, not capped by payload bytes.
  {
    BufferedFile file(filename.c_str(), BufferedFile::WRITE);
    assert(file.write(data, sizeof(data)) == sizeof(data)); ++cases;
  }
  {
    DefaultDiskWriter writer(filename);
    rejected([&] { writer.openExistingFile(); }, "payload byte limit exceeded");
  }
  std::remove(filename.c_str());
  for (bool getsn : {false, true}) {
    assert(line("", getsn).empty()); ++cases;
    assert(line("\n", getsn).empty()); ++cases;
    assert(line("hello\n", getsn) == "hello"); ++cases;
    for (const auto& input : {std::string("\0\n", 2), std::string("\0abc\n", 5)})
      rejected([&] { line(input, getsn); }, "NUL-prefixed input line rejected");
  }
  rejected([&] { line(std::string(4095, 'a') + std::string("\0\n", 2)); }, "NUL-prefixed input line rejected");
  std::cout << cases << " payload/IOFile cases passed\n";
}
