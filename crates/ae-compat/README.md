# effectcraft-ae-compat

After Effects compatibility layer for EffectCraft — modular, isolated, zero-core-changes.

## Overview

This crate adds support for Adobe After Effects formats to EffectCraft **without modifying any core code**. It's designed as a standalone Cargo workspace member that can be enabled/disabled via features.

## Features

| Feature | Description | Dependencies |
|---------|-------------|--------------|
| `aep` | Binary `.aep` parser | `nom`, `flate2`, `bzip2` |
| `aepx` | XML `.aepx` parser | `quick-xml` |
| `jsx` | ExtendScript/JSX runtime | `boa_engine` (shared with `expr`) |
| `plugin-bridge` | Plugin compatibility (OpenFX) | `openfx` |

**Default features**: `aep`, `aepx`, `jsx`, `plugin-bridge`

## Integration

### 1. Add to Workspace

In the root `Cargo.toml`, add to `workspace.members`:

```toml
members = [
    "crates/*",
    "apps/*",
    "xtask",
    "crates/ae-compat",  # Add this line
]
```

### 2. Enable in Apps

In `apps/effectcraft/Cargo.toml` (or `effectcraft-cli`):

```toml
[dependencies]
effectcraft-ae-compat = { path = "../../crates/ae-compat", features = ["aep", "aepx", "jsx", "plugin-bridge"] }
```

### 3. Register Import Formats

In your app initialization (e.g., `apps/effectcraft/src/main.rs`):

```rust
use effectcraft_ae_compat::{register_ae_import_formats, detect_ae_format};

fn main() {
    // Register AE import handlers
    register_ae_import_formats();

    // In your file import handler:
    if let Some(fmt) = detect_ae_format(path) {
        let bytes = std::fs::read(path)?;
        let opts = ImportOptions::default();
        let mut probe = |p| None; // Your footage probe function
        let result = effectcraft_ae_compat::import_ae_interchange(&mut project, &bytes, fmt, &opts, &mut probe)?;
        // Handle result...
    }
}
```

## Usage

### Import AEP/AEPX Files

```rust
use effectcraft_ae_compat::import_ae_project;
use effectcraft_project::Project;

let project: Project = import_ae_project("project.aep")?;
// Or with explicit format:
let project: Project = effectcraft_ae_compat::aepx::parse_aepx(&bytes)?;
```

### Run JSX Scripts

```rust
use effectcraft_ae_compat::{AeScriptRunner, JsxRuntime};
use effectcraft_project::Project;

let project = Project::default();
let mut runner = AeScriptRunner::new(project)?;

// Run a script file
let modified_project = runner.run_file("script.jsx")?;

// Or run from source
let modified_project = runner.run_source("app.project.save()", "save.jsx")?;
```

### Evaluate AE Expressions

```rust
use effectcraft_ae_compat::preprocess_ae_expression;
use effectcraft_expr::eval_standalone;
use effectcraft_project::{Project, ItemId, LayerId};
use effectcraft_time::Tick;

// Preprocess AE expression syntax (handles array math, etc.)
let ae_expr = "wiggle(2, 50) + [100, 100]";
let processed = preprocess_ae_expression(ae_expr);

// Evaluate using EffectCraft's deterministic engine
let value = eval_standalone(&project, comp_id, layer_id, "transform/position", &processed, 1.0)?;
```

### Load Plugins

```rust
use effectcraft_ae_compat::PluginBridge;

let mut bridge = PluginBridge::new();
bridge.add_plugin_directory("/path/to/plugins");
bridge.add_plugin_directory("/Applications/Adobe After Effects/Plug-ins");

let plugins = bridge.scan_and_load()?;
for plugin in plugins {
    println!("Loaded: {} ({})", plugin.name, plugin.plugin_type);
}

// Get EffectSpec for registry
if let Some(spec) = bridge.get_effect_spec("some_plugin_id") {
    // Register with effectcraft_effects
}
```

## Architecture

```
effectcraft-ae-compat/
├── aep/              # Binary AEP parser
│   └── mod.rs        # Chunk-based parser with gzip support
├── aepx/             # XML AEPX parser
│   └── mod.rs        # quick-xml streaming parser
├── jsx/              # JSX/ExtendScript runtime
│   └── mod.rs        # Boa engine with AE globals
├── plugin_bridge/    # Plugin compatibility
│   └── mod.rs        # OpenFX + Wasm + .aex server stub
├── interchange_integration.rs  # Import system hooks
└── lib.rs            # Public API
```

## AEP Parser Details

The binary `.aep` format is reverse-engineered (Adobe doesn't document it). This parser:

- Handles gzip-compressed AEP files (common in CC 2019+)
- Parses chunk-based structure: `[tag: u32][size: u32][data...]`
- Extracts: compositions, layers, footage, keyframes, effects, markers
- Maps 298+ AE effect match names to EffectCraft effect IDs
- Supports all layer types: footage, solid, null, text, shape, camera, light, adjustment, comp

**Limitations**: Some advanced features (3D camera tracker, Roto Brush, expressions referencing external files) may not import perfectly.

## AEPX Parser Details

The `.aepx` format is XML-based (introduced in CS6). This parser:

- Uses streaming XML parser (`quick-xml`) for memory efficiency
- Handles large projects without loading entire DOM
- Same feature coverage as AEP parser
- More robust for version-forward compatibility

## JSX Runtime Details

- Uses the **same Boa engine** as `effectcraft_expr` (deterministic, no JIT)
- Adds AE globals: `app`, `project`, `app.project`, `$.writeln`, etc.
- Supports `app.beginUndoGroup()` / `app.endUndoGroup()`
- Scripts can create/modify layers, apply effects, set keyframes
- Expression evaluation delegates to `effectcraft_expr` for consistency

## Plugin Bridge Details

### OpenFX (Recommended)

Recompile AE plugins as OpenFX (many open-source plugins support this):
```bash
# Example: recompile an open-source AE plugin as OFX
cargo build --release -p my-plugin --features openfx
```

### WebAssembly (Native)

EffectCraft already supports `.wasm` plugins natively via `effectcraft_plugin`.

### Native .aex (Stub)

Closed-source `.aex` plugins require an external **plugin server** (separate binary) that:
1. Loads the plugin using Adobe's SDK (requires license)
2. Exposes a gRPC/JSON-RPC API
3. EffectCraft connects via `PluginBridge::register_aex_plugin()`

The server binary is not included (Adobe SDK license restrictions). See `plugin_bridge/plugin_server.rs` for the interface.

## Testing

```bash
# Run all tests
cargo test -p effectcraft-ae-compat

# Test with specific features
cargo test -p effectcraft-ae-compat --features aep,aepx,jsx

# Integration test with EffectCraft
cargo test -p effectcraft --features ae-compat
```

## License

MIT OR Apache-2.0 (same as EffectCraft)