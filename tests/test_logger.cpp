#include <catch2/catch_test_macros.hpp>
#include <fstream>
#include <filesystem>
#include "logger.h"

namespace fs = std::filesystem;

TEST_CASE("Logger initializes and logs messages", "[logger]") {
  const std::string testLogPath = "test_ossc.log";

  // Clean up any existing log
  if (fs::exists(testLogPath)) {
    fs::remove(testLogPath);
  }

  ossc::Logger::Initialize(testLogPath);

  ossc::Logger::Info("Test info message");
  ossc::Logger::Warn("Test warn message");
  ossc::Logger::Error("Test error message");

  ossc::Logger::Shutdown();

  // Verify log file exists and contains messages
  REQUIRE(fs::exists(testLogPath));

  std::ifstream file(testLogPath);
  std::string content((std::istreambuf_iterator<char>(file)),
                      std::istreambuf_iterator<char>());

  REQUIRE(content.find("Test info message") != std::string::npos);
  REQUIRE(content.find("Test warn message") != std::string::npos);
  REQUIRE(content.find("Test error message") != std::string::npos);

  fs::remove(testLogPath);
}

TEST_CASE("Logger handles multiple initialization calls", "[logger]") {
  ossc::Logger::Initialize("test_multi.log");
  ossc::Logger::Initialize("test_multi.log");  // Should not crash
  ossc::Logger::Info("Should log");
  ossc::Logger::Shutdown();

  REQUIRE(fs::exists("test_multi.log"));
  fs::remove("test_multi.log");
}
