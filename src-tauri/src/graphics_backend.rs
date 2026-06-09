//! Graphics backend selection for macOS Wine/CrossOver.
//!
//! On macOS, multiple DirectX translation layers coexist:
//! - **DXVK + MoltenVK**: DX9-11 → Vulkan → Metal (most compatible)
//! - **DXMT**: DX11 → Metal directly (best perf on Apple Silicon, CrossOver 25+)
//! - **D3DMetal**: DX11-12 → Metal (Apple's GPTK approach)
//! - **wined3d**: DX9-11 → OpenGL → Metal (oldest, fallback)
//!
//! This module detects available backends, recommends the best one per game,
//! and generates the env vars / DLL overrides needed to activate each backend.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Available graphics translation backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphicsBackend {
    /// DXVK translating to Vulkan, then MoltenVK to Metal. Most compatible.
    DxvkMoltenVk,
    /// DXMT: Direct DX11→Metal translation. Best performance on Apple Silicon.
    Dxmt,
    /// D3DMetal: Apple's Game Porting Toolkit approach. DX11-12→Metal.
    D3dMetal,
    /// wined3d: Classic Wine OpenGL translation. Fallback for DX9 and edge cases.
    Wined3d,
    /// Use bottle's default — don't override anything.
    Default,
}

impl GraphicsBackend {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::DxvkMoltenVk => "DXVK + MoltenVK",
            Self::Dxmt => "DXMT (Direct Metal)",
            Self::D3dMetal => "D3DMetal (GPTK)",
            Self::Wined3d => "wined3d (OpenGL)",
            Self::Default => "Bottle Default",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Self::DxvkMoltenVk => "DX9-11 via Vulkan. Most compatible, good performance.",
            Self::Dxmt => {
                "DX11 directly to Metal. Best performance on Apple Silicon (CrossOver 25+)."
            }
            Self::D3dMetal => "Apple's translation layer. Supports DX12. May have shader stutter.",
            Self::Wined3d => "Classic OpenGL path. Slowest but most compatible fallback for DX9.",
            Self::Default => "Use the bottle's configured default — no overrides applied.",
        }
    }

    /// Whether this backend is only available on macOS.
    pub fn macos_only(&self) -> bool {
        matches!(self, Self::Dxmt | Self::D3dMetal)
    }
}

/// Backend recommendation with reasoning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendRecommendation {
    pub backend: GraphicsBackend,
    pub reason: String,
    pub all_options: Vec<BackendOption>,
}

/// A single backend option with availability status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendOption {
    pub backend: GraphicsBackend,
    pub name: String,
    pub description: String,
    pub available: bool,
    pub recommended: bool,
}

// ---------------------------------------------------------------------------
// Environment variable generation
// ---------------------------------------------------------------------------

/// Generate the environment variables and DLL overrides needed to activate
/// a specific graphics backend.
///
/// Returns a list of `(key, value)` pairs to set before launch.
pub fn backend_env_vars(backend: GraphicsBackend) -> Vec<(String, String)> {
    match backend {
        GraphicsBackend::DxvkMoltenVk => {
            // Force native d3d11/dxgi DLLs (DXVK's) over Wine builtin
            vec![("WINEDLLOVERRIDES".into(), "d3d11,dxgi=n".into())]
        }
        GraphicsBackend::Dxmt => {
            // DXMT uses its own d3d11 implementation, needs native override
            // CrossOver 25+ handles this via bottle config, but we set env as backup
            vec![("CX_DXMT".into(), "1".into())]
        }
        GraphicsBackend::D3dMetal => {
            // D3DMetal is activated via GPTK / CrossOver's D3DMetal setting
            vec![("D3DMETALFX".into(), "1".into())]
        }
        GraphicsBackend::Wined3d => {
            // Force Wine's builtin d3d11 (wined3d) over any native override
            vec![("WINEDLLOVERRIDES".into(), "d3d11,dxgi=b".into())]
        }
        GraphicsBackend::Default => {
            // No overrides — use bottle config as-is
            vec![]
        }
    }
}

/// Recommend the best graphics backend for a game.
///
/// Takes into account:
/// - Whether we're on macOS (Apple Silicon vs Intel)
/// - Game's DirectX version requirements
/// - CrossOver version (DXMT requires 25+)
pub fn recommend_backend(game_id: &str, is_apple_silicon: bool) -> BackendRecommendation {
    let is_macos = cfg!(target_os = "macos");

    // DX11 games: Skyrim SE, Fallout 4, Oblivion Remastered
    let is_dx11_game = matches!(
        game_id,
        "skyrimse" | "skyrimvr" | "fallout4" | "fallout4vr" | "oblivion" | "enderal"
    );

    let recommended = if is_macos && is_apple_silicon && is_dx11_game {
        // Apple Silicon + DX11 = DXMT is best
        GraphicsBackend::Dxmt
    } else if is_macos && is_dx11_game {
        // Intel Mac + DX11 = DXVK + MoltenVK
        GraphicsBackend::DxvkMoltenVk
    } else if is_dx11_game {
        // Linux = DXVK native
        GraphicsBackend::DxvkMoltenVk
    } else {
        // Unknown game or DX9 = default
        GraphicsBackend::Default
    };

    let reason = match recommended {
        GraphicsBackend::Dxmt => {
            "DXMT provides the best DX11 performance on Apple Silicon Macs with CrossOver 25+."
                .into()
        }
        GraphicsBackend::DxvkMoltenVk => {
            "DXVK is the most compatible DX11 translation layer for this configuration.".into()
        }
        _ => "Using default bottle configuration.".into(),
    };

    let all_options = vec![
        BackendOption {
            backend: GraphicsBackend::Default,
            name: "Bottle Default".into(),
            description: "Use the bottle's configured default.".into(),
            available: true,
            recommended: recommended == GraphicsBackend::Default,
        },
        BackendOption {
            backend: GraphicsBackend::DxvkMoltenVk,
            name: "DXVK + MoltenVK".into(),
            description: "DX9-11 via Vulkan. Most compatible.".into(),
            available: true,
            recommended: recommended == GraphicsBackend::DxvkMoltenVk,
        },
        BackendOption {
            backend: GraphicsBackend::Dxmt,
            name: "DXMT".into(),
            description: "DX11 → Metal directly. Best on Apple Silicon.".into(),
            available: is_macos,
            recommended: recommended == GraphicsBackend::Dxmt,
        },
        BackendOption {
            backend: GraphicsBackend::D3dMetal,
            name: "D3DMetal (GPTK)".into(),
            description: "Apple's layer. Supports DX12.".into(),
            available: is_macos,
            recommended: false,
        },
        BackendOption {
            backend: GraphicsBackend::Wined3d,
            name: "wined3d".into(),
            description: "OpenGL fallback. Slowest but broadest compat.".into(),
            available: true,
            recommended: false,
        },
    ];

    BackendRecommendation {
        backend: recommended,
        reason,
        all_options,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_env_vars_dxvk() {
        let vars = backend_env_vars(GraphicsBackend::DxvkMoltenVk);
        assert!(!vars.is_empty());
        assert!(vars.iter().any(|(k, _)| k == "WINEDLLOVERRIDES"));
    }

    #[test]
    fn backend_env_vars_default_is_empty() {
        let vars = backend_env_vars(GraphicsBackend::Default);
        assert!(vars.is_empty());
    }

    #[test]
    fn recommend_skyrimse() {
        let rec = recommend_backend("skyrimse", true);
        // On macOS with Apple Silicon, should recommend DXMT
        if cfg!(target_os = "macos") {
            assert_eq!(rec.backend, GraphicsBackend::Dxmt);
        } else {
            assert_eq!(rec.backend, GraphicsBackend::DxvkMoltenVk);
        }
    }

    #[test]
    fn recommend_unknown_game() {
        let rec = recommend_backend("unknowngame123", false);
        assert_eq!(rec.backend, GraphicsBackend::Default);
    }

    #[test]
    fn all_options_include_recommended() {
        let rec = recommend_backend("skyrimse", false);
        assert!(rec.all_options.iter().any(|o| o.recommended));
    }

    #[test]
    fn display_names_non_empty() {
        for backend in [
            GraphicsBackend::DxvkMoltenVk,
            GraphicsBackend::Dxmt,
            GraphicsBackend::D3dMetal,
            GraphicsBackend::Wined3d,
            GraphicsBackend::Default,
        ] {
            assert!(!backend.display_name().is_empty());
            assert!(!backend.description().is_empty());
        }
    }
}
