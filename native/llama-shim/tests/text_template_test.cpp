#include "text_template.h"
#include "air_llama.h"
#include "log.h"
#include <cassert>
#include <fstream>
#include <iterator>
#include <iostream>

static bool suffix(const std::string &value) {
  for (const std::string end : {"<eos>", "<|im_end|>", "<|eot_id|>"}) {
    if (value.rfind(end, 0) == 0 &&
        value.find_first_not_of(" \t\r\n", end.size()) == std::string::npos)
      return true;
  }
  return false;
}
static const std::string plain =
  "{{ bos_token }}{% for m in messages %}{{ m.role + ': ' + m.content + eos_token }}{% endfor %}"
  "{% if add_generation_prompt %}{{ 'assistant: ' }}{% endif %}";
static void rejected(const std::string &source) {
  bool failed = false;
  try { air_text_template candidate(source, "<bos>", "<eos>", false, false, suffix); }
  catch (...) { failed = true; }
  assert(failed);
}
static common_chat_msg message(const char *role, const std::string &content) {
  common_chat_msg value; value.role = role; value.content = content; return value;
}
static void privacy_canary() {
  common_log_set_verbosity_thold(LOG_LEVEL_TRACE);
  jinja::enable_debug(true);
  air_engine *engine = nullptr;
  air_error error{};
  assert(air_engine_create(&engine, &error) == 0);
  assert(engine && common_log_get_verbosity_thold() == -1 && !g_jinja_debug);
  const std::string canary = "NEXA_PRIVATE_CANARY_930d46b1";
  LOG_ERR("%s\n", canary.c_str()); // must be suppressed by actual engine setup
  rejected("{{ raise_exception('NEXA_PRIVATE_CANARY_930d46b1') }}");
  const std::string guarded =
      "{% if messages[0].content == 'NEXA_PRIVATE_CANARY_930d46b1' %}"
      "{{ raise_exception(messages[0].content) }}{% endif %}" + plain;
  air_text_template candidate(guarded, "<bos>", "<eos>", true, false, suffix);
  bool threw = false;
  try { candidate.render({message("user", canary)}); }
  catch (...) { threw = true; }
  assert(threw);
  air_text_template ordinary(plain, "<bos>", "<eos>", true, false, suffix);
  assert(ordinary.render({message("user", canary)}).find(canary) != std::string::npos);
  air_engine_destroy(engine);
  std::cout << "template privacy contract passed\n";
}
int main(int argc, char **argv) {
  if (argc == 2 && std::string(argv[1]) == "--privacy-canary") {
    privacy_canary();
    return 0;
  }
  assert(air_text_output_token_supported(LLAMA_TOKEN_ATTR_NORMAL));
  assert(!air_text_output_token_supported(LLAMA_TOKEN_ATTR_CONTROL));
  assert(!air_text_output_token_supported(LLAMA_TOKEN_ATTR_UNKNOWN));
  air_text_template ordinary(plain, "<bos>", "<eos>", true, false, suffix);
  const auto user = message("user", "hello");
  assert(ordinary.render({user}) == "user: hello<eos>assistant: ");
  auto collision = message("user", "NEXA_TEXT_PROBE_7bd30a6f_ASST_0");
  assert(ordinary.render({collision}).find(collision.content) != std::string::npos);
  assert(ordinary.render({message("system", "private instruction"), user}).find("private instruction") != std::string::npos);
  rejected(""); rejected("chatml"); rejected("{% invalid syntax");
  rejected("{% for m in messages %}{{ m.content }}{% endfor %}"); // no natural EOG
  rejected("{% for m in messages %}{{ m.role + ': ' + m.content + '</answer>' + eos_token }}{% endfor %}{% if add_generation_prompt %}assistant: {% endif %}");
  rejected("{% for m in messages %}{% if m.role != 'assistant' %}{{ m.content }}{% endif %}{% endfor %}");
  rejected("{% for m in messages|reverse %}{{ m.role + ': ' + m.content + eos_token }}{% endfor %}{% if add_generation_prompt %}assistant: {% endif %}");
  rejected("{% for m in messages %}{{ m.role + ': ' + m.content + eos_token }}{% endfor %}{% if add_generation_prompt %}assistant: <think>{% endif %}");
  bool missing_bos = false;
  try { air_text_template candidate(plain, "", "<eos>", false, false, suffix); }
  catch (const failure &e) { missing_bos = e.code == 4; }
  assert(missing_bos);
  std::string no_system = "{% for m in messages %}{% if m.role != 'system' %}{{ m.role + ': ' + m.content + eos_token }}{% endif %}{% endfor %}{% if add_generation_prompt %}assistant: {% endif %}";
  air_text_template limited(no_system, "", "<eos>", false, false, suffix);
  assert(!limited.render({user}).empty());
  bool blocked_system = false;
  try { limited.render({message("system", "must not vanish"), user}); }
  catch (const failure &e) { blocked_system = e.code == 4; }
  assert(blocked_system);

  // The exact template shipped in the pinned dependency: this proves prompt
  // equivalence, not real weights/tokenization/inference or Windows behavior.
  std::ifstream file(AIR_TEMPLATE_FIXTURE);
  assert(file.good());
  std::string qwen((std::istreambuf_iterator<char>(file)), {});
  air_text_template actual(qwen, "<|endoftext|>", "<|im_end|>", false, false, suffix);
  auto previous = common_chat_templates_init(nullptr, qwen, "<|endoftext|>", "<|im_end|>");
  for (const auto &messages : std::vector<std::vector<common_chat_msg>>{
      {user}, {message("system", "Be precise."), user},
      {user, message("assistant", "hello back"), message("user", "continue")}}) {
    common_chat_templates_inputs input;
    input.messages = messages; input.use_jinja = true;
    input.enable_thinking = false;
    input.chat_template_kwargs["enable_thinking"] = "false";
    const auto old_prompt = common_chat_templates_apply(previous.get(), input).prompt;
    assert(actual.render(messages) == old_prompt);
  }
  assert(actual.render({user}) == "<|im_start|>user\nhello<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\n");
  std::cout << "text template continuation contract and Qwen prompt equivalence passed\n";
}
