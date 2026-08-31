use morf_lua::{Limits, Runtime};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut runtime = Runtime::new(Limits::default());
    runtime.execute(
        "pipewire-smoke.lua",
        br#"
            local morf = require("morf")
            local pipewire = morf.pipewire.connect()
            assert(type(pipewire:nodes()) == "table")
        "#,
    )?;
    println!("native PipeWire graph binding passed");
    Ok(())
}
