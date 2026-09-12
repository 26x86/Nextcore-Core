use nextcore_core::boot_config::{
    parse_arm64_trace_configuration as parse,
    parse_arm64_trace_configuration_with_deep_tier as parse_deep,
    parse_arm64_trace_configuration_with_limit as parse_limit, Arm64TraceVideo,
};

fn config(extra: &str, budget: u64) -> String {
    format!(
        r#"<plist version="1.0"><dict><key>Nextcore</key><dict><key>Kernel</key><dict>
    <key>Profile</key><string>x86-efi-arm64-trace</string><key>Path</key><string>/kernel.kc</string>
    <key>Trace</key><dict><key>HandoffAbi</key><string>unprovisioned-sptm-prefix</string>
    <key>PhysicalBase</key><integer>0x40000000</integer><key>VirtualBase</key><integer>0xfffffe0000000000</integer>
    <key>MemorySize</key><integer>67108864</integer><key>ActualMemorySize</key><integer>67108864</integer>
    <key>KernelPhysical</key><integer>0x42000000</integer><key>DeviceTreePath</key><string>/diagnostic.dt</string>
    <key>InstructionBudget</key><integer>{budget}</integer>{extra}
    </dict></dict></dict></dict></plist>"#
    )
}

const VIDEO: &str = "<key>Video</key><string>gop-framebuffer</string>";
const PROFILE: &str = "<key>PlatformProfile</key><string>nextcore-irq-compat-v1</string>";

#[test]
fn omission_preserves_configuration_except_explicit_video_choice() {
    let plain = parse(config("", 8).as_bytes()).unwrap();
    assert_eq!(plain.video, None);
    let mut selected = parse(config(VIDEO, 8).as_bytes()).unwrap();
    assert_eq!(selected.video, Some(Arm64TraceVideo::GopFramebuffer));
    selected.video = None;
    assert_eq!(selected, plain);
    assert_eq!(
        parse_limit(config(VIDEO, 8).as_bytes(), 4096)
            .unwrap()
            .video,
        Some(Arm64TraceVideo::GopFramebuffer)
    );
    assert_eq!(
        parse_deep(config(VIDEO, 8).as_bytes()).unwrap().video,
        Some(Arm64TraceVideo::GopFramebuffer)
    );
}

#[test]
fn unknown_types_values_and_duplicates_reject() {
    for value in [
        "<true/>",
        "<false/>",
        "<integer>1</integer>",
        "<dict/>",
        "<array/>",
        "<string/>",
        "<string>auto</string>",
        "<string>GOP-framebuffer</string>",
    ] {
        let xml = config(&format!("<key>Video</key>{value}"), 8);
        assert!(parse(xml.as_bytes()).is_err(), "{value}");
        assert!(parse_deep(xml.as_bytes()).is_err(), "{value}");
    }
    assert!(parse(config(&format!("{VIDEO}{VIDEO}"), 8).as_bytes()).is_err());
}

#[test]
fn video_does_not_relax_existing_execution_or_memory_contracts() {
    for budget in [9, 64, 256, 4096, 16384] {
        assert!(parse(config(VIDEO, budget).as_bytes()).is_err());
    }
    assert!(parse(config(&format!("{VIDEO}{PROFILE}"), 256).as_bytes()).is_err());
    assert!(parse_limit(config(&format!("{VIDEO}{PROFILE}"), 16384).as_bytes(), 4096).is_err());
    let deep = format!("{VIDEO}{PROFILE}<key>DiagnosticTier</key><string>deep-16384</string>");
    assert_eq!(
        parse_deep(config(&deep, 16384).as_bytes()).unwrap().video,
        Some(Arm64TraceVideo::GopFramebuffer)
    );
    for xml in [
        config(VIDEO, 8).replace("0x40000000", "0x40000001"),
        config(VIDEO, 8).replace("unprovisioned-sptm-prefix", "complete"),
    ] {
        assert!(parse_deep(xml.as_bytes()).is_err());
    }
}
