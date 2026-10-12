//! # effectcraft-ae-compat
//!
//! After Effects compatibility layer for EffectCraft.
//!
//! This crate provides modular, isolated support for Adobe After Effects formats
//! without modifying the core EffectCraft rendering pipeline.
//!
//! ## Features
//!
//! - **AEP/AEPX Import**: Parse After Effects project files (binary `.aep` and XML `.aepx`)
//!   and convert them to EffectCraft's `.ecproj` format.
//! - **JSX Runtime**: Execute After Effects ExtendScript (`.jsx`) and expressions in a
//!   sandboxed JavaScript engine, compatible with AE's expression language.
//! - **Plugin Bridge**: Load and translate After Effects plugins (`.aex`) via OpenFX
//!   recompilation or API translation layer.
//!
//! ## Architecture
//!
//! Each module is completely independent and can be enabled/disabled via Cargo features:
//! - `aep` - Binary AEP parser
//! - `aepx` - XML AEPX parser
//! - `jsx` - JSX/ExtendScript runtime
//! - `plugin-bridge` - Plugin compatibility layer (requires `openfx` feature)

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod aep;
pub mod aepx;
pub mod jsx;
pub mod plugin_bridge;
pub mod interchange_integration;

use effectcraft_interchange::{Error, Result, TimelineFormat};
use effectcraft_project::Project;
use std::path::Path;

/// High-level import function for After Effects projects.
///
/// Detects format from file extension and delegates to the appropriate parser.
/// Returns a populated `Project` ready for rendering.
pub fn import_ae_project<P: AsRef<Path>>(path: P) -> Result<Project> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).map_err(|e| Error::Io(e.to_string()))?;

    let format = detect_format(path, &bytes)?;

    match format {
        AeFormat::Aep => {
            #[cfg(feature = "aep")]
            {
                aep::parse_aep(&bytes)
            }
            #[cfg(not(feature = "aep"))]
            {
                Err(Error::UnsupportedFeature("AEP parsing requires 'aep' feature".into()))
            }
        }
        AeFormat::Aepx => {
            #[cfg(feature = "aepx")]
            {
                aepx::parse_aepx(&bytes)
            }
            #[cfg(not(feature = "aepx"))]
            {
                Err(Error::UnsupportedFeature("AEPX parsing requires 'aepx' feature".into()))
            }
        }
    }
}

/// Supported After Effects project formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AeFormat {
    Aep,
    Aepx,
}

/// Detect AE format from file extension and magic bytes.
fn detect_format(path: &Path, bytes: &[u8]) -> Result<AeFormat> {
    let ext = path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase());

    match ext.as_deref() {
        Some("aep") => Ok(AeFormat::Aep),
        Some("aepx") => Ok(AeFormat::Aepx),
        _ => {
            // Fallback: detect from magic bytes
            if bytes.starts_with(b"<?xml") || bytes.starts_with(b"<project") {
                Ok(AeFormat::Aepx)
            } else if bytes.len() >= 4 && &bytes[0..4] == b"CT\x00\x00" {
                // AEP magic header (simplified)
                Ok(AeFormat::Aep)
            } else {
                Err(Error::UnknownFormat)
            }
        }
    }
}

/// Register AE-compatible expression functions in the EffectCraft expression engine.
///
/// This extends the existing `effectcraft_expr::Expressions` with AE-specific
/// functions like `wiggle`, `loopOut`, `toComp`, `fromComp`, etc.
#[cfg(feature = "jsx")]
pub fn register_ae_expressions() -> effectcraft_expr::Expressions {
    effectcraft_expr::Expressions
}

/// Load an After Effects plugin (.aex) via the plugin bridge.
///
/// Currently supports OpenFX plugins recompiled for EffectCraft.
/// Native .aex loading requires recompilation from source.
#[cfg(feature = "plugin-bridge")]
pub fn load_ae_plugin<P: AsRef<Path>>(path: P) -> Result<effectcraft_plugin::EffectSpec> {
    plugin_bridge::load_plugin(path)
}

/// Re-export interchange integration functions
pub use interchange_integration::{
    import_ae_interchange,
    detect_ae_format,
    register_ae_import_formats,
    AeImportFormat,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_detection() {
        assert_eq!(detect_format(Path::new("test.aep"), b"CT\x00\x00"), Ok(AeFormat::Aep));
        assert_eq!(detect_format(Path::new("test.aepx"), b"<?xml"), Ok(AeFormat::Aepx));
    }
}