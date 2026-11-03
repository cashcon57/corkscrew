//! Choosing which CrossOver install runs a bottle (macOS).
//!
//! A Mac can have several CrossOver builds side by side (e.g.
//! `CrossOver.app` and `CrossOver Preview.app`). A bottle must be run by the
//! build that owns it: if that build's wineserver is already running for the
//! prefix, a `wine` from another build can't attach to it ("version
//! mismatch") and exits within seconds.
//!
//! Selection order for a bottle:
//! 1. A per-bottle override (`crossover_app_path:<bottle>` config value).
//! 2. The install whose wineserver is live for the bottle's prefix.
//! 3. The install matching the bottle's `cxbottle.conf` `Timestamp` (then
//!    `Version`) — the build that created or last upgraded it.
//! 4. The first install found (`/Applications/CrossOver.app` first).

use std::fs;
use std::path::{Path, PathBuf};

use log::{debug, warn};
use serde::{Deserialize, Serialize};

/// Path from a CrossOver .app bundle root to its Wine launcher.
pub const CROSSOVER_WINE_SUFFIX: &str = "Contents/SharedSupport/CrossOver/bin/wine";

/// Path from a CrossOver .app bundle root to its product config.
const CROSSOVER_CONF_SUFFIX: &str = "Contents/SharedSupport/CrossOver/etc/CrossOver.conf";

/// Bundle identifier prefix shared by CrossOver and CrossOver Preview.
const CROSSOVER_BUNDLE_ID_PREFIX: &str = "com.codeweavers.CrossOver";

/// Config key prefix for the per-bottle CrossOver override.
pub const OVERRIDE_KEY_PREFIX: &str = "crossover_app_path:";

/// One installed CrossOver build.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrossOverInstall {
    /// The `.app` bundle.
    pub app_path: PathBuf,
    /// Bundle name without `.app` (e.g. "CrossOver Preview").
    pub name: String,
    /// `ProductVersion` from CrossOver.conf, else `CFBundleVersion`.
    pub version: Option<String>,
    /// `BuildTimestamp` from CrossOver.conf (e.g. "20260715T103801Z").
    pub build_timestamp: Option<String>,
}

impl CrossOverInstall {
    pub fn wine(&self) -> PathBuf {
        self.app_path.join(CROSSOVER_WINE_SUFFIX)
    }
}

/// Why an install was chosen for a bottle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionReason {
    /// The user picked it in Settings.
    Override,
    /// Its wineserver is running for this bottle.
    LiveWineserver,
    /// The bottle was created or last upgraded by this build.
    BottleVersion,
    /// No better signal; first install found.
    Default,
}

/// Why no install could be chosen.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SelectError {
    #[error("No CrossOver install found")]
    NoInstalls,
    #[error(
        "The CrossOver install chosen for this bottle ({0}) was not found or has no Wine. \
         Pick another in Settings \u{2192} Game Maintenance \u{2192} CrossOver install."
    )]
    BadOverride(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrossOverSelection {
    pub install: CrossOverInstall,
    pub reason: SelectionReason,
}

// ---------------------------------------------------------------------------
// Config file parsing (pure)
// ---------------------------------------------------------------------------

/// Read `"Key" = "Value"` from a CrossOver config (`CrossOver.conf`,
/// `cxbottle.conf`). Commented lines (`;;`) are skipped. With `section`, only
/// that `[Section]` is searched.
pub(crate) fn cx_conf_value(content: &str, section: Option<&str>, key: &str) -> Option<String> {
    let mut in_section = section.is_none();
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with(';') || line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            if let Some(want) = section {
                in_section = name.eq_ignore_ascii_case(want);
            }
            continue;
        }
        if !in_section {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        if k.trim().trim_matches('"').eq_ignore_ascii_case(key) {
            let v = v.trim().trim_matches('"').trim();
            return (!v.is_empty()).then(|| v.to_string());
        }
    }
    None
}

/// Pick the install that created or last upgraded a bottle, from its
/// `cxbottle.conf`: exact build timestamp first, then product version.
pub(crate) fn match_bottle_conf<'a>(
    cxbottle_conf: &str,
    installs: &'a [CrossOverInstall],
) -> Option<&'a CrossOverInstall> {
    if let Some(ts) = cx_conf_value(cxbottle_conf, Some("Bottle"), "Timestamp") {
        if let Some(i) = installs
            .iter()
            .find(|i| i.build_timestamp.as_deref() == Some(ts.as_str()))
        {
            return Some(i);
        }
    }
    let version = cx_conf_value(cxbottle_conf, Some("Bottle"), "Version")?;
    installs
        .iter()
        .find(|i| i.version.as_deref() == Some(version.as_str()))
}

/// Name of Wine's per-prefix server directory: `server-<st_dev>-<st_ino>` of
/// the prefix directory, in lowercase hex (ntdll `init_server_dir`).
pub(crate) fn wineserver_dir_name(dev: u64, ino: u64) -> String {
    format!("server-{dev:x}-{ino:x}")
}

/// The outermost `.app` bundle containing `exe` (wineservers live several
/// levels deep, sometimes inside a nested `wine.app`).
pub(crate) fn owning_app_bundle(exe: &Path) -> Option<PathBuf> {
    exe.ancestors()
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("app"))
        })
        .last()
        .map(Path::to_path_buf)
}

// ---------------------------------------------------------------------------
// Install discovery
// ---------------------------------------------------------------------------

/// Describe a CrossOver bundle, or `None` if it has no Wine launcher.
pub fn crossover_install_info(app: &Path) -> Option<CrossOverInstall> {
    if !app.join(CROSSOVER_WINE_SUFFIX).is_file() {
        return None;
    }
    let conf = fs::read_to_string(app.join(CROSSOVER_CONF_SUFFIX)).unwrap_or_default();
    let version = cx_conf_value(&conf, None, "ProductVersion")
        .or_else(|| bundle_plist_string(app, "CFBundleVersion"));
    let name = app
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "CrossOver".to_string());
    Some(CrossOverInstall {
        app_path: app.to_path_buf(),
        name,
        version,
        build_timestamp: cx_conf_value(&conf, None, "BuildTimestamp"),
    })
}

fn bundle_plist_string(app: &Path, key: &str) -> Option<String> {
    let value = ::plist::Value::from_file(app.join("Contents/Info.plist")).ok()?;
    value
        .as_dictionary()?
        .get(key)?
        .as_string()
        .map(str::to_string)
}

/// `CrossOver*.app` bundles directly inside `dir`, sorted with the plain
/// `CrossOver.app` first.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn crossover_bundles_in(dir: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                n.starts_with("CrossOver") && n.to_ascii_lowercase().ends_with(".app")
            })
        })
        .collect();
    found.sort_by_key(|p| {
        (
            p.file_name().is_some_and(|n| n != "CrossOver.app"),
            p.clone(),
        )
    });
    found
}

/// All CrossOver installs on this Mac: `CrossOver*.app` in `/Applications`
/// and `~/Applications`, plus anything Spotlight knows by bundle id.
#[cfg(target_os = "macos")]
pub fn list_crossover_installs() -> Vec<CrossOverInstall> {
    let mut candidates = crossover_bundles_in(Path::new("/Applications"));
    if let Some(home) = dirs::home_dir() {
        candidates.extend(crossover_bundles_in(&home.join("Applications")));
    }
    if let Ok(out) = std::process::Command::new("mdfind")
        .arg(format!(
            "kMDItemCFBundleIdentifier == '{CROSSOVER_BUNDLE_ID_PREFIX}*'"
        ))
        .output()
    {
        candidates.extend(
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(|l| PathBuf::from(l.trim()))
                .filter(|p| p.is_absolute()),
        );
    }

    let mut seen: Vec<PathBuf> = Vec::new();
    let mut installs = Vec::new();
    for app in candidates {
        let key = fs::canonicalize(&app).unwrap_or_else(|_| app.clone());
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        if let Some(info) = crossover_install_info(&app) {
            installs.push(info);
        }
    }
    installs
}

#[cfg(not(target_os = "macos"))]
pub fn list_crossover_installs() -> Vec<CrossOverInstall> {
    Vec::new()
}

// ---------------------------------------------------------------------------
// Live wineserver lookup
// ---------------------------------------------------------------------------

/// Wine's server directory for a prefix: `/tmp/.wine-<uid>/server-<dev>-<ino>`.
#[cfg(unix)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn wineserver_dir_for_prefix(prefix: &Path) -> Option<PathBuf> {
    // SAFETY: getuid has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    let dir = wineserver_dir_path(prefix, uid)?;
    let base = dir.parent()?;
    // /tmp is shared: only trust directories this user owns that nobody else
    // can write to (Wine itself creates them 0700).
    (owned_private_dir(base, uid) && owned_private_dir(&dir, uid)).then_some(dir)
}

/// Where Wine puts the server directory for `prefix` (not checked).
#[cfg(unix)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn wineserver_dir_path(prefix: &Path, uid: u32) -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    let meta = fs::metadata(prefix).ok()?;
    Some(PathBuf::from(format!("/tmp/.wine-{uid}")).join(wineserver_dir_name(meta.dev(), meta.ino())))
}

/// A real directory (not a symlink) owned by `uid` and not group/world-writable.
#[cfg(unix)]
pub(crate) fn owned_private_dir(path: &Path, uid: u32) -> bool {
    use std::os::unix::fs::MetadataExt;
    fs::symlink_metadata(path).is_ok_and(|m| {
        m.file_type().is_dir() && m.uid() == uid && m.mode() & 0o022 == 0
    })
}

/// PID holding the wineserver's lock (`F_GETLK` on `<server dir>/lock`), if
/// a server is running for that directory.
#[cfg(unix)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn wineserver_lock_holder(server_dir: &Path) -> Option<i32> {
    use std::os::unix::io::AsRawFd;
    if !server_dir.join("socket").exists() {
        return None;
    }
    let lock = fs::File::open(server_dir.join("lock")).ok()?;
    // SAFETY: zeroed flock is a valid "whole file" lock description.
    let mut fl: libc::flock = unsafe { std::mem::zeroed() };
    fl.l_type = libc::F_WRLCK as _;
    fl.l_whence = libc::SEEK_SET as _;
    // SAFETY: fd is open for the call's duration and fl is a valid flock.
    let rc = unsafe { libc::fcntl(lock.as_raw_fd(), libc::F_GETLK, &mut fl) };
    if rc != 0 || i32::from(fl.l_type) == libc::F_UNLCK as i32 || fl.l_pid <= 0 {
        return None;
    }
    Some(fl.l_pid)
}

#[cfg(target_os = "macos")]
fn process_exe_path(pid: i32) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: buf is writable for buf.len() bytes.
    let n = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    if n <= 0 {
        return None;
    }
    buf.truncate(n as usize);
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(&buf)))
}

/// The `.app` whose wineserver is currently running for `prefix`.
#[cfg(target_os = "macos")]
pub fn live_wineserver_app(prefix: &Path) -> Option<PathBuf> {
    let dir = wineserver_dir_for_prefix(prefix)?;
    let pid = wineserver_lock_holder(&dir)?;
    let exe = process_exe_path(pid)?;
    debug!(
        "live wineserver for {}: pid {} ({})",
        prefix.display(),
        pid,
        exe.display()
    );
    owning_app_bundle(&exe)
}

#[cfg(not(target_os = "macos"))]
pub fn live_wineserver_app(_prefix: &Path) -> Option<PathBuf> {
    None
}

// ---------------------------------------------------------------------------
// Selection
// ---------------------------------------------------------------------------

/// The per-bottle override, if set (an empty value means "automatic").
pub fn override_for_bottle(bottle_name: &str) -> Option<PathBuf> {
    crate::config::get_config_value(&format!("{OVERRIDE_KEY_PREFIX}{bottle_name}"))
        .ok()
        .flatten()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// Pure selection over already-gathered signals; see the module docs.
pub(crate) fn select_install(
    installs: &[CrossOverInstall],
    override_app: Option<&Path>,
    live_app: Option<&Path>,
    cxbottle_conf: Option<&str>,
) -> Result<CrossOverSelection, SelectError> {
    let same = |a: &Path, b: &Path| {
        a == b
            || fs::canonicalize(a)
                .ok()
                .zip(fs::canonicalize(b).ok())
                .is_some_and(|(x, y)| x == y)
    };
    // Only ever run a CrossOver that was found by `list_crossover_installs`;
    // a path from config or from a process on the system is never probed
    // and trusted on its own.
    let known = |app: &Path| installs.iter().find(|i| same(&i.app_path, app)).cloned();

    if let Some(app) = override_app {
        return known(app)
            .map(|install| CrossOverSelection {
                install,
                reason: SelectionReason::Override,
            })
            .ok_or_else(|| SelectError::BadOverride(app.display().to_string()));
    }

    if let Some(install) = live_app.and_then(known) {
        return Ok(CrossOverSelection {
            install,
            reason: SelectionReason::LiveWineserver,
        });
    }

    if let Some(install) = cxbottle_conf.and_then(|c| match_bottle_conf(c, installs)) {
        return Ok(CrossOverSelection {
            install: install.clone(),
            reason: SelectionReason::BottleVersion,
        });
    }

    installs
        .first()
        .cloned()
        .map(|install| CrossOverSelection {
            install,
            reason: SelectionReason::Default,
        })
        .ok_or(SelectError::NoInstalls)
}

/// Choose the CrossOver install to run `bottle_path` (a CrossOver bottle
/// named `bottle_name`).
pub fn select_for_bottle(
    bottle_name: &str,
    bottle_path: &Path,
) -> Result<CrossOverSelection, SelectError> {
    select_for_bottle_among(&list_crossover_installs(), bottle_name, bottle_path)
}

/// [`select_for_bottle`] over an already-listed set of installs.
pub fn select_for_bottle_among(
    installs: &[CrossOverInstall],
    bottle_name: &str,
    bottle_path: &Path,
) -> Result<CrossOverSelection, SelectError> {
    let override_app = override_for_bottle(bottle_name);
    let live_app = live_wineserver_app(bottle_path);
    let conf = fs::read_to_string(bottle_path.join("cxbottle.conf")).ok();
    let selection = select_install(
        installs,
        override_app.as_deref(),
        live_app.as_deref(),
        conf.as_deref(),
    )?;
    if installs.len() > 1 || selection.reason == SelectionReason::Override {
        log::info!(
            "CrossOver for bottle '{}': {} ({:?}; {} install(s) found)",
            bottle_name,
            selection.install.app_path.display(),
            selection.reason,
            installs.len()
        );
    }
    if let (Some(live), SelectionReason::Override) = (&live_app, selection.reason) {
        if live != &selection.install.app_path {
            warn!(
                "Bottle '{}' has a running wineserver from {}, but the override selects {}; \
                 the launch will fail until that wineserver exits",
                bottle_name,
                live.display(),
                selection.install.app_path.display()
            );
        }
    }
    Ok(selection)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const CROSSOVER_CONF: &str = r#";;-----< [CrossOver] Section >----
[CrossOver]
;; ProductVersion       The internal product version.
;;"ProductPackage" = ""
"ProductVersion" = "26.3.0.39832"
"BuildTag" = "cxoffice-26.3.0rc2"
"BuildTimestamp" = "20260715T103801Z"
"#;

    fn fake_bundle(dir: &Path, name: &str, version: &str, ts: &str) -> PathBuf {
        let app = dir.join(format!("{name}.app"));
        let wine = app.join(CROSSOVER_WINE_SUFFIX);
        fs::create_dir_all(wine.parent().unwrap()).unwrap();
        fs::write(&wine, "#!/bin/sh\n").unwrap();
        let conf = app.join(CROSSOVER_CONF_SUFFIX);
        fs::create_dir_all(conf.parent().unwrap()).unwrap();
        fs::write(
            &conf,
            format!(
                "[CrossOver]\n;;\"ProductVersion\" = \"0\"\n\"ProductVersion\" = \"{version}\"\n\"BuildTimestamp\" = \"{ts}\"\n"
            ),
        )
        .unwrap();
        app
    }

    fn cxbottle(version: &str, ts: &str) -> String {
        format!(
            "[Bottle]\n\"WineArch\" = \"win64\"\n;; Version   the CrossOver version\n\
             \"Version\" = \"{version}\"\n\"Timestamp\" = \"{ts}\"\n\n[EnvironmentVariables]\n\"Version\" = \"bogus\"\n"
        )
    }

    #[test]
    fn parses_crossover_conf_values() {
        assert_eq!(
            cx_conf_value(CROSSOVER_CONF, None, "ProductVersion").as_deref(),
            Some("26.3.0.39832")
        );
        assert_eq!(
            cx_conf_value(CROSSOVER_CONF, None, "BuildTimestamp").as_deref(),
            Some("20260715T103801Z")
        );
        assert_eq!(cx_conf_value(CROSSOVER_CONF, None, "ProductPackage"), None);
        let b = cxbottle("26.3.0.39832", "20260715T103801Z");
        assert_eq!(
            cx_conf_value(&b, Some("Bottle"), "Version").as_deref(),
            Some("26.3.0.39832")
        );
        assert_eq!(
            cx_conf_value(&b, Some("EnvironmentVariables"), "Version").as_deref(),
            Some("bogus")
        );
    }

    #[test]
    fn wineserver_dir_name_matches_wine_format() {
        // st_dev 16777232, st_ino 80411094 → /tmp/.wine-501/server-1000010-4caf9d6
        assert_eq!(
            wineserver_dir_name(16_777_232, 80_411_094),
            "server-1000010-4caf9d6"
        );
        assert_eq!(
            wineserver_dir_name(0x4748_0000, 0x967e_0655_6388_f73f),
            "server-47480000-967e06556388f73f"
        );
    }

    #[cfg(unix)]
    #[test]
    fn wineserver_dir_for_prefix_uses_prefix_inode() {
        use std::os::unix::fs::MetadataExt;
        let tmp = tempfile::tempdir().unwrap();
        let meta = fs::metadata(tmp.path()).unwrap();
        let dir = wineserver_dir_path(tmp.path(), 501).unwrap();
        assert!(dir.starts_with("/tmp/.wine-501"));
        assert_eq!(
            dir.file_name().unwrap().to_str().unwrap(),
            format!("server-{:x}-{:x}", meta.dev(), meta.ino())
        );
        // Not running: no socket, no lock holder.
        assert_eq!(wineserver_lock_holder(tmp.path()), None);
    }

    #[cfg(unix)]
    #[test]
    fn wineserver_dirs_must_be_private_and_owned() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        // SAFETY: getuid has no preconditions.
        let uid = unsafe { libc::getuid() };
        let dir = tmp.path().join("server");
        fs::create_dir(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(owned_private_dir(&dir, uid));
        assert!(!owned_private_dir(&dir, uid + 1), "other owner");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(!owned_private_dir(&dir, uid), "world-writable");
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&dir, &link).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(!owned_private_dir(&link, uid), "symlink");
    }

    #[test]
    fn owning_app_bundle_picks_outermost_app() {
        let exe = Path::new(
            "/Users/x/Applications/CrossOver Preview.app/Contents/SharedSupport/CrossOver/lib/wine/aarch64-unix/wine.app/Contents/MacOS/wineserver",
        );
        assert_eq!(
            owning_app_bundle(exe),
            Some(PathBuf::from("/Users/x/Applications/CrossOver Preview.app"))
        );
        assert_eq!(
            owning_app_bundle(Path::new("/usr/local/bin/wineserver")),
            None
        );
    }

    #[test]
    fn matches_bottle_to_bundle_by_timestamp_then_version() {
        let tmp = tempfile::tempdir().unwrap();
        let stable = crossover_install_info(&fake_bundle(
            tmp.path(),
            "CrossOver",
            "26.3.0.39832",
            "20260715T103801Z",
        ))
        .unwrap();
        let preview = crossover_install_info(&fake_bundle(
            tmp.path(),
            "CrossOver Preview",
            "26.4.0.40100",
            "20261015T090000Z",
        ))
        .unwrap();
        assert_eq!(stable.version.as_deref(), Some("26.3.0.39832"));
        assert_eq!(preview.name, "CrossOver Preview");
        let installs = vec![stable.clone(), preview.clone()];

        let by_ts = cxbottle("26.4.0.40100", "20261015T090000Z");
        assert_eq!(match_bottle_conf(&by_ts, &installs), Some(&preview));

        // Timestamp unknown (e.g. rebuilt preview) → product version.
        let by_version = cxbottle("26.3.0.39832", "20250101T000000Z");
        assert_eq!(match_bottle_conf(&by_version, &installs), Some(&stable));

        assert_eq!(match_bottle_conf(&cxbottle("1.0", "x"), &installs), None);

        // Selection: bottle conf beats the default first install.
        let sel = select_install(&installs, None, None, Some(&by_ts)).unwrap();
        assert_eq!(sel.install, preview);
        assert_eq!(sel.reason, SelectionReason::BottleVersion);
        let sel = select_install(&installs, None, None, None).unwrap();
        assert_eq!(
            (sel.install, sel.reason),
            (stable.clone(), SelectionReason::Default)
        );
    }

    #[test]
    fn selection_prefers_override_then_live_wineserver() {
        let tmp = tempfile::tempdir().unwrap();
        let stable_app = fake_bundle(tmp.path(), "CrossOver", "26.3", "A");
        let preview_app = fake_bundle(tmp.path(), "CrossOver Preview", "26.4", "B");
        let installs: Vec<_> = [&stable_app, &preview_app]
            .iter()
            .filter_map(|a| crossover_install_info(a))
            .collect();
        let conf = cxbottle("26.3", "A"); // bottle conf says stable…

        // …but the running wineserver is Preview's.
        let sel = select_install(&installs, None, Some(&preview_app), Some(&conf)).unwrap();
        assert_eq!(sel.install.app_path, preview_app);
        assert_eq!(sel.reason, SelectionReason::LiveWineserver);

        // The override wins over everything.
        let sel = select_install(
            &installs,
            Some(&stable_app),
            Some(&preview_app),
            Some(&conf),
        )
        .unwrap();
        assert_eq!(sel.install.app_path, stable_app);
        assert_eq!(sel.reason, SelectionReason::Override);

        // An override pointing nowhere is an error, not a silent fallback.
        let err =
            select_install(&installs, Some(&tmp.path().join("Gone.app")), None, None).unwrap_err();
        assert!(err.to_string().contains("Gone.app"), "{err}");

        // A bundle Corkscrew didn't find itself is never trusted: not as the
        // live wineserver's app (falls back to the first install)…
        let other = fake_bundle(&tmp.path().join("elsewhere"), "CrossOver Beta", "27", "C");
        let sel = select_install(&installs, None, Some(&other), None).unwrap();
        assert_eq!(sel.install.app_path, stable_app);
        assert_eq!(sel.reason, SelectionReason::Default);
        // …and not as an override.
        assert!(select_install(&installs, Some(&other), None, None).is_err());

        assert!(select_install(&[], None, None, None).is_err());
    }

    #[test]
    fn bundles_in_dir_lists_crossover_apps_stable_first() {
        let tmp = tempfile::tempdir().unwrap();
        fake_bundle(tmp.path(), "CrossOver Preview", "2", "B");
        fake_bundle(tmp.path(), "CrossOver", "1", "A");
        fs::create_dir_all(tmp.path().join("Other.app")).unwrap();
        let names: Vec<_> = crossover_bundles_in(tmp.path())
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["CrossOver.app", "CrossOver Preview.app"]);
    }
}
