cmake_minimum_required(VERSION 3.24)
project(air_llama LANGUAGES C CXX)
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_POSITION_INDEPENDENT_CODE ON)
if(NOT DEFINED CMAKE_MSVC_RUNTIME_LIBRARY)
  set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>DLL")
endif()
# The Linux cross route is opt-in and records Clang honestly. Native Windows
# builds keep their existing MSVC path; this profile is a test-build capability.
set(AIR_NATIVE_PROFILE "" CACHE STRING "Explicit non-native build profile")
if(AIR_NATIVE_PROFILE)
  if(NOT AIR_NATIVE_PROFILE STREQUAL "linux-clang-cl-msvc")
    message(FATAL_ERROR "Unknown Nexa native build profile")
  endif()
  if(NOT CMAKE_HOST_SYSTEM_NAME STREQUAL "Linux" OR
     NOT CMAKE_SYSTEM_NAME STREQUAL "Windows" OR NOT CMAKE_CROSSCOMPILING OR
     NOT CMAKE_SYSTEM_PROCESSOR MATCHES "^(AMD64|amd64|x86_64)$" OR
     NOT CMAKE_SIZEOF_VOID_P EQUAL 8 OR NOT CMAKE_BUILD_TYPE STREQUAL "Release" OR
     NOT CMAKE_MSVC_RUNTIME_LIBRARY STREQUAL "MultiThreadedDLL")
    message(FATAL_ERROR "Linux cross profile requires Windows x64 Release /MD")
  endif()
  foreach(LANGUAGE C CXX)
    if(NOT CMAKE_${LANGUAGE}_COMPILER_ID STREQUAL "Clang" OR
       NOT CMAKE_${LANGUAGE}_COMPILER_FRONTEND_VARIANT STREQUAL "MSVC" OR
       NOT CMAKE_${LANGUAGE}_SIMULATE_ID STREQUAL "MSVC" OR
       NOT CMAKE_${LANGUAGE}_COMPILER_TARGET STREQUAL "x86_64-pc-windows-msvc")
      message(FATAL_ERROR "Linux cross profile requires an explicit Clang MSVC-ABI target and frontend")
    endif()
  endforeach()
  # Compile a target-specific probe rather than treating a user-supplied profile
  # string as proof. No Windows executable is run on the Linux build host.
  include(CheckCXXSourceCompiles)
  set(CMAKE_TRY_COMPILE_CONFIGURATION Release)
  unset(AIR_CROSS_ABI_VERIFIED CACHE)
  check_cxx_source_compiles("
#if !defined(__clang__) || !defined(_MSC_VER) || !defined(_WIN64) || !defined(_M_X64) || !defined(_MT) || !defined(_DLL) || defined(_DEBUG)
#error Expected Clang Windows x64 MSVC ABI and Release dynamic CRT
#endif
static_assert(sizeof(void*) == 8, \"x64 required\");
int main() { return 0; }
" AIR_CROSS_ABI_VERIFIED)
  if(NOT AIR_CROSS_ABI_VERIFIED)
    message(FATAL_ERROR "Clang target/runtime compile probe failed")
  endif()
endif()
if(MSVC)
  add_compile_options(/utf-8)
endif()
set(BUILD_SHARED_LIBS OFF CACHE BOOL "" FORCE)
set(GGML_NATIVE OFF CACHE BOOL "" FORCE)
set(GGML_BACKEND_DL OFF CACHE BOOL "" FORCE)
set(GGML_OPENMP OFF CACHE BOOL "" FORCE)
set(GGML_CUDA OFF CACHE BOOL "" FORCE)
set(GGML_VULKAN OFF CACHE BOOL "" FORCE)
set(GGML_METAL OFF CACHE BOOL "" FORCE)
set(LLAMA_OPENSSL OFF CACHE BOOL "" FORCE)
set(LLAMA_BUILD_COMMON ON CACHE BOOL "" FORCE)
set(LLAMA_BUILD_TESTS OFF CACHE BOOL "" FORCE)
set(LLAMA_BUILD_TOOLS ON CACHE BOOL "" FORCE)
set(LLAMA_BUILD_EXAMPLES OFF CACHE BOOL "" FORCE)
set(LLAMA_BUILD_SERVER OFF CACHE BOOL "" FORCE)
set(LLAMA_BUILD_MTMD OFF CACHE BOOL "" FORCE)
set(LLAMA_SOURCE "${CMAKE_CURRENT_SOURCE_DIR}/../../vendor/llama.cpp")
execute_process(COMMAND git -C "${LLAMA_SOURCE}" rev-parse HEAD OUTPUT_VARIABLE LLAMA_COMMIT OUTPUT_STRIP_TRAILING_WHITESPACE COMMAND_ERROR_IS_FATAL ANY)
if(NOT LLAMA_COMMIT STREQUAL "2149c00f4442dc59302e134a02e4c99d5f7ed9fc")
  message(FATAL_ERROR "llama.cpp commit differs from Nexa lock")
endif()
add_subdirectory("${LLAMA_SOURCE}" llama)
if(AIR_NATIVE_PROFILE STREQUAL "linux-clang-cl-msvc")
  foreach(OPTION GGML_SSE42 GGML_AVX GGML_AVX2 GGML_FMA GGML_F16C GGML_BMI2)
    if(NOT "${${OPTION}}" STREQUAL "ON")
      message(FATAL_ERROR "Cross test builds must retain the fixed AVX2 CPU baseline: ${OPTION}=ON")
    endif()
  endforeach()
  if(NOT GGML_AVX512 STREQUAL "OFF")
    message(FATAL_ERROR "Cross test builds require GGML_AVX512=OFF")
  endif()
  # Verify the actual locked target settings too: CMake options alone are not
  # proof that the MSVC-frontend branch applied the requested CPU baseline.
  get_target_property(AIR_GGML_CPU_OPTIONS ggml-cpu COMPILE_OPTIONS)
  get_target_property(AIR_GGML_CPU_DEFINITIONS ggml-cpu COMPILE_DEFINITIONS)
  if(NOT "/arch:AVX2" IN_LIST AIR_GGML_CPU_OPTIONS)
    message(FATAL_ERROR "ggml-cpu did not apply the requested /arch:AVX2")
  endif()
  set(AIR_GGML_CPU_BASELINE_OPTIONS "/arch:AVX2")
  set(AIR_GGML_CPU_BASELINE_DEFINITIONS "")
  foreach(DEFINITION GGML_AVX2 GGML_FMA GGML_F16C __BMI2__ GGML_BMI2)
    if(NOT DEFINITION IN_LIST AIR_GGML_CPU_DEFINITIONS)
      message(FATAL_ERROR "ggml-cpu did not apply the requested definition: ${DEFINITION}")
    endif()
    list(APPEND AIR_GGML_CPU_BASELINE_DEFINITIONS "${DEFINITION}")
  endforeach()
  set(AIR_CROSS_CPU_BASELINE_VERIFIED 1)
endif()
add_library(air_llama STATIC src/air_llama.cpp)
target_include_directories(air_llama PUBLIC include)
target_link_libraries(air_llama PUBLIC llama-common llama)
target_compile_definitions(air_llama PRIVATE AIR_LLAMA_COMMIT="${LLAMA_COMMIT}")
enable_testing()
add_executable(air-stream-test tests/stream_buffer_test.cpp)
add_test(NAME air-stream-test COMMAND air-stream-test)

# Build identity is generated per configuration. Consumers must use these exact
# target paths rather than recursively selecting an archive with a matching name.
if(MSVC)
  if(NOT CMAKE_MSVC_RUNTIME_LIBRARY STREQUAL "MultiThreaded$<$<CONFIG:Debug>:Debug>DLL" AND
     NOT CMAKE_MSVC_RUNTIME_LIBRARY STREQUAL "MultiThreadedDLL")
    message(FATAL_ERROR "Nexa supports the dynamic MSVC CRT (/MD) only")
  endif()
  set(AIR_CRT "$<IF:$<CONFIG:Debug>,MDd,MD>")
else()
  set(AIR_CRT "platform")
endif()
set(AIR_IDENTITY "schema=1\nsystem=${CMAKE_SYSTEM_NAME}\nprocessor=${CMAKE_SYSTEM_PROCESSOR}\npointer_bytes=${CMAKE_SIZEOF_VOID_P}\nconfiguration=$<CONFIG>\ncrt=${AIR_CRT}\nllama_commit=${LLAMA_COMMIT}\ncompiler_id=${CMAKE_CXX_COMPILER_ID}\ncompiler_version=${CMAKE_CXX_COMPILER_VERSION}\ncompiler=${CMAKE_CXX_COMPILER}\ngenerator=${CMAKE_GENERATOR}\ngenerator_platform=${CMAKE_GENERATOR_PLATFORM}\nwindows_sdk_version=${CMAKE_VS_WINDOWS_TARGET_PLATFORM_VERSION}\nmsvc_toolset_version=${CMAKE_VS_PLATFORM_TOOLSET_VERSION}\nsource=${CMAKE_CURRENT_SOURCE_DIR}\nbuild=${CMAKE_BINARY_DIR}\n")
string(APPEND AIR_IDENTITY "profile=${AIR_NATIVE_PROFILE}\nhost_system=${CMAKE_HOST_SYSTEM_NAME}\ncross_compiling=${CMAKE_CROSSCOMPILING}\ncompiler_frontend=${CMAKE_CXX_COMPILER_FRONTEND_VARIANT}\ncompiler_simulate_id=${CMAKE_CXX_SIMULATE_ID}\ncompiler_target=${CMAKE_CXX_COMPILER_TARGET}\nc_compiler_id=${CMAKE_C_COMPILER_ID}\nc_compiler_frontend=${CMAKE_C_COMPILER_FRONTEND_VARIANT}\nc_compiler_simulate_id=${CMAKE_C_SIMULATE_ID}\nc_compiler_target=${CMAKE_C_COMPILER_TARGET}\nmsvc_runtime_library=${CMAKE_MSVC_RUNTIME_LIBRARY}\ncross_abi_verified=${AIR_CROSS_ABI_VERIFIED}\n")
string(APPEND AIR_IDENTITY "cross_cpu_baseline_verified=${AIR_CROSS_CPU_BASELINE_VERIFIED}\nggml_cpu_baseline_options=${AIR_GGML_CPU_BASELINE_OPTIONS}\nggml_cpu_baseline_definitions=${AIR_GGML_CPU_BASELINE_DEFINITIONS}\n")
foreach(OPTION GGML_NATIVE GGML_BACKEND_DL GGML_OPENMP GGML_CUDA GGML_VULKAN GGML_METAL GGML_AVX GGML_AVX2 GGML_AVX512 GGML_F16C GGML_FMA GGML_BMI2 GGML_SSE42 LLAMA_OPENSSL BUILD_SHARED_LIBS)
  string(APPEND AIR_IDENTITY "${OPTION}=${${OPTION}}\n")
endforeach()
foreach(FLAG CMAKE_C_FLAGS CMAKE_C_FLAGS_RELEASE CMAKE_CXX_FLAGS CMAKE_CXX_FLAGS_RELEASE)
  if(MSVC AND "${${FLAG}}" MATCHES "(^|[ \t])[-/](MTd?|MDd)([ \t]|$)")
    message(FATAL_ERROR "Nexa Release flags cannot override the /MD runtime: ${FLAG}")
  endif()
  string(APPEND AIR_IDENTITY "${FLAG}=${${FLAG}}\n")
endforeach()
foreach(LIBRARY air_llama llama-common llama-common-base cpp-httplib llama ggml ggml-cpu ggml-base)
  string(APPEND AIR_IDENTITY "library.${LIBRARY}=$<TARGET_FILE:${LIBRARY}>\n")
endforeach()
file(GENERATE OUTPUT "${CMAKE_BINARY_DIR}/air-native-$<CONFIG>.txt" CONTENT "${AIR_IDENTITY}")

# Pure-template tests require no model weights and cannot certify inference.
add_executable(air-template-test tests/text_template_test.cpp)
target_include_directories(air-template-test PRIVATE src)
target_link_libraries(air-template-test PRIVATE air_llama llama-common llama)
target_compile_definitions(air-template-test PRIVATE AIR_TEMPLATE_FIXTURE="${LLAMA_SOURCE}/models/templates/Qwen-Qwen3-0.6B.jinja")
# Keep assert-based contract checks active for the Release verification target.
if(MSVC)
  target_compile_options(air-template-test PRIVATE /UNDEBUG)
  target_compile_options(air-stream-test PRIVATE /UNDEBUG)
else()
  target_compile_options(air-template-test PRIVATE -UNDEBUG)
  target_compile_options(air-stream-test PRIVATE -UNDEBUG)
endif()
add_test(NAME air-template-test COMMAND air-template-test)

add_test(NAME air-template-privacy-test COMMAND air-template-test --privacy-canary)
set_tests_properties(air-template-privacy-test PROPERTIES
  FAIL_REGULAR_EXPRESSION "NEXA_PRIVATE_CANARY_930d46b1")

# No weights or tool execution: inspect the locked template/parser boundary.
add_executable(air-tool-parser-test tests/tool_parser_test.cpp)
target_link_libraries(air-tool-parser-test PRIVATE air_llama llama-common llama)
target_compile_definitions(air-tool-parser-test PRIVATE
  AIR_TOOL_TEMPLATE="${LLAMA_SOURCE}/models/templates/Qwen-Qwen3-0.6B.jinja"
  AIR_TOOL_FIXTURE="${CMAKE_CURRENT_SOURCE_DIR}/../../tests/fixtures/tools/single-tool-cycle.json")
if(MSVC)
  target_compile_options(air-tool-parser-test PRIVATE /UNDEBUG)
else()
  target_compile_options(air-tool-parser-test PRIVATE -UNDEBUG)
endif()
add_test(NAME air-tool-parser-test COMMAND air-tool-parser-test)
set_tests_properties(air-tool-parser-test PROPERTIES
  FAIL_REGULAR_EXPRESSION "NEXA_TOOL_PRIVATE_CANARY_26f074"
  TIMEOUT 30)
