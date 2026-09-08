//! Counts-only inspection; template expressions and original bytes are not printed.
use nextcore_core::firmware_dt::{FirmwareDeviceTree, MAX_INPUT_BYTES};
use std::{fs::File, io::Read};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let path = arguments
        .next()
        .ok_or("usage: inspect_firmware_dt <input>")?;
    if arguments.next().is_some() {
        return Err("unexpected extra argument".into());
    }
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_INPUT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    let tree = FirmwareDeviceTree::parse(&bytes)?;
    let statistics = tree.statistics();
    let runtime = tree.validated_runtime_bytes();
    let result = serde_json::json!({
        "schema": "nextcore.firmware-dt-inspection.v1", "bytes": bytes.len(),
        "nodes": statistics.nodes, "properties": statistics.properties,
        "templates": statistics.templates, "maximum_depth": statistics.maximum_depth,
        "source_preserved": tree.source() == bytes, "runtime_structure_valid": runtime.is_ok(),
        "runtime_error": runtime.err().map(|error| error.to_string()),
        "template_values_resolved": false, "platform_complete": false,
    });
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
