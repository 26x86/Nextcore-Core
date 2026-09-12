use nextcore_core::boot_config::{
    parse_arm64_trace_configuration as ordinary,
    parse_arm64_trace_configuration_with_deep_tier as deep,
    parse_arm64_trace_configuration_with_initialization_tier as initialization,
    parse_arm64_trace_configuration_with_limit as limited,
    parse_arm64_trace_configuration_with_long_tier as long, BootConfigError as E,
};
const INIT: &str = "<string>initialization-67108864</string>";

fn fixture(budget: u64, selector: Option<&str>, profile: bool) -> Vec<u8> {
    let selector = selector
        .map(|s| format!("<key>DiagnosticTier</key>{s}"))
        .unwrap_or_default();
    let profile = if profile {
        "<key>PlatformProfile</key><string>nextcore-irq-compat-v1</string>"
    } else {
        ""
    };
    format!(r#"<plist version="1.0"><dict><key>Nextcore</key><dict><key>Kernel</key><dict>
<key>Profile</key><string>x86-efi-arm64-trace</string><key>Path</key><string>/authored.kc</string>
<key>Trace</key><dict>{selector}{profile}
<key>HandoffAbi</key><string>unprovisioned-sptm-prefix</string>
<key>PhysicalBase</key><integer>0x40000000</integer><key>VirtualBase</key><integer>0xfffffe0000000000</integer>
<key>MemorySize</key><integer>67108864</integer><key>ActualMemorySize</key><integer>67108864</integer>
<key>KernelPhysical</key><integer>0x42000000</integer><key>DeviceTreePath</key><string>/authored.dt</string>
<key>InstructionBudget</key><integer>{budget}</integer></dict></dict></dict></dict></plist>"#).into_bytes()
}

#[test]
fn omission_preserves_lower_tier_results_with_or_without_platform() {
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
            65536,
            67108863,
            67108864,
            67108865,
            u64::MAX,
        ] {
            let input = fixture(budget, None, profile);
            assert_eq!(initialization(&input), limited(&input, 4096));
            assert_eq!(initialization(&input), long(&input));
            assert_eq!(initialization(&input), deep(&input));
        }
    }
}

#[test]
fn capability_selector_budget_matrix_preserves_all_old_ceilings() {
    for (selector, required) in [
        (INIT, 67108864),
        ("<string>long-65536</string>", 65536),
        ("<string>deep-16384</string>", 16384),
    ] {
        for profile in [false, true] {
            for budget in [
                0,
                8,
                64,
                256,
                1024,
                4096,
                16384,
                65536,
                67108863,
                67108864,
                67108865,
                u64::MAX,
            ] {
                let input = fixture(budget, Some(selector), profile);
                let result = initialization(&input);
                if profile && budget == required {
                    let mut expected = limited(&fixture(4096, None, true), 4096).unwrap();
                    expected.instruction_budget = required;
                    assert_eq!(result, Ok(expected));
                } else {
                    assert_eq!(result, Err(E::InvalidTraceConfiguration));
                }
                if selector == INIT {
                    assert_eq!(ordinary(&input), Err(E::InvalidTraceConfiguration));
                    assert_eq!(deep(&input), Err(E::InvalidTraceConfiguration));
                    assert_eq!(long(&input), Err(E::InvalidTraceConfiguration));
                    for limit in [64, 256, 1024, 4096, 16384, 65536, 67108864] {
                        assert_eq!(limited(&input, limit), Err(E::InvalidTraceConfiguration));
                    }
                } else {
                    assert_eq!(result, long(&input));
                }
            }
        }
    }
}

#[test]
fn exact_selector_type_duplicates_and_budget_encoding_are_required() {
    for name in [
        "",
        "67108864",
        "initialization-67108863",
        "initialization-67108865",
        "Initialization-67108864",
        "automatic",
    ] {
        assert_eq!(
            initialization(&fixture(
                67108864,
                Some(&format!("<string>{name}</string>")),
                true
            )),
            Err(E::InvalidTraceConfiguration)
        );
    }
    for body in [
        "<integer>67108864</integer>",
        "<true/>",
        "<dict/>",
        "<array/>",
    ] {
        assert_eq!(
            initialization(&fixture(67108864, Some(body), true)),
            Err(E::InvalidType)
        );
    }
    for second in [INIT, "<string>long-65536</string>"] {
        assert_eq!(
            initialization(&fixture(
                67108864,
                Some(&format!("{INIT}<key>DiagnosticTier</key>{second}")),
                true
            )),
            Err(E::DuplicateKey)
        );
    }
    let base = String::from_utf8(fixture(67108864, Some(INIT), true)).unwrap();
    for body in [
        "<true/>",
        "<string>67108864</string>",
        "<integer>-1</integer>",
        "<integer>18446744073709551616</integer>",
    ] {
        let bad = base.replace(
            "<key>InstructionBudget</key><integer>67108864</integer>",
            &format!("<key>InstructionBudget</key>{body}"),
        );
        assert!(initialization(bad.as_bytes()).is_err());
    }
    let duplicate = base.replace(
        "<key>InstructionBudget</key>",
        "<key>InstructionBudget</key><integer>67108864</integer><key>InstructionBudget</key>",
    );
    assert_eq!(initialization(duplicate.as_bytes()), Err(E::DuplicateKey));
}

#[test]
fn initialization_does_not_provision_or_relax_other_contracts() {
    let base = String::from_utf8(fixture(67108864, Some(INIT), true)).unwrap();
    for bad in [
        base.replace("unprovisioned-sptm-prefix", "automatic"),
        base.replace("0x40000000", "0x40000001"),
    ] {
        assert_eq!(
            initialization(bad.as_bytes()),
            Err(E::InvalidTraceConfiguration)
        );
    }
    assert!(initialization(
        base.replace("nextcore-irq-compat-v1", "automatic")
            .as_bytes()
    )
    .is_err());
    let bad = base.replace("<key>HandoffAbi</key>", "<key>Platform</key><dict><key>IrqLevel</key><integer>2</integer></dict><key>HandoffAbi</key>");
    assert_eq!(
        initialization(bad.as_bytes()),
        Err(E::InvalidPlatformConfiguration)
    );
    let video = base.replace(
        "<key>HandoffAbi</key>",
        "<key>Video</key><string>gop-framebuffer</string><key>HandoffAbi</key>",
    );
    assert!(initialization(video.as_bytes()).unwrap().video.is_some());
    assert!(initialization(video.replace("gop-framebuffer", "automatic").as_bytes()).is_err());
}
