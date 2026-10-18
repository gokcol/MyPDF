// Narrow C ABI. Never logs inputs or forwards them to a network service.
#include <ctranslate2/translator.h>
#include <sentencepiece_processor.h>
#include <algorithm>
#include <cstring>
#include <memory>
#include <string>
#include <thread>
#include <vector>

#ifdef _WIN32
#define API extern "C" __declspec(dllexport)
#else
#define API extern "C" __attribute__((visibility("default")))
#endif

struct Engine {
  sentencepiece::SentencePieceProcessor tokenizer;
  std::unique_ptr<ctranslate2::Translator> translator;
};
using Cancelled = bool (*)(void*);

API unsigned mypdf_ct2_abi() { return 1; }

// Error codes are deliberately content-free. Exceptions never cross the ABI.
API void* mypdf_ct2_create(const char* model, const char* tokenizer) {
  try {
    if (!model || !tokenizer) return nullptr;
    auto engine = std::make_unique<Engine>();
    // CTranslate2's file helper also handles UTF-8 Windows account paths.
    auto file = ctranslate2::open_file_read(tokenizer, std::ios::in | std::ios::binary);
    std::string serialized((std::istreambuf_iterator<char>(file)), std::istreambuf_iterator<char>());
    if (!engine->tokenizer.LoadFromSerializedProto(serialized).ok()) return nullptr;
    ctranslate2::ReplicaPoolConfig config;
    config.num_threads_per_replica = std::max(1u, std::min(4u, std::thread::hardware_concurrency()));
    config.max_queued_batches = 1;
    engine->translator = std::make_unique<ctranslate2::Translator>(
      model, ctranslate2::Device::CPU, ctranslate2::ComputeType::DEFAULT,
      std::vector<int>{0}, false, config);
    return engine.release();
  } catch (...) { return nullptr; }
}

API void mypdf_ct2_destroy(void* pointer) {
  delete static_cast<Engine*>(pointer);
}

// 0 success, 1 cancelled, 2 invalid/oversized input, 3 inference failure,
// 4 output limit (never silently publish truncated translations).
API int mypdf_ct2_translate(void* pointer, const char* input, size_t length,
                            Cancelled cancelled, void* context, char** output) {
  if (!pointer || !input || !output || length > 32768) return 2;
  *output = nullptr;
  try {
    if (cancelled && cancelled(context)) return 1;
    auto& engine = *static_cast<Engine*>(pointer);
    std::vector<std::string> tokens;
    if (!engine.tokenizer.Encode(std::string(input, length), &tokens).ok()) return 2;
    if (tokens.size() > 512) return 2;
    ctranslate2::TranslationOptions options;
    options.beam_size = 1; // Enables per-token cancellation.
    options.max_input_length = 0;
    options.max_decoding_length = 1024;
    options.callback = [cancelled, context](ctranslate2::GenerationStepResult) {
      return cancelled && cancelled(context);
    };
    const auto results = engine.translator->translate_batch({tokens}, options);
    if (cancelled && cancelled(context)) return 1;
    if (results.empty() || results[0].hypotheses.empty()) return 3;
    if (results[0].output().size() >= options.max_decoding_length) return 4;
    std::string translated;
    if (!engine.tokenizer.Decode(results[0].output(), &translated).ok()) return 3;
    auto text = std::make_unique<char[]>(translated.size() + 1);
    std::memcpy(text.get(), translated.c_str(), translated.size() + 1);
    *output = text.release();
    return 0;
  } catch (...) { return 3; }
}

API void mypdf_ct2_free(char* text) { delete[] text; }
