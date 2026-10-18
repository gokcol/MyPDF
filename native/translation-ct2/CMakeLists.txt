cmake_minimum_required(VERSION 3.24)
project(MyPDFTranslation LANGUAGES C CXX)
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_POSITION_INDEPENDENT_CODE ON)
set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>")
include(FetchContent)

# Pin source revisions, including CTranslate2's own pinned submodules.
set(BUILD_SHARED_LIBS OFF CACHE BOOL "" FORCE)
set(BUILD_CLI OFF CACHE BOOL "" FORCE)
set(BUILD_TESTS OFF CACHE BOOL "" FORCE)
set(WITH_CUDA OFF CACHE BOOL "" FORCE)
set(WITH_CUDNN OFF CACHE BOOL "" FORCE)
set(WITH_MKL OFF CACHE BOOL "" FORCE)
set(WITH_OPENBLAS OFF CACHE BOOL "" FORCE)
set(OPENMP_RUNTIME NONE CACHE STRING "" FORCE)
set(WITH_RUY ON CACHE BOOL "" FORCE)
if(APPLE)
  set(WITH_ACCELERATE ON CACHE BOOL "" FORCE)
endif()
FetchContent_Declare(ctranslate2
  GIT_REPOSITORY https://github.com/OpenNMT/CTranslate2.git
  GIT_TAG 617405f4b050e994e829d527da6caa0e0030afe7
  GIT_SUBMODULES_RECURSE TRUE)
FetchContent_MakeAvailable(ctranslate2)

set(SPM_ENABLE_SHARED OFF CACHE BOOL "" FORCE)
set(SPM_ENABLE_TCMALLOC OFF CACHE BOOL "" FORCE)
set(SPM_BUILD_TEST OFF CACHE BOOL "" FORCE)
FetchContent_Declare(sentencepiece
  GIT_REPOSITORY https://github.com/google/sentencepiece.git
  GIT_TAG 17d7580d6407802f85855d2cc9190634e2c95624)
FetchContent_MakeAvailable(sentencepiece)

add_library(mypdf_ct2 SHARED bridge.cpp)
target_include_directories(mypdf_ct2 PRIVATE
  ${ctranslate2_SOURCE_DIR}/include ${sentencepiece_SOURCE_DIR}/src)
target_link_libraries(mypdf_ct2 PRIVATE ctranslate2 sentencepiece-static)
set_target_properties(mypdf_ct2 PROPERTIES
  CXX_VISIBILITY_PRESET hidden VISIBILITY_INLINES_HIDDEN YES)
install(TARGETS mypdf_ct2
  RUNTIME DESTINATION . COMPONENT MyPDFTranslation
  LIBRARY DESTINATION . COMPONENT MyPDFTranslation)
install(FILES ${ctranslate2_SOURCE_DIR}/LICENSE DESTINATION licenses RENAME CTranslate2.txt COMPONENT MyPDFTranslation)
install(FILES ${sentencepiece_SOURCE_DIR}/LICENSE DESTINATION licenses RENAME SentencePiece.txt COMPONENT MyPDFTranslation)
install(DIRECTORY ${ctranslate2_SOURCE_DIR}/third_party/ DESTINATION licenses/CTranslate2-dependencies
  COMPONENT MyPDFTranslation FILES_MATCHING PATTERN "LICENSE*" PATTERN "COPYING*" PATTERN "COPYRIGHT*" PATTERN "NOTICE*")
install(DIRECTORY ${sentencepiece_SOURCE_DIR}/third_party/ DESTINATION licenses/SentencePiece-dependencies
  COMPONENT MyPDFTranslation FILES_MATCHING PATTERN "LICENSE*" PATTERN "COPYING*" PATTERN "COPYRIGHT*" PATTERN "NOTICE*")
install(FILES licenses/Argos-Translate.txt languages.json DESTINATION licenses COMPONENT MyPDFTranslation)
