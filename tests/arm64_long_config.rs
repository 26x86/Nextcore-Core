use nextcore_core::boot_config::{
    parse_arm64_trace_configuration as ordinary,
    parse_arm64_trace_configuration_with_deep_tier as deep,
    parse_arm64_trace_configuration_with_limit as limited,
    parse_arm64_trace_configuration_with_long_tier as long, BootConfigError as E,
};

fn config(budget: u64, tier: Option<&str>, profile: bool) -> Vec<u8> {
    let selection = tier
        .map(|body| format!("<key>DiagnosticTier</key>{body}"))
        .unwrap_or_default();
    let platform = if profile {
        "<key>PlatformProfile</key><string>nextcore-irq-compat-v1</string>"
    } else {
        ""
    };
    format!(r#"<plist version="1.0"><dict><key>Nextcore</key><dict><key>Kernel</key><dict>
<key>Profile</key><string>x86-efi-arm64-trace</string><key>Path</key><string>/fixture.kc</string>
<key>Trace</key><dict>{selection}{platform}
<key>HandoffAbi</key><string>unprovisioned-sptm-prefix</string>
<key>PhysicalBase</key><integer>0x40000000</integer><key>VirtualBase</key><integer>0xfffffe0000000000</integer>
<key>MemorySize</key><integer>67108864</integer><key>ActualMemorySize</key><integer>67108864</integer>
<key>KernelPhysical</key><integer>0x42000000</integer><key>DeviceTreePath</key><string>/fixture.dt</string>
<key>InstructionBudget</key><integer>{budget}</integer></dict></dict></dict></dict></plist>"#).into_bytes()
}
const LONG: &str = "<string>long-65536</string>";
const DEEP: &str = "<string>deep-16384</string>";

#[test]
fn long_capability_never_changes_unselected_results() {
    for profile in [false, true] {
        for budget in [
            0,
            1,
            8,
            9,
            64,
            65,
            256,
            512,
            1024,
            4096,
            4097,
            16384,
            65535,
            65536,
            65537,
            u64::MAX,
        ] {
            let input = config(budget, None, profile);
            assert_eq!(long(&input), limited(&input, 4096));
            assert_eq!(long(&input), deep(&input));
        }
    }
}

#[test]
fn selected_tiers_require_exact_budget_and_profile() {
    for (selector, expected) in [(LONG, 65536), (DEEP, 16384)] {
        for profile in [false, true] {
            for budget in [
                0,
                8,
                64,
                256,
                1024,
                4096,
                16383,
                16384,
                16385,
                65535,
                65536,
                65537,
                u64::MAX,
            ] {
                let result = long(&config(budget, Some(selector), profile));
                if profile && budget == expected {
                    let parsed = result.unwrap();
                    assert_eq!(parsed.instruction_budget, expected);
                    let mut lower = limited(&config(4096, None, true), 4096).unwrap();
                    lower.instruction_budget = expected;
                    assert_eq!(parsed, lower);
                } else {
                    assert_eq!(result, Err(E::InvalidTraceConfiguration));
                }
            }
        }
    }
}

#[test]
fn existing_parsers_reject_long_even_with_small_budget() {
    for budget in [1, 8, 64, 256, 1024, 4096, 16384, 65536] {
        let input = config(budget, Some(LONG), true);
        assert_eq!(ordinary(&input), Err(E::InvalidTraceConfiguration));
        assert_eq!(deep(&input), Err(E::InvalidTraceConfiguration));
        for limit in [64, 256, 1024, 4096, 16384, 65536] {
            assert_eq!(limited(&input, limit), Err(E::InvalidTraceConfiguration));
        }
    }
    let input = config(16384, Some(DEEP), true);
    assert_eq!(long(&input), deep(&input));
    assert_eq!(ordinary(&input), Err(E::InvalidTraceConfiguration));
}

#[test]
fn unknown_types_duplicates_and_budget_values_are_rejected() {
    for value in [
        "",
        "65536",
        "long-65535",
        "long-65537",
        "LONG-65536",
        "automatic",
    ] {
        assert_eq!(
            long(&config(
                65536,
                Some(&format!("<string>{value}</string>")),
                true
            )),
            Err(E::InvalidTraceConfiguration)
        );
    }
    for body in ["<integer>65536</integer>", "<true/>", "<dict/>", "<array/>"] {
        assert_eq!(long(&config(65536, Some(body), true)), Err(E::InvalidType));
    }
    for body in [
        format!("{LONG}<key>DiagnosticTier</key>{LONG}"),
        format!("{LONG}<key>DiagnosticTier</key>{DEEP}"),
    ] {
        assert_eq!(
            long(&config(65536, Some(&body), true)),
            Err(E::DuplicateKey)
        );
    }
    let base = String::from_utf8(config(65536, Some(LONG), true)).unwrap();
    for value in [
        "<string>65536</string>",
        "<true/>",
        "<integer>-1</integer>",
        "<integer>18446744073709551616</integer>",
    ] {
        assert!(long(base.replace("<integer>65536</integer>", value).as_bytes()).is_err());
    }
    let duplicate = base.replace(
        "<key>InstructionBudget</key>",
        "<key>InstructionBudget</key><integer>65536</integer><key>InstructionBudget</key>",
    );
    assert_eq!(long(duplicate.as_bytes()), Err(E::DuplicateKey));
}

#[test]
fn long_selection_preserves_handoff_platform_and_video_validation() {
    let base = String::from_utf8(config(65536, Some(LONG), true)).unwrap();
    for bad in [
        base.replace("unprovisioned-sptm-prefix", "automatic"),
        base.replace("0x40000000", "0x40000001"),
    ] {
        assert_eq!(long(bad.as_bytes()), Err(E::InvalidTraceConfiguration));
    }
    assert!(long(
        base.replace("nextcore-irq-compat-v1", "automatic")
            .as_bytes()
    )
    .is_err());
    let bad = base.replace("<key>HandoffAbi</key>", "<key>Platform</key><dict><key>IrqLevel</key><integer>2</integer></dict><key>HandoffAbi</key>");
    assert_eq!(long(bad.as_bytes()), Err(E::InvalidPlatformConfiguration));
    let video = base.replace(
        "<key>HandoffAbi</key>",
        "<key>Video</key><string>gop-framebuffer</string><key>HandoffAbi</key>",
    );
    assert!(long(video.as_bytes()).unwrap().video.is_some());
    assert!(long(video.replace("gop-framebuffer", "automatic").as_bytes()).is_err());
}
