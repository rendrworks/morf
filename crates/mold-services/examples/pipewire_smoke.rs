use mold_services::PipeWire;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pipewire = PipeWire::connect()?;
    let nodes = pipewire.nodes();
    assert!(!nodes.is_empty(), "PipeWire returned no nodes");
    println!("PipeWire nodes: {}", nodes.len());
    for node in &nodes {
        println!(
            "{} {} {} ({})",
            node.id, node.media_class, node.description, node.name
        );
    }

    let sink = nodes
        .iter()
        .find(|node| node.media_class == "Audio/Sink")
        .ok_or("PipeWire returned no audio sink")?;
    let before = pipewire.volume(sink.id)?;
    assert!(
        !before.channels.is_empty(),
        "audio sink returned no channel volumes"
    );
    pipewire.set_volume(sink.id, &before.channels, before.muted)?;
    let after = pipewire.volume(sink.id)?;
    assert_eq!(before.muted, after.muted);
    assert_eq!(before.channels.len(), after.channels.len());
    println!(
        "PipeWire sink {}: {:.0}%{}",
        sink.description,
        after.average() * 100.0,
        if after.muted { " muted" } else { "" }
    );
    Ok(())
}
