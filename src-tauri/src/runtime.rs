//! Runtime discriminator for detected games.
//!
//! Corkscrew supports games running through Wine/CrossOver bottles AND
//! games running natively on macOS. This module defines the type-level
//! split so the rest of the codebase can branch on runtime in one place
//! rather than carrying optional bottle fields.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Discriminates between a Wine/CrossOver-hosted game and a natively-running
/// macOS game. Serialises as `{"runtime": "wine", ...}` or
/// `{"runtime": "native", ...}` via the `tag` attribute.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "runtime", rename_all = "lowercase")]
pub enum GameRuntime {
    Wine(WineContext),
    Native(NativeContext),
}

/// Context for a game hosted inside a Wine/CrossOver bottle.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WineContext {
    pub bottle_name: String,
    pub bottle_path: PathBuf,
    /// The bottle provider, e.g. `"CrossOver"` or `"Wine"`.
    pub source: String,
}

/// Context for a game running natively on the host OS (macOS).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NativeContext {
    /// Path to the `.app` bundle (e.g. `/Applications/Stardew Valley.app`).
    pub app_bundle_path: PathBuf,
    /// Absolute path to the directory where game files (and per-game mod
    /// content) live. Resolved by the per-game native plugin — typically
    /// `<app_bundle>/Contents/MacOS` for Stardew, but can differ.
    pub game_data_root: PathBuf,
    pub architecture: Architecture,
    /// Whether the app runs inside an App Sandbox.
    pub sandboxed: bool,
    pub source: NativeSource,
}

/// CPU architecture the game binary targets.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Architecture {
    AppleSilicon,
    IntelOnly,
    Universal,
    Unknown,
}

/// How the native game was discovered.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NativeSource {
    SystemApplications,
    Steam,
    Gog,
    Manual,
    AppStore,
}

impl GameRuntime {
    /// Returns `true` if this is a Wine/CrossOver-hosted game.
    pub fn is_wine(&self) -> bool {
        matches!(self, Self::Wine(_))
    }

    /// Returns `true` if this is a natively-running macOS game.
    pub fn is_native(&self) -> bool {
        matches!(self, Self::Native(_))
    }

    /// Returns a reference to the inner [`WineContext`], or `None`.
    pub fn wine(&self) -> Option<&WineContext> {
        match self {
            Self::Wine(w) => Some(w),
            _ => None,
        }
    }

    /// Returns a reference to the inner [`NativeContext`], or `None`.
    pub fn native(&self) -> Option<&NativeContext> {
        match self {
            Self::Native(n) => Some(n),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn wine_context_for_test() -> WineContext {
        WineContext {
            bottle_name: "GTS".into(),
            bottle_path: PathBuf::from("/Users/x/Bottles/GTS"),
            source: "CrossOver".into(),
        }
    }

    #[test]
    fn wine_variant_round_trips_through_json() {
        let r = GameRuntime::Wine(WineContext {
            bottle_name: "GTS".into(),
            bottle_path: PathBuf::from("/Users/x/Bottles/GTS"),
            source: "CrossOver".into(),
        });
        let json = serde_json::to_string(&r).unwrap();
        let back: GameRuntime = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, GameRuntime::Wine(_)));
        let w = back.wine().unwrap();
        assert_eq!(w.bottle_name, "GTS");
    }

    #[test]
    fn native_variant_round_trips_through_json() {
        let r = GameRuntime::Native(NativeContext {
            app_bundle_path: PathBuf::from("/Applications/Stardew Valley.app"),
            game_data_root: PathBuf::from("/Applications/Stardew Valley.app/Contents/MacOS"),
            architecture: Architecture::AppleSilicon,
            sandboxed: false,
            source: NativeSource::Steam,
        });
        let json = serde_json::to_string(&r).unwrap();
        let back: GameRuntime = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, GameRuntime::Native(_)));
        let n = back.native().unwrap();
        assert_eq!(n.architecture, Architecture::AppleSilicon);
        assert_eq!(n.source, NativeSource::Steam);
        assert_eq!(
            n.app_bundle_path,
            PathBuf::from("/Applications/Stardew Valley.app")
        );
        assert_eq!(
            n.game_data_root,
            PathBuf::from("/Applications/Stardew Valley.app/Contents/MacOS")
        );
        assert!(!n.sandboxed);
    }

    #[test]
    fn discriminator_is_stable_string() {
        let r = GameRuntime::Wine(wine_context_for_test());
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["runtime"], "wine");
        assert!(!r.is_native());
        assert!(r.is_wine());
    }
}
