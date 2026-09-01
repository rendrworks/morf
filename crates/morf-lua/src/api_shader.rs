use luna::{Callback, CallbackReturn, Context, Table, Value as LuaValue};
use morf_shader::{Binding, ShaderKind, ShaderSpec, Type};
use std::cell::RefCell;
use std::rc::Rc;

use crate::{scene_bindings::*, state::*};

/// A shader a configuration registered, ready for the renderer.
pub(crate) struct RegisteredShader {
    pub(crate) compiled: morf_shader::Compiled,
    /// Parameter names in the order the uniform block holds them.
    pub(crate) params: Vec<String>,
    /// The values a node gets when it does not override them.
    pub(crate) defaults: Vec<f32>,
}

/// Installs `morf.shader`.
///
/// Compilation happens here, while the configuration is loading and its author
/// is watching, rather than lazily at first paint: a shader error must be
/// something they see in the terminal, not something that appears on the frame
/// a node first becomes visible.
pub(crate) fn install_shader_api<'gc>(
    ctx: Context<'gc>,
    morf: Table<'gc>,
    state: Rc<RefCell<ReactiveState>>,
) {
    let shader = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (name, spec): (String, Table) = stack.consume(ctx)?;
        register(&state, ctx, &name, spec).map_err(HostError)?;
        Ok(CallbackReturn::Return)
    });
    morf.set_field(ctx, "shader", shader);
}

fn register<'gc>(
    state: &Rc<RefCell<ReactiveState>>,
    ctx: Context<'gc>,
    name: &str,
    spec: Table<'gc>,
) -> Result<(), String> {
    if name.is_empty() {
        return Err("a shader needs a name".into());
    }
    let kind = match spec.get_value(ctx, "kind") {
        LuaValue::Nil => ShaderKind::Material,
        LuaValue::String(value) => ShaderKind::parse(&value.display_lossy().to_string())
            .ok_or_else(|| format!("shader {name}: kind must be material, surface or effect"))?,
        _ => return Err(format!("shader {name}: kind must be a string")),
    };
    let LuaValue::String(source) = spec.get_value(ctx, "fragment") else {
        return Err(format!("shader {name}: `fragment` must be a string"));
    };
    let source = source.display_lossy().to_string();
    let (params, defaults) = parse_params(ctx, name, spec)?;

    let compiled = morf_shader::compile(
        &source,
        &ShaderSpec {
            kind,
            inputs: ShaderSpec::default_inputs(kind),
            params: params
                .iter()
                .map(|param| Binding {
                    name: param.clone(),
                    ty: Type::F32,
                })
                .collect(),
            entry: "fragment".to_owned(),
        },
    )
    .map_err(|diagnostics| morf_shader::report(name, &diagnostics))?;

    state.borrow_mut().shaders.insert(
        name.to_owned(),
        RegisteredShader {
            compiled,
            params,
            defaults,
        },
    );
    Ok(())
}

/// Reads the declared parameters, in a fixed order.
///
/// Sorted by name, because a Lua table has none of its own and the uniform
/// layout has to be the same every time the same configuration loads —
/// otherwise a shader's parameters would land at different offsets between two
/// runs of the same file.
fn parse_params<'gc>(
    ctx: Context<'gc>,
    name: &str,
    spec: Table<'gc>,
) -> Result<(Vec<String>, Vec<f32>), String> {
    let mut declared = Vec::new();
    match spec.get_value(ctx, "params") {
        LuaValue::Nil => {}
        LuaValue::Table(params) => {
            for (key, value) in params.iter(ctx) {
                let LuaValue::String(key) = key else {
                    return Err(format!("shader {name}: parameter names must be strings"));
                };
                let default = match value {
                    LuaValue::Integer(value) => value as f32,
                    LuaValue::Number(value) => value as f32,
                    _ => {
                        return Err(format!(
                            "shader {name}: parameter `{}` must be a number",
                            key.display_lossy()
                        ));
                    }
                };
                declared.push((key.display_lossy().to_string(), default));
            }
        }
        _ => return Err(format!("shader {name}: params must be a table")),
    }
    declared.sort_by(|left, right| left.0.cmp(&right.0));
    Ok((
        declared.iter().map(|(name, _)| name.clone()).collect(),
        declared.iter().map(|(_, value)| *value).collect(),
    ))
}

/// Resolves a node's `shader` and `shader_params` into a scene attachment.
///
/// Called while an element is being configured, so the name is looked up once
/// rather than on every paint, and a name that does not resolve is reported
/// where the author wrote it.
pub(crate) fn attach_shader<'gc>(
    state: &Rc<RefCell<ReactiveState>>,
    ctx: Context<'gc>,
    node: morf_scene::NodeHandle,
    name: &str,
    overrides: Option<Table<'gc>>,
) -> Result<(), String> {
    if name.is_empty() {
        state.borrow_mut().scene.detach_shader(node);
        return Ok(());
    }
    let borrowed = state.borrow();
    let Some(shader) = borrowed.shaders.get(name) else {
        return Err(format!(
            "shader `{name}` was never registered; call morf.shader(\"{name}\", …) first"
        ));
    };
    let program = shader.compiled.hash;
    let mut params = shader.defaults.clone();
    let order = shader.params.clone();
    drop(borrowed);

    if let Some(overrides) = overrides {
        for (key, value) in overrides.iter(ctx) {
            let LuaValue::String(key) = key else {
                return Err("shader parameter names must be strings".into());
            };
            let key = key.display_lossy().to_string();
            let Some(index) = order.iter().position(|param| *param == key) else {
                return Err(format!("shader `{name}` has no parameter `{key}`"));
            };
            params[index] = match value {
                LuaValue::Integer(value) => value as f32,
                LuaValue::Number(value) => value as f32,
                _ => return Err(format!("shader parameter `{key}` must be a number")),
            };
        }
    }
    state
        .borrow_mut()
        .scene
        .attach_shader(node, morf_scene::NodeShader { program, params });
    Ok(())
}
