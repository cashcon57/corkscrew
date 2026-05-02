#include "logger.h"
#include <spdlog/sinks/basic_file_sink.h>
#include <spdlog/sinks/stdout_color_sinks.h>
#include <spdlog/pattern_formatter.h>

namespace ossc {

std::shared_ptr<spdlog::logger> Logger::s_logger = nullptr;

void Logger::Initialize(const std::string& logPath) {
  try {
    std::vector<spdlog::sink_ptr> sinks;
    sinks.push_back(std::make_shared<spdlog::sinks::basic_file_sink_mt>(logPath));
    sinks.push_back(std::make_shared<spdlog::sinks::stdout_color_sink_mt>());

    s_logger = std::make_shared<spdlog::logger>("osscapture", sinks.begin(), sinks.end());
    s_logger->set_level(spdlog::level::debug);
    s_logger->set_pattern("[%Y-%m-%d %H:%M:%S] [%l] %v");
    spdlog::register_logger(s_logger);
  } catch (const spdlog::spdlog_ex& ex) {
    // Fallback: create console logger
    s_logger = spdlog::stdout_color_mt("osscapture");
  }
}

void Logger::Shutdown() {
  if (s_logger) {
    s_logger->flush();
    spdlog::drop("osscapture");
    s_logger = nullptr;
  }
}

std::shared_ptr<spdlog::logger> Logger::Get() {
  if (!s_logger) {
    Initialize();
  }
  return s_logger;
}

}  // namespace ossc
