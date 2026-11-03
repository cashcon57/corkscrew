/* Minimal entry point so the trampoline needs no C runtime. */
#define WIN32_LEAN_AND_MEAN
#ifndef UNICODE
#define UNICODE
#endif
#include <windows.h>

int WINAPI wWinMain(HINSTANCE, HINSTANCE, PWSTR, int);

void wWinMainCRTStartup_(void) {
    ExitProcess((UINT)wWinMain(GetModuleHandleW(NULL), NULL, GetCommandLineW(), SW_SHOWDEFAULT));
}
