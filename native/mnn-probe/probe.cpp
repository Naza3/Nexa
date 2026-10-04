// T07-A private development executable; not a production ABI or Android Executor.
#include "llm/llm.hpp"
#include "ujson.hpp"
#include <chrono>
#include <fstream>
#include <memory>
#include <stdexcept>
#include <streambuf>

using Json = ujson::json;
using MNN::Transformer::Llm;
using MNN::Transformer::LlmStatus;
namespace {
constexpr size_t kInputBytes = 64 * 1024;
constexpr size_t kOutputBytes = 256 * 1024;
std::string read(const char* path, size_t limit) {
    std::ifstream file(path, std::ios::binary);
    if (!file) throw std::runtime_error("input_open_failed");
    std::string data(limit + 1, '\0');
    file.read(data.data(), static_cast<std::streamsize>(data.size()));
    data.resize(static_cast<size_t>(file.gcount()));
    if (data.size() > limit) throw std::runtime_error("input_too_large");
    return data;
}
Json parse(const std::string& data) {
    // ujson reports malformed syntax as null. RapidJSON also validates UTF-8 here.
    rapidjson::Document checked;
    checked.Parse<rapidjson::kParseValidateEncodingFlag>(data.data(), data.size());
    if (checked.HasParseError() || !checked.IsObject()) throw std::runtime_error("invalid_json");
    return Json::parse(data);
}
int integer(const Json& j, const char* key, int low, int high) {
    if (!j.contains(key) || !j[key].is_number_integer()) throw std::runtime_error("invalid_integer");
    const auto v = j[key].get<int64_t>();
    if (v < low || v > high) throw std::runtime_error("integer_out_of_range");
    return static_cast<int>(v);
}
void require(bool condition, const char* error) {
    if (!condition) throw std::runtime_error(error);
}
// The sink is bounded; native internal generate_str is NOT claimed to have this cap.
// On sink overflow discard later bytes, finish the bounded token loop, then fail.
class Sink final : public std::streambuf {
public:
    std::string bytes;
    bool overflowed = false;
protected:
    std::streamsize xsputn(const char* s, std::streamsize n) override {
        if (n < 0) return 0;
        const size_t available = kOutputBytes - bytes.size();
        const size_t length = static_cast<size_t>(n);
        bytes.append(s, std::min(available, length));
        overflowed |= length > available;
        return n;
    }
    int_type overflow(int_type c) override {
        if (!traits_type::eq_int_type(c, traits_type::eof())) {
            char ch = traits_type::to_char_type(c);
            xsputn(&ch, 1);
        }
        return traits_type::not_eof(c);
    }
};
Json identity() {
    Json result = Json::object();
    result["prototype"] = "T07-A";
    result["probe_report_version"] = 1;
    result["target_system"] = NEXA_TARGET_SYSTEM;
    result["target_processor"] = NEXA_TARGET_PROCESSOR;
    result["compiler"] = NEXA_COMPILER;
    result["mnn_commit"] = NEXA_MNN_COMMIT;
    result["requested_backend"] = "cpu";
    result["compiled_backend"] = "cpu";
    result["sampler"] = "load-time-greedy";
    result["per_request_sampling"] = false;
    result["thread_safe_cancel"] = false;
    result["production_executor"] = false;
    return result;
}
}
int main(int argc, char** argv) {
    if (argc == 2 && std::string(argv[1]) == "--identity") {
        std::cout << identity().dump() << '\n';
        return 0;
    }
    if (argc != 4) {
        std::cerr << "usage: nexa-mnn-probe RUN_CONFIG.json REQUEST.json RESULT.json\n";
        return 2;
    }
    try {
        auto request = parse(read(argv[2], kInputBytes));
        int limit = integer(request, "max_new_tokens", 1, 64);
        int context = integer(request, "logical_context", 1, 2048);
        require(request.contains("messages") && request["messages"].is_array(), "invalid_messages");
        require(request["messages"].size() > 0 && request["messages"].size() <= 16, "invalid_message_count");
        MNN::Transformer::ChatMessages messages;
        for (const auto& message : request["messages"]) {
            require(message.is_object() && message["role"].is_string() && message["content"].is_string(), "invalid_message");
            auto role = message["role"].get<std::string>();
            require(role == "system" || role == "user" || role == "assistant", "invalid_role");
            messages.emplace_back(role, message["content"].get<std::string>());
        }
        // All Llm resources, including destruction on failures, remain on this thread.
        auto config_input = parse(read(argv[1], 1024 * 1024));
        require(config_input["jinja"]["chat_template"].is_string(), "template_required");
        require(!config_input["jinja"]["chat_template"].get<std::string>().empty(), "template_required");
        std::unique_ptr<Llm, decltype(&Llm::destroy)> llm(Llm::createLLM(argv[1]), &Llm::destroy);
        require(llm != nullptr, "create_failed");
        auto config = parse(llm->dump_config());
        require(config.value("backend_type", "") == "cpu", "cpu_required");
        require(config.value("sampler_type", "") == "greedy", "greedy_required");
        integer(config, "thread_num", 1, 4);
        require(integer(config, "max_all_tokens", 1, 2048) == context, "context_config_mismatch");
        require(integer(config, "max_new_tokens", 1, 64) == limit, "token_config_mismatch");
        require(!config.value("async", true), "synchronous_required");
        require(config_input["jinja"]["context"].contains("enable_thinking")
            && config_input["jinja"]["context"]["enable_thinking"].is_boolean()
            && !config_input["jinja"]["context"]["enable_thinking"].get<bool>(), "nonthinking_required");
        require(!config.value("is_visual", false) && !config.value("is_audio", false)
            && !config.value("has_talker", false), "text_only_required");
        for (const auto* key : {"reuse_kv", "prompt_cache", "use_mmap", "kvcache_mmap", "use_cached_mmap"})
            require(!config.value(key, false), "cache_disabled_required");
        require(config.value("speculative_type", "").empty(), "speculation_disabled_required");
        auto start = std::chrono::steady_clock::now();
        require(llm->load(), "load_failed");
        auto loaded = std::chrono::steady_clock::now();
        // Reapply exactly the launcher-verified template/context after load's context merge.
        Json template_config = Json::object();
        template_config["jinja"] = config_input["jinja"];
        require(llm->set_config(template_config.dump()), "template_config_failed");
        auto rendered = llm->apply_chat_template(messages);
        require(!rendered.empty() && rendered.size() <= kInputBytes, "render_failed_or_too_large");
        auto tokens = llm->tokenizer_encode(rendered);
        require(!tokens.empty(), "empty_tokens");
        require(tokens.size() + static_cast<size_t>(limit) <= static_cast<size_t>(context), "logical_context_exceeded");
        auto prepared = std::chrono::steady_clock::now();
        Sink sink;
        std::ostream output(&sink);
        // The exact budgeted token vector is consumed once, without templating again.
        llm->response(tokens, &output, "", limit);
        auto done = std::chrono::steady_clock::now();
        const auto* ctx = llm->getContext();
        require(!sink.overflowed, "output_too_large");
        require(ctx && (ctx->status == LlmStatus::NORMAL_FINISHED || ctx->status == LlmStatus::MAX_TOKENS_FINISHED), "generation_failed");
        require(ctx->output_tokens.size() <= static_cast<size_t>(limit), "native_token_limit_exceeded");
        auto micros = [](auto a, auto b) { return std::chrono::duration_cast<std::chrono::microseconds>(b-a).count(); };
        Json result = identity();
        result["status"] = "ok";
        result["runtime_backend"] = config.value("backend_type", "unknown");
        result["finish_reason"] = ctx->status == LlmStatus::NORMAL_FINISHED ? "native_stop" : "native_max_tokens";
        result["logical_context"] = context;
        result["requested_max_new_tokens"] = limit;
        result["prompt_tokens"] = Json::array();
        for (int token : tokens) result["prompt_tokens"].push_back(token);
        result["output_tokens"] = Json::array();
        for (int token : ctx->output_tokens) result["output_tokens"].push_back(token);
        result["rendered_prompt"] = rendered;
        result["output_text"] = sink.bytes;
        result["native_gen_seq_len"] = ctx->gen_seq_len;
        result["native_prompt_len"] = ctx->prompt_len;
        result["load_wall_us"] = micros(start, loaded);
        result["prepare_wall_us"] = micros(loaded, prepared);
        result["generate_wall_us"] = micros(prepared, done);
        result["native_prefill_us"] = ctx->prefill_us;
        result["native_decode_us"] = ctx->decode_us;
        // Explicit output file is a private synthetic-test artifact, not a default log.
        llm.reset();
        std::ofstream file(argv[3], std::ios::binary | std::ios::trunc);
        require(static_cast<bool>(file), "result_open_failed");
        file << result.dump() << '\n';
        file.close();
        require(static_cast<bool>(file), "result_write_failed");
        std::cout << "nexa_mnn_probe_ok\n";
        return 0;
    } catch (const std::exception& e) {
        // Our errors are stable labels; upstream may abort with exceptions disabled.
        std::cerr << "nexa_mnn_probe_error: " << e.what() << '\n';
        return 1;
    } catch (...) {
        std::cerr << "nexa_mnn_probe_error: unexpected_exception\n";
        return 1;
    }
}
