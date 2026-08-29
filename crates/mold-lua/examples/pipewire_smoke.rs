use mold_lua::{Limits, Runtime};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut runtime = Runtime::new(Limits::default());
    runtime.execute(
        "pipewire-smoke.lua",
        br#"
            local volume = require("patin.services.volume").new()
            local level = volume.level()
            local muted = volume.muted()
            volume:set_level(level)
            volume:set_muted(muted)
            assert(type(volume.node.description) == "string")
            assert(type(level) == "number")
            assert(type(muted) == "boolean")
        "#,
    )?;
    println!("pure-Lua volume service round-trip passed");
    Ok(())
}
