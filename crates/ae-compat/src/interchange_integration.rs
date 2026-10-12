//! Interchange Integration for AE Compatibility
//!
//! This module integrates the AE compatibility layer into EffectCraft's
//! existing import system (File ▸ Import).

use effectcraft_ae_compat::{import_ae_project, AeFormat};
use effectcraft_interchange::{Error, Result, TimelineFormat, ImportOptions, ImportResult};
use effectcraft_project::Project;
use std::path::Path;

/// AE project import format for the interchange system
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AeImportFormat {
    Aep,
    Aepx,
}

impl From<AeImportFormat> for TimelineFormat {
    fn from(fmt: AeImportFormat) -> Self {
        // These are custom formats not in the standard TimelineFormat enum
        // They would need to be added to TimelineFormat in effectcraft-interchange
        match fmt {
            AeImportFormat::Aep => TimelineFormat::Custom("aep"),
            AeImportFormat::Aepx => TimelineFormat::Custom("aepx"),
        }
    }
}

/// Import an After Effects project via the interchange system.
///
/// This function can be called from the UI (File ▸ Import ▸ After Effects Project)
/// or from the CLI.
pub fn import_ae_interchange(
    project: &mut Project,
    bytes: &[u8],
    format: AeImportFormat,
    opts: &ImportOptions,
    probe: &mut dyn FnMut(&str) -> Option<effectcraft_project::Footage>,
) -> Result<ImportResult> {
    // Parse the AE project
    let ae_project = import_ae_project_from_bytes(bytes, format)?;

    // Merge into the current project
    let folder_name = if opts.name.is_empty() {
        "Imported AE Project".to_string()
    } else {
        opts.name.clone()
    };

    let folder_id = project.add_item(&folder_name, effectcraft_color::Label::None, None, effectcraft_project::ItemKind::Folder);

    let mut result = ImportResult {
        format: Some(format.into()),
        folder: folder_id,
        comps: Vec::new(),
        all_comps: Vec::new(),
        items: Vec::new(),
        missing: Vec::new(),
        warnings: Vec::new(),
    };

    // Add compositions
    for comp in ae_project.comps() {
        let comp_id = project.add_comp(comp.clone());
        result.comps.push(comp_id);
        result.all_comps.push(comp_id);
    }

    // Add footage
    for footage in ae_project.footage() {
        let item_id = project.add_item(&footage.name, effectcraft_color::Label::None, None, effectcraft_project::ItemKind::Footage(footage.clone()));
        result.items.push(item_id);
    }

    // Add solids
    for solid in ae_project.solids() {
        let item_id = project.add_item(&solid.name, effectcraft_color::Label::None, None, effectcraft_project::ItemKind::Solid(solid.clone()));
        result.items.push(item_id);
    }

    Ok(result)
}

/// Parse AE project from bytes with format hint
fn import_ae_project_from_bytes(bytes: &[u8], format: AeImportFormat) -> Result<Project> {
    match format {
        AeImportFormat::Aep => {
            #[cfg(feature = "aep")]
            {
                effectcraft_ae_compat::aep::parse_aep(bytes)
            }
            #[cfg(not(feature = "aep"))]
            {
                Err(Error::UnsupportedFeature("AEP import requires 'aep' feature on effectcraft-ae-compat".into()))
            }
        }
        AeImportFormat::Aepx => {
            #[cfg(feature = "aepx")]
            {
                effectcraft_ae_compat::aepx::parse_aepx(bytes)
            }
            #[cfg(not(feature = "aepx"))]
            {
                Err(Error::UnsupportedFeature("AEPX import requires 'aepx' feature on effectcraft-ae-compat".into()))
            }
        }
    }
}

/// Detect AE format from file extension
pub fn detect_ae_format(path: &Path) -> Option<AeImportFormat> {
    let ext = path.extension()?.to_str()?.to_lowercase();
    match ext.as_str() {
        "aep" => Some(AeImportFormat::Aep),
        "aepx" => Some(AeImportFormat::Aepx),
        _ => None,
    }
}

/// Register AE import formats in the interchange system.
///
/// Call this during app initialization to enable AE import.
pub fn register_ae_import_formats() {
    // This would register the formats in effectcraft_interchange::TimelineFormat
    // For now, the formats are handled via the custom import function above
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_format() {
        assert_eq!(detect_ae_format(Path::new("test.aep")), Some(AeImportFormat::Aep));
        assert_eq!(detect_ae_format(Path::new("test.aepx")), Some(AeImportFormat::Aepx));
        assert_eq!(detect_ae_format(Path::new("test.ecproj")), None);
    }
}