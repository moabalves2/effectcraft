//! AEPX (XML) Parser for After Effects Project Files
//!
//! Parses the XML-based `.aepx` format introduced in After Effects CS6.
//! The format is essentially a serialized version of the AE project structure.

use effectcraft_interchange::{Error, Result};
use effectcraft_keyframe::{Keyframe, Value, Interpolation, TemporalInterpolation, SpatialInterpolation};
use effectcraft_project::{
    AlphaMode, BlendMode, Comp, Footage, FootageKind, ItemId, ItemKind, Label, Layer, LayerSource,
    Marker, Node, ParamUi, Project, Property, PropertyId, Solid,
};
use effectcraft_time::{FrameRate, Tick};
use quick_xml::{
    events::{BytesStart, Event},
    Reader,
};
use std::collections::HashMap;
use std::io::BufRead;
use std::str::FromStr;

/// Parse an AEPX (XML) file into an EffectCraft Project.
pub fn parse_aepx(bytes: &[u8]) -> Result<Project> {
    let mut parser = AepxParser::new(bytes);
    parser.parse()
}

/// Internal AEPX parser state
struct AepxParser<'a> {
    reader: Reader<&'a [u8]>,
    project: Project,
    comps: HashMap<String, Comp>,
    footage_map: HashMap<String, Footage>,
    layers: Vec<LayerData>,
    current_element: String,
    string_buffer: String,
}

struct LayerData {
    id: String,
    comp_id: String,
    name: String,
    layer_type: LayerType,
    source_id: Option<String>,
    transform: effectcraft_project::Transform,
    properties: Vec<PropertyData>,
    effects: Vec<EffectData>,
    masks: Vec<MaskData>,
    parent: Option<String>,
    blend_mode: BlendMode,
    enabled: bool,
    solo: bool,
    locked: bool,
    shy: bool,
}

struct PropertyData {
    path: String,
    name: String,
    value: Value,
    keyframes: Vec<KeyframeData>,
    expression: Option<String>,
}

struct KeyframeData {
    time: Tick,
    value: Value,
    interpolation: Interpolation,
}

struct EffectData {
    match_name: String,
    name: String,
    enabled: bool,
    params: Vec<EffectParam>,
}

struct EffectParam {
    name: String,
    match_name: String,
    value: Value,
    keyframes: Vec<KeyframeData>,
    expression: Option<String>,
}

struct MaskData {
    name: String,
    path: effectcraft_path::Path,
    mode: MaskMode,
    feather: f32,
    opacity: f32,
    expansion: f32,
    inverted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LayerType {
    Footage,
    Solid,
    Null,
    Adjustment,
    Text,
    Shape,
    Camera,
    Light,
    Comp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MaskMode {
    None,
    Add,
    Subtract,
    Intersect,
    Lighten,
    Darken,
    Difference,
}

impl<'a> AepxParser<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        let mut reader = Reader::from_reader(bytes);
        reader.config_mut().trim_text(true);
        reader.config_mut().expand_empty_elements(true);

        Self {
            reader,
            project: Project::default(),
            comps: HashMap::new(),
            footage_map: HashMap::new(),
            layers: Vec::new(),
            current_element: String::new(),
            string_buffer: String::new(),
        }
    }

    fn parse(&mut self) -> Result<Project> {
        let mut buf = Vec::new();

        loop {
            match self.reader.read_event_into(&mut buf) {
                Ok(Event::Start(e)) => self.handle_start(e)?,
                Ok(Event::End(e)) => self.handle_end(e)?,
                Ok(Event::Text(e)) => {
                    self.string_buffer = e.unescape()?.into_owned();
                }
                Ok(Event::Eof) => break,
                Ok(_) => {}
                Err(e) => return Err(Error::InvalidFormat(format!("XML parse error: {}", e))),
            }
            buf.clear();
        }

        self.build_project()
    }

    fn handle_start(&mut self, e: BytesStart) -> Result<()> {
        let name = std::str::from_utf8(e.name().as_ref())?;
        self.current_element = name.to_string();

        match name {
            "project" => self.parse_project_attributes(e)?,
            "composition" => self.parse_composition_start(e)?,
            "layer" => self.parse_layer_start(e)?,
            "footage" => self.parse_footage_start(e)?,
            "property" => self.parse_property_start(e)?,
            "keyframe" => self.parse_keyframe_start(e)?,
            "effect" => self.parse_effect_start(e)?,
            "effectparam" => self.parse_effect_param_start(e)?,
            "mask" => self.parse_mask_start(e)?,
            "marker" => self.parse_marker_start(e)?,
            _ => {}
        }
        Ok(())
    }

    fn handle_end(&mut self, e: quick_xml::events::BytesEnd) -> Result<()> {
        let name = std::str::from_utf8(e.name().as_ref())?;

        match name {
            "composition" => self.finish_composition()?,
            "layer" => self.finish_layer()?,
            "footage" => self.finish_footage()?,
            "property" => self.finish_property()?,
            "keyframe" => self.finish_keyframe()?,
            "effect" => self.finish_effect()?,
            "effectparam" => self.finish_effect_param()?,
            "mask" => self.finish_mask()?,
            "marker" => self.finish_marker()?,
            _ => {}
        }
        Ok(())
    }

    fn parse_project_attributes(&mut self, e: BytesStart) -> Result<()> {
        for attr in e.attributes() {
            let attr = attr?;
            let key = std::str::from_utf8(attr.key.as_ref())?;
            let value = std::str::from_utf8(&attr.value)?;

            match key {
                "frameRate" => self.project.frame_rate = FrameRate::from_f64(value.parse()?),
                "duration" => self.project.duration = Tick::from_seconds_f64(value.parse()?),
                "width" => self.project.width = value.parse()?,
                "height" => self.project.height = value.parse()?,
                "bgColor" => self.project.background_color = parse_color(value)?,
                _ => {}
            }
        }
        Ok(())
    }

    fn parse_composition_start(&mut self, e: BytesStart) -> Result<()> {
        let mut comp = Comp::default();
        let mut id = String::new();

        for attr in e.attributes() {
            let attr = attr?;
            let key = std::str::from_utf8(attr.key.as_ref())?;
            let value = std::str::from_utf8(&attr.value)?;

            match key {
                "id" => id = value.to_string(),
                "name" => comp.name = value.to_string(),
                "width" => comp.width = value.parse()?,
                "height" => comp.height = value.parse()?,
                "frameRate" => comp.frame_rate = FrameRate::from_f64(value.parse()?),
                "duration" => comp.duration = Tick::from_seconds_f64(value.parse()?),
                "bgColor" => comp.background_color = parse_color(value)?,
                _ => {}
            }
        }

        comp.id = ItemId::new(id.parse().unwrap_or(0));
        self.comps.insert(id, comp);
        Ok(())
    }

    fn finish_composition(&mut self) -> Result<()> {
        // Composition data is built incrementally
        Ok(())
    }

    fn parse_layer_start(&mut self, e: BytesStart) -> Result<()> {
        let mut layer = LayerData {
            id: String::new(),
            comp_id: String::new(),
            name: String::new(),
            layer_type: LayerType::Footage,
            source_id: None,
            transform: effectcraft_project::Transform::default(),
            properties: Vec::new(),
            effects: Vec::new(),
            masks: Vec::new(),
            parent: None,
            blend_mode: BlendMode::Normal,
            enabled: true,
            solo: false,
            locked: false,
            shy: false,
        };

        for attr in e.attributes() {
            let attr = attr?;
            let key = std::str::from_utf8(attr.key.as_ref())?;
            let value = std::str::from_utf8(&attr.value)?;

            match key {
                "id" => layer.id = value.to_string(),
                "comp" => layer.comp_id = value.to_string(),
                "name" => layer.name = value.to_string(),
                "type" => layer.layer_type = parse_layer_type(value),
                "source" => layer.source_id = Some(value.to_string()),
                "parent" => layer.parent = Some(value.to_string()),
                "blendMode" => layer.blend_mode = parse_blend_mode(value),
                "enabled" => layer.enabled = value == "true",
                "solo" => layer.solo = value == "true",
                "locked" => layer.locked = value == "true",
                "shy" => layer.shy = value == "true",
                _ => {}
            }
        }

        self.layers.push(layer);
        Ok(())
    }

    fn finish_layer(&mut self) -> Result<()> {
        // Layer will be processed in build_project
        Ok(())
    }

    fn parse_footage_start(&mut self, e: BytesStart) -> Result<()> {
        let mut footage = Footage::default();
        let mut id = String::new();

        for attr in e.attributes() {
            let attr = attr?;
            let key = std::str::from_utf8(attr.key.as_ref())?;
            let value = std::str::from_utf8(&attr.value)?;

            match key {
                "id" => id = value.to_string(),
                "name" => footage.name = value.to_string(),
                "path" => footage.path = value.to_string(),
                "width" => footage.width = value.parse()?,
                "height" => footage.height = value.parse()?,
                "type" => footage.kind = parse_footage_kind(value),
                _ => {}
            }
        }

        footage.id = ItemId::new(id.parse().unwrap_or(0));
        self.footage_map.insert(id, footage);
        Ok(())
    }

    fn finish_footage(&mut self) -> Result<()> {
        Ok(())
    }

    fn parse_property_start(&mut self, e: BytesStart) -> Result<()> {
        let mut prop = PropertyData {
            path: String::new(),
            name: String::new(),
            value: Value::Number(0.0),
            keyframes: Vec::new(),
            expression: None,
        };

        for attr in e.attributes() {
            let attr = attr?;
            let key = std::str::from_utf8(attr.key.as_ref())?;
            let value = std::str::from_utf8(&attr.value)?;

            match key {
                "path" => prop.path = value.to_string(),
                "name" => prop.name = value.to_string(),
                "value" => prop.value = parse_value(value)?,
                "expression" => prop.expression = Some(value.to_string()),
                _ => {}
            }
        }

        if let Some(layer) = self.layers.last_mut() {
            layer.properties.push(prop);
        }
        Ok(())
    }

    fn finish_property(&mut self) -> Result<()> {
        Ok(())
    }

    fn parse_keyframe_start(&mut self, e: BytesStart) -> Result<()> {
        let mut kf = KeyframeData {
            time: Tick::ZERO,
            value: Value::Number(0.0),
            interpolation: Interpolation::default(),
        };

        for attr in e.attributes() {
            let attr = attr?;
            let key = std::str::from_utf8(attr.key.as_ref())?;
            let value = std::str::from_utf8(&attr.value)?;

            match key {
                "time" => kf.time = Tick::from_seconds_f64(value.parse()?),
                "value" => kf.value = parse_value(value)?,
                "temporal" => kf.interpolation.temporal = parse_temporal_interp(value),
                "spatial" => kf.interpolation.spatial = parse_spatial_interp(value),
                "easeIn" => kf.interpolation.temporal_ease = Some(effectcraft_keyframe::TemporalEase {
                    influence: value.parse()?,
                    speed: 0.0,
                }),
                "easeOut" => {
                    if let Some(ref mut ease) = kf.interpolation.temporal_ease {
                        ease.speed = value.parse()?;
                    }
                }
                _ => {}
            }
        }

        if let Some(layer) = self.layers.last_mut() {
            if let Some(prop) = layer.properties.last_mut() {
                prop.keyframes.push(kf);
            }
        }
        Ok(())
    }

    fn finish_keyframe(&mut self) -> Result<()> {
        Ok(())
    }

    fn parse_effect_start(&mut self, e: BytesStart) -> Result<()> {
        let mut effect = EffectData {
            match_name: String::new(),
            name: String::new(),
            enabled: true,
            params: Vec::new(),
        };

        for attr in e.attributes() {
            let attr = attr?;
            let key = std::str::from_utf8(attr.key.as_ref())?;
            let value = std::str::from_utf8(&attr.value)?;

            match key {
                "matchName" => effect.match_name = value.to_string(),
                "name" => effect.name = value.to_string(),
                "enabled" => effect.enabled = value == "true",
                _ => {}
            }
        }

        if let Some(layer) = self.layers.last_mut() {
            layer.effects.push(effect);
        }
        Ok(())
    }

    fn finish_effect(&mut self) -> Result<()> {
        Ok(())
    }

    fn parse_effect_param_start(&mut self, e: BytesStart) -> Result<()> {
        let mut param = EffectParam {
            name: String::new(),
            match_name: String::new(),
            value: Value::Number(0.0),
            keyframes: Vec::new(),
            expression: None,
        };

        for attr in e.attributes() {
            let attr = attr?;
            let key = std::str::from_utf8(attr.key.as_ref())?;
            let value = std::str::from_utf8(&attr.value)?;

            match key {
                "name" => param.name = value.to_string(),
                "matchName" => param.match_name = value.to_string(),
                "value" => param.value = parse_value(value)?,
                "expression" => param.expression = Some(value.to_string()),
                _ => {}
            }
        }

        if let Some(layer) = self.layers.last_mut() {
            if let Some(effect) = layer.effects.last_mut() {
                effect.params.push(param);
            }
        }
        Ok(())
    }

    fn finish_effect_param(&mut self) -> Result<()> {
        Ok(())
    }

    fn parse_mask_start(&mut self, e: BytesStart) -> Result<()> {
        let mut mask = MaskData {
            name: String::new(),
            path: effectcraft_path::Path::default(),
            mode: MaskMode::Add,
            feather: 0.0,
            opacity: 100.0,
            expansion: 0.0,
            inverted: false,
        };

        for attr in e.attributes() {
            let attr = attr?;
            let key = std::str::from_utf8(attr.key.as_ref())?;
            let value = std::str::from_utf8(&attr.value)?;

            match key {
                "name" => mask.name = value.to_string(),
                "mode" => mask.mode = parse_mask_mode(value),
                "feather" => mask.feather = value.parse()?,
                "opacity" => mask.opacity = value.parse()?,
                "expansion" => mask.expansion = value.parse()?,
                "inverted" => mask.inverted = value == "true",
                _ => {}
            }
        }

        if let Some(layer) = self.layers.last_mut() {
            layer.masks.push(mask);
        }
        Ok(())
    }

    fn finish_mask(&mut self) -> Result<()> {
        Ok(())
    }

    fn parse_marker_start(&mut self, e: BytesStart) -> Result<()> {
        // Markers are handled similarly
        Ok(())
    }

    fn finish_marker(&mut self) -> Result<()> {
        Ok(())
    }

    fn build_project(&mut self) -> Result<Project> {
        // Add compositions
        for (_, comp) in self.comps.drain() {
            self.project.add_comp(comp);
        }

        // Add footage
        for (_, footage) in self.footage_map.drain() {
            self.project.add_item(&footage.name, Label::None, None, ItemKind::Footage(footage));
        }

        // Add layers to compositions
        for layer_data in self.layers.drain(..) {
            if let Some(comp) = self.project.comp_mut(ItemId::new(layer_data.comp_id.parse().unwrap_or(0))) {
                let mut layer = Layer::new(layer_data.name);
                layer.transform = layer_data.transform;
                layer.enabled = layer_data.enabled;
                layer.solo = layer_data.solo;
                layer.locked = layer_data.locked;
                layer.shy = layer_data.shy;

                // Set source
                if let Some(source_id) = layer_data.source_id {
                    if let Some(footage) = self.project.item(ItemId::new(source_id.parse().unwrap_or(0))) {
                        layer.source = LayerSource::Footage(footage.clone());
                    } else if let Some(comp) = self.project.comp(ItemId::new(source_id.parse().unwrap_or(0))) {
                        layer.source = LayerSource::Comp(comp.clone());
                    }
                }

                // Apply properties
                for prop_data in layer_data.properties {
                    if let Some(prop) = layer.props.prop_mut(&prop_data.path) {
                        prop.set_value(prop_data.value);
                        for kf in prop_data.keyframes {
                            prop.add_keyframe(Keyframe {
                                time: kf.time,
                                value: kf.value,
                                interpolation: kf.interpolation,
                            });
                        }
                        if let Some(expr) = prop_data.expression {
                            prop.expr = Some(effectcraft_project::Expression { text: expr, ..Default::default() });
                        }
                    }
                }

                // Apply effects
                for effect_data in layer_data.effects {
                    let effect_id = map_ae_effect(&effect_data.match_name);
                    if let Some(effect_id) = effect_id {
                        let mut effect = effectcraft_effects::EffectInstance::new(effect_id);
                        effect.enabled = effect_data.enabled;
                        for param in effect_data.params {
                            if let Some(p) = effect.param_mut(&param.name) {
                                p.set_value(param.value);
                                for kf in param.keyframes {
                                    p.add_keyframe(Keyframe {
                                        time: kf.time,
                                        value: kf.value,
                                        interpolation: kf.interpolation,
                                    });
                                }
                                if let Some(expr) = param.expression {
                                    p.expr = Some(effectcraft_project::Expression { text: expr, ..Default::default() });
                                }
                            }
                        }
                        layer.add_effect(effect);
                    }
                }

                comp.add_layer(layer);
            }
        }

        Ok(std::mem::take(&mut self.project))
    }
}

/// Parse a color string (hex or rgb)
fn parse_color(s: &str) -> Result<effectcraft_color::LinearRgba> {
    if s.starts_with('#') {
        let hex = &s[1..];
        if hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16)? as f32 / 255.0;
            let g = u8::from_str_radix(&hex[2..4], 16)? as f32 / 255.0;
            let b = u8::from_str_radix(&hex[4..6], 16)? as f32 / 255.0;
            return Ok(effectcraft_color::LinearRgba::new(r, g, b, 1.0));
        } else if hex.len() == 8 {
            let r = u8::from_str_radix(&hex[0..2], 16)? as f32 / 255.0;
            let g = u8::from_str_radix(&hex[2..4], 16)? as f32 / 255.0;
            let b = u8::from_str_radix(&hex[4..6], 16)? as f32 / 255.0;
            let a = u8::from_str_radix(&hex[6..8], 16)? as f32 / 255.0;
            return Ok(effectcraft_color::LinearRgba::new(r, g, b, a));
        }
    }
    // Try space-separated values
    let parts: Vec<f32> = s.split_whitespace().filter_map(|v| v.parse().ok()).collect();
    if parts.len() >= 3 {
        return Ok(effectcraft_color::LinearRgba::new(
            parts[0], parts[1], parts[2],
            parts.get(3).copied().unwrap_or(1.0),
        ));
    }
    Ok(effectcraft_color::LinearRgba::new(0.0, 0.0, 0.0, 1.0))
}

fn parse_layer_type(s: &str) -> LayerType {
    match s {
        "footage" => LayerType::Footage,
        "solid" => LayerType::Solid,
        "null" => LayerType::Null,
        "adjustment" => LayerType::Adjustment,
        "text" => LayerType::Text,
        "shape" => LayerType::Shape,
        "camera" => LayerType::Camera,
        "light" => LayerType::Light,
        "comp" => LayerType::Comp,
        _ => LayerType::Footage,
    }
}

fn parse_footage_kind(s: &str) -> FootageKind {
    match s {
        "image" => FootageKind::Image,
        "video" => FootageKind::Video,
        "audio" => FootageKind::Audio,
        "sequence" => FootageKind::Sequence,
        _ => FootageKind::Image,
    }
}

fn parse_blend_mode(s: &str) -> BlendMode {
    match s {
        "normal" => BlendMode::Normal,
        "add" => BlendMode::Add,
        "screen" => BlendMode::Screen,
        "multiply" => BlendMode::Multiply,
        "overlay" => BlendMode::Overlay,
        "darken" => BlendMode::Darken,
        "lighten" => BlendMode::Lighten,
        "difference" => BlendMode::Difference,
        "exclusion" => BlendMode::Exclusion,
        _ => BlendMode::Normal,
    }
}

fn parse_mask_mode(s: &str) -> MaskMode {
    match s {
        "none" => MaskMode::None,
        "add" => MaskMode::Add,
        "subtract" => MaskMode::Subtract,
        "intersect" => MaskMode::Intersect,
        "lighten" => MaskMode::Lighten,
        "darken" => MaskMode::Darken,
        "difference" => MaskMode::Difference,
        _ => MaskMode::Add,
    }
}

fn parse_value(s: &str) -> Result<Value> {
    // Try parsing as number
    if let Ok(n) = s.parse::<f64>() {
        return Ok(Value::Number(n));
    }

    // Try parsing as array [x, y] or [x, y, z]
    if s.starts_with('[') && s.ends_with(']') {
        let inner = &s[1..s.len()-1];
        let parts: Vec<f64> = inner.split(',')
            .filter_map(|v| v.trim().parse().ok())
            .collect();
        return Ok(Value::Array(parts));
    }

    // Try parsing as boolean
    if s == "true" {
        return Ok(Value::Bool(true));
    }
    if s == "false" {
        return Ok(Value::Bool(false));
    }

    // Default to string
    Ok(Value::String(s.to_string()))
}

fn parse_temporal_interp(s: &str) -> TemporalInterpolation {
    match s {
        "linear" => TemporalInterpolation::Linear,
        "bezier" => TemporalInterpolation::Bezier,
        "hold" => TemporalInterpolation::Hold,
        "autoBezier" => TemporalInterpolation::AutoBezier,
        "continuousBezier" => TemporalInterpolation::ContinuousBezier,
        _ => TemporalInterpolation::Linear,
    }
}

fn parse_spatial_interp(s: &str) -> SpatialInterpolation {
    match s {
        "linear" => SpatialInterpolation::Linear,
        "bezier" => SpatialInterpolation::Bezier,
        "autoBezier" => SpatialInterpolation::AutoBezier,
        "continuousBezier" => SpatialInterpolation::ContinuousBezier,
        _ => SpatialInterpolation::Linear,
    }
}

fn map_ae_effect(match_name: &str) -> Option<String> {
    let map = [
        ("ADBE Gaussian Blur", "gaussian_blur"),
        ("ADBE Box Blur", "box_blur"),
        ("ADBE Fast Blur", "fast_blur"),
        ("ADBE Levels", "levels"),
        ("ADBE Curves", "curves"),
        ("ADBE Hue/Saturation", "hue_saturation"),
        ("ADBE Color Balance", "color_balance"),
        ("ADBE Brightness & Contrast", "brightness_contrast"),
        ("ADBE Exposure", "exposure"),
        ("ADBE Vibrance", "vibrance"),
        ("ADBE Photo Filter", "photo_filter"),
        ("ADBE Black & White", "black_white"),
        ("ADBE Channel Mixer", "channel_mixer"),
        ("ADBE Gradient Ramp", "gradient_ramp"),
        ("ADBE Fill", "fill"),
        ("ADBE Stroke", "stroke"),
        ("ADBE CC Particle World", "cc_particle_world"),
        ("ADBE CC Snowfall", "cc_snowfall"),
        ("ADBE CC Rainfall", "cc_rainfall"),
        ("ADBE CC Bubbles", "cc_bubbles"),
        ("ADBE CC Drizzle", "cc_drizzle"),
        ("ADBE CC Mr. Mercury", "cc_mr_mercury"),
        ("ADBE CC Ball Action", "cc_ball_action"),
        ("ADBE CC Pixel Polly", "cc_pixel_polly"),
        ("ADBE CC Scatterize", "cc_scatterize"),
        ("ADBE CC Grid", "cc_grid"),
        ("ADBE CC Lens", "cc_lens"),
        ("ADBE CC Light Burst", "cc_light_burst"),
        ("ADBE CC Light Sweep", "cc_light_sweep"),
        ("ADBE CC Radial Fast Blur", "cc_radial_fast_blur"),
        ("ADBE CC Radial Blur", "cc_radial_blur"),
        ("ADBE CC Directional Blur", "cc_directional_blur"),
        ("ADBE CC Cross Blur", "cc_cross_blur"),
        ("ADBE CC Vector Blur", "cc_vector_blur"),
        ("ADBE CC Force Motion Blur", "cc_force_motion_blur"),
        ("ADBE CC Time Blend", "cc_time_blend"),
        ("ADBE CC Wide Time", "cc_wide_time"),
        ("ADBE Echo", "echo"),
        ("ADBE Posterize Time", "posterize_time"),
        ("ADBE Time Displacement", "time_displacement"),
        ("ADBE Time Warp", "time_warp"),
        ("ADBE Transform", "transform"),
        ("ADBE Corner Pin", "corner_pin"),
        ("ADBE CC Power Pin", "cc_power_pin"),
        ("ADBE CC Slant", "cc_slant"),
        ("ADBE CC Scale Wipe", "cc_scale_wipe"),
        ("ADBE CC Page Turn", "cc_page_turn"),
        ("ADBE CC Glass", "cc_glass"),
        ("ADBE CC Light Wipe", "cc_light_wipe"),
        ("ADBE CC Jaws", "cc_jaws"),
        ("ADBE CC Ripple Pulse", "cc_ripple_pulse"),
        ("ADBE CC Twister", "cc_twister"),
        ("ADBE CC Warp", "cc_warp"),
        ("ADBE Bezier Warp", "bezier_warp"),
        ("ADBE Mesh Warp", "mesh_warp"),
        ("ADBE Reshape", "reshape"),
        ("ADBE Displacement Map", "displacement_map"),
        ("ADBE Turbulent Displace", "turbulent_displace"),
        ("ADBE Bulge", "bulge"),
        ("ADBE Spherize", "spherize"),
        ("ADBE Ripple", "ripple"),
        ("ADBE Wave Warp", "wave_warp"),
        ("ADBE CC Twirl", "cc_twirl"),
        ("ADBE CC Kaleidoscope", "cc_kaleidoscope"),
        ("ADBE CC Tile", "cc_tile"),
        ("ADBE Motion Tile", "motion_tile"),
        ("ADBE RepeTile", "repetile"),
        ("ADBE CC RepeTile", "cc_repetile"),
        ("ADBE Mirror", "mirror"),
        ("ADBE CC Split", "cc_split"),
        ("ADBE CC Composite", "cc_composite"),
        ("ADBE CC Kernel", "cc_kernel"),
        ("ADBE CC Vignette", "cc_vignette"),
        ("ADBE Glow", "glow"),
        ("ADBE CC Light Rays", "cc_light_rays"),
        ("ADBE CC Spotlight", "cc_spotlight"),
        ("ADBE CC Light Burst 2.5", "cc_light_burst_25"),
        ("ADBE Lens Flare", "lens_flare"),
        ("ADBE CC Lens Flare", "cc_lens_flare"),
    ];

    map.iter()
        .find(|(ae, _)| *ae == match_name)
        .map(|(_, ec)| ec.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_color() {
        let c = parse_color("#ff0000").unwrap();
        assert_eq!(c.r, 1.0);
        assert_eq!(c.g, 0.0);
        assert_eq!(c.b, 0.0);
    }

    #[test]
    fn test_parse_value() {
        assert_eq!(parse_value("42").unwrap(), Value::Number(42.0));
        assert_eq!(parse_value("[1, 2]").unwrap(), Value::Array(vec![1.0, 2.0]));
        assert_eq!(parse_value("true").unwrap(), Value::Bool(true));
    }
}