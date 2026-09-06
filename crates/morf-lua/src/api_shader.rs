use luna::{Callback, CallbackReturn, Context, Table, Value as LuaValue};
use morf_shader::{Binding, ShaderKind, ShaderSpec, Type};
use std::cell::RefCell;
use std::rc::Rc;

use crate::{scene_bindings::*, state::*};

/// A shader a configuration registered, ready for the renderer.
pub(crate) struct RegisteredShader {
    pub(crate) compiled: morf_shader::Compiled,
    /// The vertex displacement, compiled separately: it is a different stage
    /// with a different signature, not a second entry point in one program.
    pub(crate) vertex: Option<morf_shader::Compiled>,
    /// Image paths for the declared textures, in binding order.
    pub(crate) texture_paths: Vec<String>,
    /// What the shader is allowed to decide, which selects its pipeline.
    pub(crate) kind: ShaderKind,
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
    let state_for_api = Rc::clone(&state);
    let shader = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (name, spec): (String, Table) = stack.consume(ctx)?;
        register(&state, ctx, &name, spec).map_err(HostError)?;
        Ok(CallbackReturn::Return)
    });
    morf.set_field(ctx, "shader", shader);

    let state_for_data = Rc::clone(&state_for_api);
    let shader_data = Callback::from_fn(&ctx, move |ctx, _, mut stack| {
        let (node, block, values): (LuaValue, String, Table) = stack.consume(ctx)?;
        let LuaValue::UserData(node) = node else {
            return Err(HostError("shader_data needs a morf node".into()).into());
        };
        let node = node
            .downcast_static::<NodeToken>()
            .map_err(|_| HostError("shader_data needs a morf node".into()))?;
        set_data(&state_for_data, ctx, node.handle, &block, values).map_err(HostError)?;
        Ok(CallbackReturn::Return)
    });
    morf.set_field(ctx, "shader_data", shader_data);
}

/// Fills one data block of the shader attached to a node.
fn set_data<'gc>(
    state: &Rc<RefCell<ReactiveState>>,
    ctx: Context<'gc>,
    node: morf_scene::NodeHandle,
    block: &str,
    values: Table<'gc>,
) -> Result<(), String> {
    // Which block, by the order the shader declared them — the same order the
    // bindings are numbered in, so the name is resolved once here rather than
    // on every frame.
    let index = {
        let borrowed = state.borrow();
        let Some(shader) = borrowed.scene.node_shader(node) else {
            return Err("this node has no shader".into());
        };
        let program = shader.program;
        let Some(registered) = borrowed
            .shaders
            .values()
            .find(|candidate| candidate.compiled.hash == program)
        else {
            return Err("this node's shader is not registered".into());
        };
        registered
            .compiled
            .data
            .iter()
            .position(|(name, _)| name == block)
            .ok_or_else(|| format!("this shader has no data block `{block}`"))?
    };
    let mut numbers = Vec::new();
    for (key, value) in values.iter(ctx) {
        let LuaValue::Integer(slot) = key else {
            return Err("shader data is a list of numbers".into());
        };
        let number = match value {
            LuaValue::Integer(value) => value as f32,
            LuaValue::Number(value) => value as f32,
            _ => return Err("shader data is a list of numbers".into()),
        };
        numbers.push((slot, number));
    }
    numbers.sort_by_key(|(slot, _)| *slot);
    let values: Vec<f32> = numbers.into_iter().map(|(_, value)| value).collect();
    state
        .borrow_mut()
        .scene
        .set_shader_data(node, index, &values);
    Ok(())
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
    let (texture_names, texture_paths) = parse_textures(ctx, name, spec)?;
    let data = parse_data(ctx, name, spec)?;

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
            textures: texture_names,
            data,
            entry: "fragment".to_owned(),
            vertex: false,
        },
    )
    .map_err(|diagnostics| morf_shader::report(name, &diagnostics))?;

    // A vertex displacement is its own compile: a different signature, a
    // different stage, and a shader may have one without the other.
    let vertex = match spec.get_value(ctx, "vertex") {
        LuaValue::Nil => None,
        LuaValue::String(source) => Some(
            morf_shader::compile(
                &source.display_lossy().to_string(),
                &ShaderSpec {
                    kind,
                    inputs: ShaderSpec::vertex_inputs(),
                    params: Vec::new(),
                    textures: Vec::new(),
                    data: Vec::new(),
                    entry: "vertex".to_owned(),
                    vertex: true,
                },
            )
            .map_err(|diagnostics| morf_shader::report(name, &diagnostics))?,
        ),
        _ => return Err(format!("shader {name}: `vertex` must be a string")),
    };

    state.borrow_mut().shaders.insert(
        name.to_owned(),
        RegisteredShader {
            compiled,
            vertex,
            texture_paths,
            kind,
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

/// Reads the declared textures: a name and the image each is bound to.
fn parse_textures<'gc>(
    ctx: Context<'gc>,
    name: &str,
    spec: Table<'gc>,
) -> Result<(Vec<String>, Vec<String>), String> {
    let mut declared = Vec::new();
    match spec.get_value(ctx, "textures") {
        LuaValue::Nil => {}
        LuaValue::Table(textures) => {
            for (key, value) in textures.iter(ctx) {
                let (LuaValue::String(key), LuaValue::String(path)) = (key, value) else {
                    return Err(format!(
                        "shader {name}: a texture is a name and an image path"
                    ));
                };
                declared.push((
                    key.display_lossy().to_string(),
                    path.display_lossy().to_string(),
                ));
            }
        }
        _ => return Err(format!("shader {name}: textures must be a table")),
    }
    // Sorted for the same reason parameters are: a Lua table has no order, and
    // the binding numbers have to be the same every time the file loads.
    declared.sort_by(|left, right| left.0.cmp(&right.0));
    Ok((
        declared.iter().map(|(name, _)| name.clone()).collect(),
        declared.iter().map(|(_, path)| path.clone()).collect(),
    ))
}

/// Reads the declared data blocks: a name and how many numbers it holds.
fn parse_data<'gc>(
    ctx: Context<'gc>,
    name: &str,
    spec: Table<'gc>,
) -> Result<Vec<(String, Type, u32)>, String> {
    let mut declared = Vec::new();
    match spec.get_value(ctx, "data") {
        LuaValue::Nil => {}
        LuaValue::Table(blocks) => {
            for (key, value) in blocks.iter(ctx) {
                let LuaValue::String(key) = key else {
                    return Err(format!("shader {name}: a data block needs a name"));
                };
                let length = match value {
                    LuaValue::Integer(value) if value > 0 => value as u32,
                    _ => {
                        return Err(format!(
                            "shader {name}: data block `{}` needs a positive length",
                            key.display_lossy()
                        ));
                    }
                };
                declared.push((key.display_lossy().to_string(), Type::F32, length));
            }
        }
        _ => return Err(format!("shader {name}: data must be a table")),
    }
    declared.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(declared)
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
    // A data block starts as zeros: the configuration fills it through
    // `morf.shader_data`, and a shader reading one before it is written should
    // see nothing rather than whatever was in the buffer.
    let data_values: Vec<Vec<f32>> = shader
        .compiled
        .data
        .iter()
        .map(|(_, length)| vec![0.0; *length as usize])
        .collect();
    let owns_coverage = shader.kind == ShaderKind::Surface;
    let samples_behind = shader.kind == ShaderKind::Effect;
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
    state.borrow_mut().scene.attach_shader(
        node,
        morf_scene::NodeShader {
            program,
            params,
            data: data_values,
            samples_behind,
            owns_coverage,
        },
    );
    Ok(())
}
