mod caps;
mod diagnostics;
mod golden;

use crate::*;

/// Compiles a shader body with the default material inputs.
pub(crate) fn compile_material(body: &str) -> Result<Compiled, Vec<Diagnostic>> {
    compile_with(body, ShaderKind::Material, Vec::new())
}

pub(crate) fn compile_with(
    body: &str,
    kind: ShaderKind,
    params: Vec<Binding>,
) -> Result<Compiled, Vec<Diagnostic>> {
    let spec = ShaderSpec {
        kind,
        inputs: ShaderSpec::default_inputs(kind),
        params,
        entry: "fragment".to_owned(),
    };
    compile(body, &spec)
}

/// The WGSL for a body that is expected to compile.
pub(crate) fn wgsl(body: &str) -> String {
    match compile_material(body) {
        Ok(compiled) => compiled.wgsl,
        Err(diagnostics) => panic!(
            "expected this to compile:\n{}",
            report("test", &diagnostics)
        ),
    }
}

/// The diagnostics for a body that is expected not to.
pub(crate) fn errors(body: &str) -> Vec<Diagnostic> {
    match compile_material(body) {
        Ok(_) => panic!("expected this to fail, but it compiled"),
        Err(diagnostics) => diagnostics,
    }
}

/// Whether any diagnostic mentions the phrase.
pub(crate) fn mentions(diagnostics: &[Diagnostic], phrase: &str) -> bool {
    diagnostics.iter().any(|diagnostic| {
        diagnostic.message.contains(phrase)
            || diagnostic
                .note
                .as_deref()
                .is_some_and(|note| note.contains(phrase))
    })
}
