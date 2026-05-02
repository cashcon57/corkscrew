#pragma once
#include <memory>
#include <spdlog/logger.h>

namespace ossc {

class Logger {
public:
  static void Initialize(const std::string& logPath = "ossc_capture.log");
  static void Shutdown();
  static std::shared_ptr<spdlog::logger> Get();

  template<typename... Args>
  static void Info(const char* fmt, Args... args) {
    Get()->info(fmt, args...);
  }
  template<typename... Args>
  static void Warn(const char* fmt, Args... args) {
    Get()->warn(fmt, args...);
  }
  template<typename... Args>
  static void Error(const char* fmt, Args... args) {
    Get()->error(fmt, args...);
  }
  template<typename... Args>
  static void Debug(const char* fmt, Args... args) {
    Get()->debug(fmt, args...);
  }

private:
  static std::shared_ptr<spdlog::logger> s_logger;
};

}  // namespace ossc
