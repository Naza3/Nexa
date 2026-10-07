#include "ocr_template.h"
#include <cassert>
#include <iostream>

int main() {
  const std::string source = "[gMASK]<sop><|user|>\n{{ messages[0].content }}<|assistant|>\n";
  air_ocr_template tmpl(source, "", "<|endoftext|>");
  for (const std::string content : {"<__media__>Text Recognition:", "Text Recognition:<__media__>", "中文<__media__>"})
    assert(tmpl.render(content) == "[gMASK]<sop><|user|>\n" + content + "<|assistant|>");
  for (const auto &bad : {std::string("{{ messages[0].content }}"), source + "<think>", std::string("[gMASK]<sop><|user|>\nignored<|assistant|>\n")}) {
    bool rejected = false;
    try { air_ocr_template invalid(bad, "", ""); }
    catch (const failure &error) { rejected = error.code == 4; }
    assert(rejected);
  }
  // No assistant EOS: OCR contract permits this, the ordinary chat contract does not.
  bool text_rejected = false;
  try { air_text_template text(source, "", "", false, false, [](const std::string &) { return false; }); }
  catch (const failure &error) { text_rejected = error.code == 4; }
  assert(text_rejected);
  std::cout << "OCR template contract passed\n";
}
