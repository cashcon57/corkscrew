#include <windows.h>
#include "logger.h"
#include "types.h"

namespace ossc {

static bool g_initialized = false;

extern "C" {
  __declspec(dllexport) HRESULT AttachCapture() {
    if (g_initialized) {
      return S_FALSE;  // Already attached
    }

    try {
      Logger::Initialize();
      Logger::Info("OSSContribute attached to process");
      g_initialized = true;
      return S_OK;
    } catch (const std::exception& ex) {
      Logger::Error("Failed to attach: {}", ex.what());
      return E_FAIL;
    }
  }

  __declspec(dllexport) HRESULT DetachCapture() {
    if (!g_initialized) {
      return S_FALSE;  // Not attached
    }

    try {
      Logger::Info("OSSContribute detached from process");
      Logger::Shutdown();
      g_initialized = false;
      return S_OK;
    } catch (const std::exception& ex) {
      Logger::Error("Failed to detach: {}", ex.what());
      return E_FAIL;
    }
  }
}

}  // namespace ossc

BOOL APIENTRY DllMain(HMODULE hModule, DWORD ul_reason_for_call, LPVOID lpReserved) {
  switch (ul_reason_for_call) {
    case DLL_PROCESS_ATTACH:
      // Lazy init: AttachCapture() called explicitly from host
      break;
    case DLL_PROCESS_DETACH:
      if (ossc::g_initialized) {
        ossc::DetachCapture();
      }
      break;
    case DLL_THREAD_ATTACH:
    case DLL_THREAD_DETACH:
      break;
  }
  return TRUE;
}
