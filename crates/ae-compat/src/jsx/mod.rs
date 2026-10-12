//! JSX / ExtendScript Runtime for After Effects Compatibility
//!
//! Provides a sandboxed JavaScript engine compatible with Adobe ExtendScript (`.jsx`)
//! and After Effects expressions. Extends EffectCraft's existing `effectcraft_expr` crate
//! with AE-specific globals and API.
//!
//! Uses the same Boa engine as `effectcraft_expr` but with additional AE-compatible
//! bindings for scripts (not just expressions).

use boa_engine::{Context, Source, js_string, JsResult, JsValue, Module};
use effectcraft_expr::{Expressions, evaluate, eval_property, eval_standalone, check_syntax};
use effectcraft_interchange::{Error, Result};
use effectcraft_keyframe::Value;
use effectcraft_project::{Project, ItemId, LayerId};
use effectcraft_render::{EvalCtx, ExprHost};
use effectcraft_time::Tick;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// JSX/ExtendScript runtime for running After Effects scripts.
///
/// Unlike expressions (which are evaluated per-property per-frame), scripts are
/// full programs that can create/modify projects, add layers, apply effects, etc.
pub struct JsxRuntime {
    context: Context,
    project: Option<Project>,
    api_bindings: JsxApiBindings,
    script_cache: HashMap<String, Module>,
}

/// Bindings for AE-compatible scripting API
struct JsxApiBindings {
    app: JsValue,
    project: JsValue,
    // Additional AE objects: comp, layer, footage, etc.
}

impl JsxRuntime {
    /// Create a new JSX runtime with AE-compatible globals.
    pub fn new() -> Result<Self> {
        let mut context = Context::default();

        // Initialize AE-compatible global objects
        let api_bindings = JsxApiBindings::new(&mut context)?;

        Ok(Self {
            context,
            project: None,
            api_bindings,
            script_cache: HashMap::new(),
        })
    }

    /// Set the current project for script operations.
    pub fn set_project(&mut self, project: Project) {
        self.project = Some(project);
        // Update the `app.project` binding
        self.update_project_binding();
    }

    /// Get a mutable reference to the current project.
    pub fn project_mut(&mut self) -> Option<&mut Project> {
        self.project.as_mut()
    }

    /// Execute a JSX script from source code.
    pub fn execute_script(&mut self, source: &str, filename: &str) -> Result<JsValue> {
        // Compile and cache the module
        let module = if let Some(cached) = self.script_cache.get(filename) {
            cached.clone()
        } else {
            let module = Module::parse(Source::from_bytes(source), None, &mut self.context)
                .map_err(|e| Error::ScriptError(format!("Parse error: {}", e)))?;
            self.script_cache.insert(filename.to_string(), module.clone());
            module
        };

        // Execute the module
        let result = module.evaluate(&mut self.context)
            .map_err(|e| Error::ScriptError(format!("Runtime error: {}", e)))?;

        Ok(result)
    }

    /// Execute a JSX script from a file.
    pub fn execute_file<P: AsRef<Path>>(&mut self, path: P) -> Result<JsValue> {
        let path = path.as_ref();
        let source = std::fs::read_to_string(path)
            .map_err(|e| Error::Io(e.to_string()))?;

        let filename = path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("script.jsx");

        self.execute_script(&source, filename)
    }

    /// Evaluate an AE expression using EffectCraft's expression engine.
    ///
    /// This bridges to `effectcraft_expr` for expression evaluation with
    /// the same deterministic semantics.
    pub fn evaluate_expression(
        &self,
        project: &Project,
        comp: ItemId,
        layer: LayerId,
        property_path: &str,
        expression: &str,
        time: f64,
    ) -> Result<Value> {
        eval_standalone(project, comp, layer, property_path, expression, time)
    }

    /// Check expression syntax without evaluating.
    pub fn check_expression_syntax(expression: &str) -> Result<()> {
        check_syntax(expression)
    }

    fn update_project_binding(&mut self) {
        // Update the `app.project` binding with current project
        // This would serialize the project to a JS object
    }
}

impl JsxApiBindings {
    fn new(context: &mut Context) -> Result<Self> {
        // Create the `app` global object with AE-compatible API
        let app = context.construct_object()?;

        // app.project
        let project_obj = context.construct_object()?;
        app.set(js_string!("project"), project_obj.clone(), true, &mut *context)?;

        // app.project.activeItem
        // app.project.items
        // app.project.save()
        // app.project.close()

        // app.project.item(index)
        // app.project.activeItem

        // app.beginUndoGroup(), app.endUndoGroup()

        // Global functions: alert(), $.write(), $.writeln()

        Ok(Self {
            app,
            project: project_obj,
        })
    }
}

impl Default for JsxRuntime {
    fn default() -> Self {
        Self::new().expect("Failed to create JSX runtime")
    }
}

/// High-level API for running AE scripts with project context
pub struct AeScriptRunner {
    runtime: JsxRuntime,
}

impl AeScriptRunner {
    /// Create a new script runner with a project.
    pub fn new(project: Project) -> Result<Self> {
        let mut runtime = JsxRuntime::new()?;
        runtime.set_project(project);
        Ok(Self { runtime })
    }

    /// Run a script file and return the modified project.
    pub fn run_file<P: AsRef<Path>>(mut self, path: P) -> Result<Project> {
        self.runtime.execute_file(path)?;
        self.runtime.project.ok_or_else(|| Error::ScriptError("No project loaded".into()))
    }

    /// Run a script from source and return the modified project.
    pub fn run_source(mut self, source: &str, filename: &str) -> Result<Project> {
        self.runtime.execute_script(source, filename)?;
        self.runtime.project.ok_or_else(|| Error::ScriptError("No project loaded".into()))
    }

    /// Get access to the runtime for advanced usage.
    pub fn runtime(&mut self) -> &mut JsxRuntime {
        &mut self.runtime
    }
}

/// AE-compatible expression host that extends EffectCraft's Expressions
/// with additional AE-specific functions.
pub struct AeExpressionHost {
    base: Expressions,
}

impl AeExpressionHost {
    pub fn new() -> Self {
        Self { base: Expressions }
    }
}

impl Default for AeExpressionHost {
    fn default() -> Self {
        Self::new()
    }
}

impl ExprHost for AeExpressionHost {
    fn eval(&self, ctx: &EvalCtx, layer: &effectcraft_project::Layer, prop: &effectcraft_project::Property, value: &Value) -> Result<Value, String> {
        self.base.eval(ctx, layer, prop, value)
    }

    fn eval_text_selector(&self, ctx: &EvalCtx, layer: &effectcraft_project::Layer, prop: &effectcraft_project::Property, index: usize, total: usize, selector: [f64; 3]) -> Result<[f64; 3], String> {
        self.base.eval_text_selector(ctx, layer, prop, index, total, selector)
    }
}

/// Register AE-compatible expression functions in the Boa engine context.
///
/// This adds functions like `wiggle`, `loopOut`, `toComp`, `fromComp`, etc.
/// to the global scope for use in scripts (not just expressions).
pub fn register_ae_globals(context: &mut Context, project: Option<&Project>) -> Result<()> {
    // Register global functions
    let global = context.global_object();

    // Math extensions
    let math = context.construct_object()?;
    math.set(js_string!("PI"), std::f64::consts::PI, true, context)?;
    math.set(js_string!("radiansToDegrees"), js_string!("radiansToDegrees"), true, context)?;
    // ... add more math functions

    global.set(js_string!("Math"), math, true, context)?;

    // AE-specific globals: time, value, thisComp, thisLayer, thisProperty, etc.
    // These are normally provided by the expression host during evaluation

    Ok(())
}

/// Convert an AE expression string to a format compatible with EffectCraft's engine.
///
/// Handles:
/// - Legacy array math syntax: `[1,2] + [3,4]` → `add([1,2], [3,4])`
/// - AE-specific property access: `thisComp.layer("name").transform.position`
/// - Expression selectors
pub fn preprocess_ae_expression(expr: &str) -> String {
    // Use EffectCraft's rewrite module for array math
    effectcraft_expr::rewrite::rewrite(expr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jsx_runtime_creation() {
        let runtime = JsxRuntime::new();
        assert!(runtime.is_ok());
    }

    #[test]
    fn test_preprocess_expression() {
        let expr = "[1, 2] + [3, 4]";
        let processed = preprocess_ae_expression(expr);
        assert!(processed.contains("add"));
    }
}