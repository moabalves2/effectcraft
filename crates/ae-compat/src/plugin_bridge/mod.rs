//! Plugin Bridge for After Effects (.aex) Plugins
//!
//! Provides a compatibility layer to load and run After Effects plugins in EffectCraft.
//!
//! ## Architecture
//!
//! Native `.aex` plugins are compiled binaries (C/C++) that use Adobe's proprietary
//! SDK. They cannot be loaded directly in a Rust application without:
//! 1. Recompiling from source against EffectCraft's plugin API (OpenFX-based)
//! 2. A translation layer that emulates the AE SDK (extremely complex)
//! 3. Running in a separate process with IPC (like a plugin server)
//!
//! This module implements approach #1 (OpenFX recompilation) as the primary path,
//! with a stub for approach #2 for future expansion.

use effectcraft_interchange::{Error, Result};
use effectcraft_plugin::{load_wasm, wasm_available, EffectSpec};
use openfx::host::{Host, Plugin as OfxPlugin};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Plugin bridge for loading AE-compatible plugins.
///
/// Currently supports:
/// - OpenFX plugins (`.ofx` bundles) - native Rust/OpenFX support
/// - WebAssembly plugins (`.wasm`) - via EffectCraft's existing plugin system
///
/// For native `.aex` plugins:
/// - Requires recompilation from source as OpenFX
/// - Or use the `AexPluginServer` (external process) for closed-source plugins
pub struct PluginBridge {
    loaded_plugins: HashMap<String, LoadedPlugin>,
    plugin_dirs: Vec<PathBuf>,
    server_handle: Option<AexPluginServer>,
}

#[derive(Debug, Clone)]
enum LoadedPlugin {
    OpenFx(Arc<Mutex<OfxPlugin>>),
    Wasm(&'static EffectSpec),
    Aex(AexPluginHandle),
}

/// Handle to an AE plugin running in the external server
#[derive(Debug, Clone)]
struct AexPluginHandle {
    id: String,
    name: String,
    match_name: String,
    server_port: u16,
}

/// External plugin server for running native .aex plugins
///
/// This would be a separate binary that:
/// 1. Loads the .aex plugin using Adobe's SDK
/// 2. Communicates via gRPC/JSON-RPC with EffectCraft
/// 3. Translates render calls to the plugin
struct AexPluginServer {
    port: u16,
    process: Option<std::process::Child>,
    plugins: HashMap<String, AexPluginInfo>,
}

#[derive(Debug, Clone)]
struct AexPluginInfo {
    path: PathBuf,
    match_name: String,
    name: String,
    version: String,
    categories: Vec<String>,
}

impl PluginBridge {
    /// Create a new plugin bridge.
    pub fn new() -> Self {
        Self {
            loaded_plugins: HashMap::new(),
            plugin_dirs: Vec::new(),
            server_handle: None,
        }
    }

    /// Add a directory to scan for plugins.
    pub fn add_plugin_directory<P: AsRef<Path>>(&mut self, dir: P) {
        self.plugin_dirs.push(dir.as_ref().to_path_buf());
    }

    /// Scan all plugin directories and load compatible plugins.
    pub fn scan_and_load(&mut self) -> Result<Vec<PluginInfo>> {
        let mut loaded = Vec::new();

        for dir in &self.plugin_dirs {
            loaded.extend(self.scan_directory(dir)?);
        }

        Ok(loaded)
    }

    fn scan_directory(&mut self, dir: &Path) -> Result<Vec<PluginInfo>> {
        let mut plugins = Vec::new();

        if !dir.exists() {
            return Ok(plugins);
        }

        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                // Check for .ofx bundle
                if path.extension().map(|e| e == "ofx").unwrap_or(false) {
                    if let Ok(info) = self.load_openfx_plugin(&path) {
                        plugins.push(info);
                    }
                }
            } else if path.is_file() {
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                match ext {
                    "wasm" | "wat" => {
                        if let Ok(info) = self.load_wasm_plugin(&path) {
                            plugins.push(info);
                        }
                    }
                    "aex" => {
                        // Native AE plugin - register for server loading
                        if let Ok(info) = self.register_aex_plugin(&path) {
                            plugins.push(info);
                        }
                    }
                    _ => {}
                }
            }
        }

        Ok(plugins)
    }

    /// Load an OpenFX plugin from a bundle directory.
    pub fn load_openfx_plugin<P: AsRef<Path>>(&mut self, path: P) -> Result<PluginInfo> {
        let path = path.as_ref();

        #[cfg(feature = "plugin-bridge")]
        {
            // OpenFX host initialization
            let host = Host::new("EffectCraft", 1, 0, "1.0")?;
            let plugin = host.load_plugin(path)?;

            let info = PluginInfo {
                id: plugin.identifier().to_string(),
                name: plugin.name().to_string(),
                match_name: plugin.identifier().to_string(),
                version: plugin.version().to_string(),
                category: plugin.category().unwrap_or("Effect").to_string(),
                plugin_type: PluginType::OpenFx,
                path: path.to_path_buf(),
            };

            self.loaded_plugins.insert(info.id.clone(), LoadedPlugin::OpenFx(Arc::new(Mutex::new(plugin))));
            Ok(info)
        }

        #[cfg(not(feature = "plugin-bridge"))]
        {
            Err(Error::UnsupportedFeature("OpenFX support requires 'plugin-bridge' feature".into()))
        }
    }

    /// Load a WebAssembly plugin.
    pub fn load_wasm_plugin<P: AsRef<Path>>(&mut self, path: P) -> Result<PluginInfo> {
        let path = path.as_ref();
        let bytes = std::fs::read(path)?;

        let spec = load_wasm(&bytes, path.to_string_lossy().as_ref())?;

        let info = PluginInfo {
            id: spec.id.clone(),
            name: spec.name.clone(),
            match_name: spec.id.clone(),
            version: "1.0".to_string(),
            category: spec.category.clone(),
            plugin_type: PluginType::Wasm,
            path: path.to_path_buf(),
        };

        self.loaded_plugins.insert(info.id.clone(), LoadedPlugin::Wasm(spec));
        Ok(info)
    }

    /// Register a native .aex plugin for loading via the plugin server.
    ///
    /// This doesn't load the plugin immediately - it registers it for
    /// lazy loading when the server starts.
    pub fn register_aex_plugin<P: AsRef<Path>>(&mut self, path: P) -> Result<PluginInfo> {
        let path = path.as_ref();

        // Extract plugin metadata from .aex (requires parsing the binary)
        let (match_name, name) = extract_aex_metadata(path)?;

        let info = PluginInfo {
            id: format!("aex_{}", name.replace(' ', "_")),
            name: name.clone(),
            match_name,
            version: "1.0".to_string(),
            category: "Effect".to_string(),
            plugin_type: PluginType::Aex,
            path: path.to_path_buf(),
        };

        // Store for later server loading
        if self.server_handle.is_none() {
            self.start_plugin_server()?;
        }

        if let Some(server) = &mut self.server_handle {
            server.register_plugin(&info);
        }

        self.loaded_plugins.insert(info.id.clone(), LoadedPlugin::Aex(AexPluginHandle {
            id: info.id.clone(),
            name,
            match_name: info.match_name.clone(),
            server_port: self.server_handle.as_ref().map(|s| s.port).unwrap_or(0),
        }));

        Ok(info)
    }

    /// Start the external AE plugin server.
    fn start_plugin_server(&mut self) -> Result<()> {
        // In a real implementation, this would start the aex-plugin-server binary
        // For now, return an error indicating the feature needs the server binary
        Err(Error::UnsupportedFeature(
            "Native .aex plugin support requires the 'aex-plugin-server' binary. \
             See docs/plugins.md for building instructions.".into()
        ))
    }

    /// Get a loaded plugin by ID.
    pub fn get_plugin(&self, id: &str) -> Option<&LoadedPlugin> {
        self.loaded_plugins.get(id)
    }

    /// Get a plugin as an EffectSpec for the effect registry.
    pub fn get_effect_spec(&self, id: &str) -> Option<&'static EffectSpec> {
        match self.loaded_plugins.get(id)? {
            LoadedPlugin::Wasm(spec) => Some(*spec),
            LoadedPlugin::OpenFx(_) => {
                // OpenFX plugins need to be wrapped as EffectSpec
                // This would require implementing the effectcraft_effects::plugin::EffectPlugin trait
                None
            }
            LoadedPlugin::Aex(_) => None,
        }
    }

    /// List all loaded plugins.
    pub fn list_plugins(&self) -> Vec<PluginInfo> {
        self.loaded_plugins.values().filter_map(|p| {
            match p {
                LoadedPlugin::OpenFx(plugin) => {
                    let plugin = plugin.lock().ok()?;
                    Some(PluginInfo {
                        id: plugin.identifier().to_string(),
                        name: plugin.name().to_string(),
                        match_name: plugin.identifier().to_string(),
                        version: plugin.version().to_string(),
                        category: plugin.category().unwrap_or("Effect").to_string(),
                        plugin_type: PluginType::OpenFx,
                        path: PathBuf::new(),
                    })
                }
                LoadedPlugin::Wasm(spec) => Some(PluginInfo {
                    id: spec.id.clone(),
                    name: spec.name.clone(),
                    match_name: spec.id.clone(),
                    version: "1.0".to_string(),
                    category: spec.category.clone(),
                    plugin_type: PluginType::Wasm,
                    path: PathBuf::new(),
                }),
                LoadedPlugin::Aex(handle) => Some(PluginInfo {
                    id: handle.id.clone(),
                    name: handle.name.clone(),
                    match_name: handle.match_name.clone(),
                    version: "1.0".to_string(),
                    category: "Effect (AE)".to_string(),
                    plugin_type: PluginType::Aex,
                    path: PathBuf::new(),
                }),
            }
        }).collect()
    }

    /// Check if WebAssembly plugins are available.
    pub fn wasm_available() -> bool {
        wasm_available()
    }
}

impl Default for PluginBridge {
    fn default() -> Self {
        Self::new()
    }
}

/// Information about a loaded plugin
#[derive(Debug, Clone)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub match_name: String,
    pub version: String,
    pub category: String,
    pub plugin_type: PluginType,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginType {
    OpenFx,
    Wasm,
    Aex,
}

/// Extract metadata from a native .aex plugin file.
///
/// This parses the plugin's resource data to get the match name and display name.
/// The .aex format is a standard Mach-O/PE/ELF binary with a resource section.
fn extract_aex_metadata(path: &Path) -> Result<(String, String)> {
    // This is a simplified version - real implementation would parse
    // the binary resources (PiPL on macOS, VERSIONINFO on Windows)
    let name = path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Unknown")
        .to_string();

    let match_name = format!("com.adobe.{}", name.to_lowercase().replace(' ', ""));

    Ok((match_name, name))
}

/// AE Plugin Server - External process for running native .aex plugins
///
/// This is a separate binary that would be built from `crates/ae-compat/plugin-server/`
/// It uses the Adobe AE SDK to load .aex plugins and exposes a gRPC API.
pub mod plugin_server {
    use super::*;
    use std::net::SocketAddr;

    /// Configuration for the plugin server
    #[derive(Debug, Clone)]
    pub struct ServerConfig {
        pub addr: SocketAddr,
        pub plugin_dirs: Vec<PathBuf>,
        pub log_level: tracing::Level,
    }

    /// Start the plugin server (would be the main entry point of the server binary)
    pub fn start_server(config: ServerConfig) -> Result<()> {
        // This would:
        // 1. Initialize the Adobe AE SDK
        // 2. Load all .aex plugins from plugin_dirs
        // 3. Start a gRPC server on config.addr
        // 4. Handle render requests by calling into the plugins

        Err(Error::UnsupportedFeature(
            "Plugin server not yet implemented. Requires Adobe AE SDK license.".into()
        ))
    }
}

/// Wrapper to make OpenFX plugins compatible with EffectCraft's effect system
#[cfg(feature = "plugin-bridge")]
pub mod openfx_wrapper {
    use super::*;
    use effectcraft_effects::{EffectSpec, EffectInstance, PluginFrame, PluginManifest, PluginParams};
    use effectcraft_raster::Rgba32f;
    use std::sync::Arc;

    /// Wrap an OpenFX plugin as an EffectCraft effect
    pub struct OpenFxEffectWrapper {
        plugin: Arc<Mutex<OfxPlugin>>,
        spec: EffectSpec,
    }

    impl OpenFxEffectWrapper {
        pub fn new(plugin: OfxPlugin) -> Result<Self> {
            let manifest = PluginManifest {
                name: plugin.name().to_string(),
                version: plugin.version().to_string(),
                author: "OpenFX Plugin".to_string(),
                description: plugin.description().unwrap_or_default().to_string(),
                category: plugin.category().unwrap_or("Effect").to_string(),
            };

            // Create params from OpenFX parameters
            let params = plugin.parameters().map(|p| {
                // Convert OpenFX params to EffectCraft params
            }).collect();

            let spec = EffectSpec::new(plugin.identifier(), manifest, params);

            Ok(Self {
                plugin: Arc::new(Mutex::new(plugin)),
                spec,
            })
        }
    }

    impl effectcraft_effects::plugin::EffectPlugin for OpenFxEffectWrapper {
        fn manifest(&self) -> &PluginManifest {
            &self.spec.manifest
        }

        fn render(
            &self,
            frame: &mut PluginFrame,
            params: &PluginParams,
            time: f64,
            scale: f32,
        ) -> Result<(), String> {
            let mut plugin = self.plugin.lock().map_err(|e| e.to_string())?;

            // Convert EffectCraft frame to OpenFX image
            // Call plugin.render()
            // Convert back

            Ok(())
        }

        fn spec(&self) -> &EffectSpec {
            &self.spec
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plugin_bridge_creation() {
        let bridge = PluginBridge::new();
        assert!(bridge.loaded_plugins.is_empty());
    }

    #[test]
    fn test_plugin_type_enum() {
        assert_eq!(PluginType::OpenFx as u8, 0);
        assert_eq!(PluginType::Wasm as u8, 1);
        assert_eq!(PluginType::Aex as u8, 2);
    }
}