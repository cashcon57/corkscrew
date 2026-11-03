/*
 * Corkscrew launch trampoline.
 *
 * Steam (Proton, NotProton) and GameHub start a game by running its launcher
 * exe inside the game's own Wine environment, and they offer no way to run a
 * different program there. Corkscrew puts this trampoline in the launcher's
 * place (the real launcher is kept as "<name>.corkscrew-orig") so it can run
 * a script extender loader or a modding tool in that environment.
 *
 * What to run, all relative paths resolved against this exe's folder:
 *   1. corkscrew_launch.txt  (one-shot: read, deleted, then run)
 *   2. corkscrew_default.txt (persistent: what a plain Play runs)
 *   3. "<own file name>.corkscrew-orig" (the real launcher)
 *
 * Request file format (UTF-8, one item per line):
 *   line 1: program to run (relative to this folder, or an absolute
 *           Windows path such as Z:\Users\me\Tools\SSEEdit.exe)
 *   line 2: arguments, passed verbatim (may be empty)
 *   line 3: working directory (optional; defaults to the program's folder)
 * When no request is given, the arguments Steam passed are forwarded.
 *
 * The trampoline waits for the program and returns its exit code, so Steam
 * and GameHub keep the session open for as long as it runs.
 *
 * Build: scripts/build-trampoline.sh (x86_64-w64-mingw32-gcc, no CRT deps).
 */

#define WIN32_LEAN_AND_MEAN
#ifndef UNICODE
#define UNICODE
#endif
#ifndef _UNICODE
#define _UNICODE
#endif
#include <windows.h>

#define MAX_REQ (64 * 1024)
#define BIG 32768

static WCHAR g_dir[BIG];

static void log_line(const WCHAR *msg) {
    WCHAR path[BIG];
    lstrcpyW(path, g_dir);
    lstrcatW(path, L"corkscrew_trampoline.log");
    HANDLE h = CreateFileW(path, FILE_APPEND_DATA, FILE_SHARE_READ, NULL,
                           OPEN_ALWAYS, FILE_ATTRIBUTE_NORMAL, NULL);
    if (h == INVALID_HANDLE_VALUE) return;
    char buf[BIG];
    int n = WideCharToMultiByte(CP_UTF8, 0, msg, -1, buf, sizeof(buf) - 2, NULL, NULL);
    if (n > 0) {
        buf[n - 1] = '\r';
        buf[n] = '\n';
        DWORD w;
        WriteFile(h, buf, (DWORD)n + 1, &w, NULL);
    }
    CloseHandle(h);
}

/* Reads a UTF-8 file into a wide buffer. Returns 0 if it doesn't exist. */
static int read_utf8(const WCHAR *path, WCHAR *out, int out_len) {
    HANDLE h = CreateFileW(path, GENERIC_READ, FILE_SHARE_READ, NULL,
                           OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
    if (h == INVALID_HANDLE_VALUE) return 0;
    static char raw[MAX_REQ];
    DWORD got = 0;
    BOOL ok = ReadFile(h, raw, MAX_REQ - 1, &got, NULL);
    CloseHandle(h);
    if (!ok) return 0;
    raw[got] = 0;
    char *start = raw;
    if (got >= 3 && (unsigned char)raw[0] == 0xEF && (unsigned char)raw[1] == 0xBB &&
        (unsigned char)raw[2] == 0xBF)
        start += 3;
    int n = MultiByteToWideChar(CP_UTF8, 0, start, -1, out, out_len);
    return n > 0;
}

/* Splits buf in place at newlines; returns the number of lines (max 3). */
static int split_lines(WCHAR *buf, WCHAR *lines[3]) {
    int count = 0;
    WCHAR *p = buf;
    while (count < 3) {
        lines[count++] = p;
        WCHAR *nl = p;
        while (*nl && *nl != L'\n') nl++;
        if (!*nl) break;
        if (nl > p && nl[-1] == L'\r') nl[-1] = 0;
        *nl = 0;
        p = nl + 1;
    }
    for (int i = 0; i < count; i++) {
        int len = lstrlenW(lines[i]);
        if (len > 0 && lines[i][len - 1] == L'\r') lines[i][len - 1] = 0;
    }
    return count;
}

static int is_absolute(const WCHAR *p) {
    return (p[0] && p[1] == L':') || (p[0] == L'\\' && p[1] == L'\\');
}

static void resolve(const WCHAR *p, WCHAR *out) {
    if (is_absolute(p)) {
        lstrcpyW(out, p);
    } else {
        lstrcpyW(out, g_dir);
        lstrcatW(out, p);
    }
}

/* The arguments this exe was started with, without its own path. */
static const WCHAR *own_args(void) {
    const WCHAR *cmd = GetCommandLineW();
    if (*cmd == L'"') {
        cmd++;
        while (*cmd && *cmd != L'"') cmd++;
        if (*cmd) cmd++;
    } else {
        while (*cmd && *cmd != L' ' && *cmd != L'\t') cmd++;
    }
    while (*cmd == L' ' || *cmd == L'\t') cmd++;
    return cmd;
}

static int run(const WCHAR *program, const WCHAR *args, const WCHAR *workdir) {
    static WCHAR cmdline[BIG];
    lstrcpyW(cmdline, L"\"");
    lstrcatW(cmdline, program);
    lstrcatW(cmdline, L"\"");
    if (args && *args) {
        lstrcatW(cmdline, L" ");
        lstrcatW(cmdline, args);
    }

    static WCHAR dir[BIG];
    if (workdir && *workdir) {
        resolve(workdir, dir);
    } else {
        lstrcpyW(dir, program);
        WCHAR *slash = dir + lstrlenW(dir);
        while (slash > dir && *slash != L'\\' && *slash != L'/') slash--;
        *slash = 0;
    }

    STARTUPINFOW si;
    PROCESS_INFORMATION pi;
    ZeroMemory(&si, sizeof(si));
    si.cb = sizeof(si);
    ZeroMemory(&pi, sizeof(pi));

    static WCHAR msg[BIG];
    lstrcpyW(msg, L"run: ");
    lstrcatW(msg, cmdline);
    log_line(msg);

    if (!CreateProcessW(program, cmdline, NULL, NULL, FALSE, 0, NULL,
                        dir[0] ? dir : NULL, &si, &pi)) {
        wsprintfW(msg, L"CreateProcess failed: %lu", GetLastError());
        log_line(msg);
        return 1;
    }
    WaitForSingleObject(pi.hProcess, INFINITE);
    DWORD code = 0;
    GetExitCodeProcess(pi.hProcess, &code);
    CloseHandle(pi.hThread);
    CloseHandle(pi.hProcess);
    return (int)code;
}

int WINAPI wWinMain(HINSTANCE inst, HINSTANCE prev, PWSTR cmd, int show) {
    (void)inst; (void)prev; (void)cmd; (void)show;

    static WCHAR self[BIG];
    GetModuleFileNameW(NULL, self, BIG);
    lstrcpyW(g_dir, self);
    WCHAR *slash = g_dir + lstrlenW(g_dir);
    while (slash > g_dir && *slash != L'\\' && *slash != L'/') slash--;
    slash[1] = 0;

    static WCHAR path[BIG], buf[MAX_REQ], program[BIG];
    WCHAR *lines[3] = {0};
    int count = 0;

    lstrcpyW(path, g_dir);
    lstrcatW(path, L"corkscrew_launch.txt");
    if (read_utf8(path, buf, MAX_REQ)) {
        DeleteFileW(path); /* one-shot */
        count = split_lines(buf, lines);
    } else {
        lstrcpyW(path, g_dir);
        lstrcatW(path, L"corkscrew_default.txt");
        if (read_utf8(path, buf, MAX_REQ)) count = split_lines(buf, lines);
    }

    if (count >= 1 && lines[0][0]) {
        resolve(lines[0], program);
        const WCHAR *args = (count >= 2 && lines[1][0]) ? lines[1] : own_args();
        return run(program, args, count >= 3 ? lines[2] : NULL);
    }

    /* No request: start the real launcher. */
    lstrcpyW(program, self);
    lstrcatW(program, L".corkscrew-orig");
    return run(program, own_args(), NULL);
}
