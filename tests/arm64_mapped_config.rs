use nextcore_core::boot_config::*;

fn input(extra: &str, budget: u64) -> Vec<u8> {
    format!(r#"<plist version="1.0"><dict><key>Nextcore</key><dict><key>Kernel</key><dict>
    <key>Profile</key><string>x86-efi-arm64-trace</string><key>Path</key><string>/kernel.kc</string>
    <key>Trace</key><dict><key>HandoffAbi</key><string>unprovisioned-sptm-prefix</string>
    <key>PhysicalBase</key><integer>0x40000000</integer><key>VirtualBase</key><integer>0xfffffe0000000000</integer>
    <key>MemorySize</key><integer>67108864</integer><key>ActualMemorySize</key><integer>67108864</integer>
    <key>KernelPhysical</key><integer>0x42000000</integer><key>DeviceTreePath</key><string>/diagnostic.dt</string>
    <key>InstructionBudget</key><integer>{budget}</integer>{extra}</dict></dict></dict></dict></plist>"#).into_bytes()
}
const PLATFORM: &str = "<key>PlatformProfile</key><string>nextcore-irq-compat-v1</string>";
const MEMORY: &str = "<key>MemoryProfile</key><string>mapped-normal-nc-v1</string>";

#[test]
fn all_previous_parsers_reject_any_memory_profile_field() {
    for value in [
        "<string>mapped-normal-nc-v1</string>",
        "<string/>",
        "<false/>",
        "<dict/>",
        "<integer>3</integer>",
    ] {
        let bytes = input(&format!("{PLATFORM}<key>MemoryProfile</key>{value}"), 64);
        assert!(parse_arm64_trace_configuration(&bytes).is_err());
        for limit in [64, 256, 1024, 4096] {
            assert!(parse_arm64_trace_configuration_with_limit(&bytes, limit).is_err());
        }
        assert!(parse_arm64_trace_configuration_with_deep_tier(&bytes).is_err());
        assert!(parse_arm64_trace_configuration_with_long_tier(&bytes).is_err());
        assert!(parse_arm64_trace_configuration_with_initialization_tier(&bytes).is_err());
    }
}

#[test]
fn exact_selection_requires_platform_and_retains_tier_contracts() {
    assert!(parse_arm64_trace_configuration_with_mapped_tier(&input(MEMORY, 8)).is_err());
    for (tier, budget) in [
        ("deep-16384", 16384),
        ("long-65536", 65536),
        ("initialization-67108864", 67108864),
    ] {
        let extra = format!("{PLATFORM}{MEMORY}<key>DiagnosticTier</key><string>{tier}</string>");
        let parsed =
            parse_arm64_trace_configuration_with_mapped_tier(&input(&extra, budget)).unwrap();
        assert_eq!(
            parsed.memory_profile,
            Some(Arm64TraceMemoryProfile::MappedNormalNcV1)
        );
        assert_eq!(parsed.instruction_budget, budget);
        assert!(
            parse_arm64_trace_configuration_with_mapped_tier(&input(&extra, budget - 1)).is_err()
        );
    }
    assert!(parse_arm64_trace_configuration_with_mapped_tier(&input(
        &format!("{PLATFORM}{MEMORY}"),
        65536
    ))
    .is_err());
    for value in [
        "<string>mapped-normal-nc-V1</string>",
        "<string/>",
        "<false/>",
        "<integer>3</integer>",
    ] {
        assert!(parse_arm64_trace_configuration_with_mapped_tier(&input(
            &format!("{PLATFORM}<key>MemoryProfile</key>{value}"),
            64
        ))
        .is_err());
    }
    assert!(parse_arm64_trace_configuration_with_mapped_tier(&input(
        &format!("{PLATFORM}{MEMORY}{MEMORY}"),
        64
    ))
    .is_err());
}

#[test]
fn omission_preserves_initialization_policy() {
    for platform in ["", PLATFORM] {
        for budget in [8, 9, 64, 256, 4096, 16384, 65536, 67108864] {
            let bytes = input(platform, budget);
            assert_eq!(
                parse_arm64_trace_configuration_with_mapped_tier(&bytes),
                parse_arm64_trace_configuration_with_initialization_tier(&bytes)
            );
        }
    }
}
