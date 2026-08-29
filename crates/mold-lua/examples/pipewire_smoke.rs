use mold_lua::{Limits, Runtime};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut runtime = Runtime::new(Limits::default());
    runtime.execute(
        "pipewire-smoke.lua",
        br#"
            local mold = require("mold")
            local pipewire = mold.pipewire.connect()
            assert(type(pipewire:nodes()) == "table")
        "#,
    )?;
    println!("native PipeWire graph binding passed");
    Ok(())
}
