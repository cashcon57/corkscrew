// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // MUST be the very first thing before ANY GTK/WebKit code runs.
    // WebKitGTK's DMABuf renderer crashes on SteamOS/Gamescope with
    // "Could not create default EGL display: EGL_BAD_PARAMETER".
    #[cfg(target_os = "linux")]
    {
        // Disable DMABuf renderer (falls back to shared memory — fine for UI)
        if std::env::var("WEBKIT_DISABLE_DMABUF_RENDERER").is_err() {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
        // Also disable GPU compositing as a belt-and-suspenders fix
        // for systems where the DMABuf flag alone isn't enough
        if std::env::var("WEBKIT_DISABLE_COMPOSITING_MODE").is_err() {
            std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
        }
    }

    corkscrew_lib::run()
}
