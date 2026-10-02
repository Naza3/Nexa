cmake_minimum_required(VERSION 3.22)
project(nexa_mnn_probe LANGUAGES C CXX)
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(NEXA_MNN_COMMIT d407447ed56c4121a11ccbd266dc184ca1ead0c2)
set(NEXA_MNN_SOURCE "" CACHE PATH "Existing, unmodified MNN checkout; never downloaded by this build")
find_package(Git REQUIRED)
execute_process(COMMAND "${GIT_EXECUTABLE}" -C "${NEXA_MNN_SOURCE}" rev-parse HEAD
    RESULT_VARIABLE git_result OUTPUT_VARIABLE source_commit OUTPUT_STRIP_TRAILING_WHITESPACE ERROR_QUIET)
if(NOT git_result EQUAL 0 OR NOT source_commit STREQUAL NEXA_MNN_COMMIT)
    message(FATAL_ERROR "Supply the exact MNN 3.6.1 checkout using NEXA_MNN_SOURCE")
endif()
execute_process(COMMAND "${GIT_EXECUTABLE}" -C "${NEXA_MNN_SOURCE}" status --porcelain --untracked-files=all
    RESULT_VARIABLE git_result OUTPUT_VARIABLE source_dirty)
if(NOT git_result EQUAL 0 OR NOT source_dirty STREQUAL "")
    message(FATAL_ERROR "MNN checkout must be clean, including untracked files")
endif()
if(ANDROID AND NOT ANDROID_ABI STREQUAL "arm64-v8a")
    message(FATAL_ERROR "The Android prototype only targets arm64-v8a")
endif()
if(NOT ANDROID AND NOT CMAKE_SYSTEM_NAME STREQUAL "Linux")
    message(FATAL_ERROR "Only Android arm64 and Linux development validation are in scope")
endif()
# CPU-only, offline, text-only. FORCE prevents a reused cache enabling other backends.
foreach(flag MNN_BUILD_SHARED_LIBS MNN_SEP_BUILD MNN_OPENMP MNN_BUILD_TRAIN MNN_BUILD_DEMO
    MNN_BUILD_TOOLS MNN_BUILD_QUANTOOLS MNN_EVALUATION MNN_BUILD_CONVERTER MNN_BUILD_PROTOBUFFER
    MNN_BUILD_TEST MNN_BUILD_BENCHMARK MNN_BUILD_LLM_OMNI MNN_BUILD_OPENCV MNN_BUILD_AUDIO
    MNN_IMGCODECS MNN_BUILD_DIFFUSION MNN_INTERNAL MNN_JNI MNN_WITH_PLUGIN MNN_OPENCL
    MNN_OPENGL MNN_VULKAN MNN_METAL MNN_CUDA MNN_MUSA MNN_TENSORRT MNN_COREML MNN_NNAPI
    MNN_QNN MNN_HEXAGON MNN_ONEDNN MNN_KLEIDIAI MNN_SME2 MNN_AVX512 MNN_LLM_BUILD_DEMO
    MNN_LLM_BUILD_TEST LLM_SUPPORT_HTTP_RESOURCE BUILD_MLS MNN_USE_LOGCAT)
    set(${flag} OFF CACHE BOOL "Nexa CPU probe fixed option" FORCE)
endforeach()
set(MNN_BUILD_LLM ON CACHE BOOL "Nexa CPU probe" FORCE)
set(MNN_BUILD_FOR_ANDROID_COMMAND ON CACHE BOOL "Nexa CLI probe" FORCE)
add_subdirectory("${NEXA_MNN_SOURCE}" "${CMAKE_CURRENT_BINARY_DIR}/mnn" EXCLUDE_FROM_ALL)
add_executable(nexa-mnn-probe probe.cpp)
target_include_directories(nexa-mnn-probe PRIVATE "${NEXA_MNN_SOURCE}/include"
    "${NEXA_MNN_SOURCE}/transformers/llm/engine/include"
    "${NEXA_MNN_SOURCE}/transformers/llm/engine/src" "${NEXA_MNN_SOURCE}/3rd_party")
target_link_libraries(nexa-mnn-probe PRIVATE MNN)
target_compile_definitions(nexa-mnn-probe PRIVATE NEXA_MNN_COMMIT="${NEXA_MNN_COMMIT}"
    NEXA_TARGET_SYSTEM="${CMAKE_SYSTEM_NAME}" NEXA_TARGET_PROCESSOR="${CMAKE_SYSTEM_PROCESSOR}"
    NEXA_COMPILER="${CMAKE_CXX_COMPILER_ID}-${CMAKE_CXX_COMPILER_VERSION}")
if(CMAKE_CXX_COMPILER_ID MATCHES "GNU|Clang")
    target_compile_options(nexa-mnn-probe PRIVATE -Wall -Wextra -Wpedantic)
endif()
file(GENERATE OUTPUT "${CMAKE_CURRENT_BINARY_DIR}/nexa-mnn-build-$<CONFIG>.txt" CONTENT
"prototype=T07-A\nmnn_commit=${NEXA_MNN_COMMIT}\npatches=none\nsystem=${CMAKE_SYSTEM_NAME}\nprocessor=${CMAKE_SYSTEM_PROCESSOR}\ncompiler=${CMAKE_CXX_COMPILER_ID} ${CMAKE_CXX_COMPILER_VERSION}\ncmake=${CMAKE_VERSION}\nbuild_type=$<CONFIG>\nandroid_abi=${ANDROID_ABI}\nandroid_api=${ANDROID_PLATFORM}\nbackend=cpu\nhttp=off\nomni=off\nsampler=load-time-greedy\ncancel=unsupported\n")
enable_testing()
add_test(NAME mnn-probe-identity COMMAND nexa-mnn-probe --identity)
set_tests_properties(mnn-probe-identity PROPERTIES PASS_REGULAR_EXPRESSION "\"mnn_commit\":\"${NEXA_MNN_COMMIT}\"")
add_test(NAME mnn-probe-invalid-arguments COMMAND nexa-mnn-probe)
set_tests_properties(mnn-probe-invalid-arguments PROPERTIES WILL_FAIL TRUE)
