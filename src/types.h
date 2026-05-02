#pragma once
#include <string>
#include <vector>
#include <cstdint>
#include <cstddef>

namespace ossc {

struct ResourceSlot {
  std::string name;           // e.g., "LRColorRT"
  uint32_t descriptorIndex;  // D3D12 descriptor heap index
  uint32_t width;
  uint32_t height;
  const char* dxgiFormat;    // e.g., "DXGI_FORMAT_R8G8B8A8_UNORM"
};

struct FrameCapture {
  uint64_t frameNumber;
  uint64_t timestampMs;
  std::vector<std::string> resourceNames;  // Resources captured in this frame
  std::string outputDirectory;
  bool captureColor;
  bool captureDepth;
  bool captureNormals;
  bool captureMotion;
};

struct CaptureConfig {
  uint32_t frameInterval;        // Capture every N frames (default 30)
  std::string outputBase;        // Base output directory
  std::string gameProfile;       // e.g., "cyberpunk2077.json"
  bool validateAntiCheat;        // Check for BattlEye/EAC/Vanguard
  uint32_t maxStagingBuffersMB;  // Max vRAM for staging (default 256)
};

}  // namespace ossc
