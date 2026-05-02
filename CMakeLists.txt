cmake_minimum_required(VERSION 3.20)
project(OSSContribute CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# Dependencies
find_package(nlohmann_json REQUIRED)
find_package(spdlog REQUIRED)
find_package(OpenEXR REQUIRED)
find_package(Catch2 REQUIRED)

# MinHook (bundled or vcpkg)
add_library(MinHook STATIC IMPORTED)
set_target_properties(MinHook PROPERTIES
  IMPORTED_LOCATION "${CMAKE_SOURCE_DIR}/extern/MinHook/lib/MinHook.lib"
  INTERFACE_INCLUDE_DIRECTORIES "${CMAKE_SOURCE_DIR}/extern/MinHook/include"
)

# Main DLL
add_library(osscapture SHARED
  src/main.cpp
  src/logger.cpp
)

target_link_libraries(osscapture PRIVATE
  nlohmann_json::nlohmann_json
  spdlog::spdlog
  OpenEXR::OpenEXR
  MinHook
  ws2_32 kernel32 user32 gdi32
)

target_include_directories(osscapture PRIVATE
  "${CMAKE_SOURCE_DIR}/extern/MinHook/include"
  "${CMAKE_SOURCE_DIR}/src"
)

# Tests
enable_testing()
add_executable(tests
  tests/test_logger.cpp
)
target_link_libraries(tests PRIVATE osscapture Catch2::Catch2WithMain)
add_test(NAME Logger COMMAND tests)
