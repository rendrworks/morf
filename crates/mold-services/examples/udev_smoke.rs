use std::time::Duration;

use mold_services::UdevMonitor;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let monitor = UdevMonitor::new(None)?;
    let _ = monitor.next_event(Duration::ZERO)?;
    println!("udev monitor ready");
    Ok(())
}
