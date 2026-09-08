use nextcore_core::boot_config::{
    parse_arm64_trace_configuration as parse, Arm64PlatformProfile, BootConfigError as E,
    parse_arm64_trace_configuration_with_limit as parse_with_limit,
    parse_arm64_trace_configuration_with_deep_tier as parse_deep,
};

fn config(profile: Option<&str>, options: Option<&str>, budget: u64) -> Vec<u8> {
    let profile = profile
        .map(|name| format!("<key>PlatformProfile</key><string>{name}</string>"))
        .unwrap_or_default();
    let options = options
        .map(|body| format!("<key>Platform</key><dict>{body}</dict>"))
        .unwrap_or_default();
    format!(r#"<plist version="1.0"><dict><key>Nextcore</key><dict><key>Kernel</key><dict>
    <key>Profile</key><string>x86-efi-arm64-trace</string><key>Path</key><string>/kernel.kc</string>
    <key>Trace</key><dict><key>HandoffAbi</key><string>unprovisioned-sptm-prefix</string>
    <key>PhysicalBase</key><integer>0x40000000</integer><key>VirtualBase</key><integer>0xfffffe0000000000</integer>
    <key>MemorySize</key><integer>67108864</integer><key>ActualMemorySize</key><integer>67108864</integer>
    <key>KernelPhysical</key><integer>0x42000000</integer><key>DeviceTreePath</key><string>/diagnostic.dt</string>
    <key>InstructionBudget</key><integer>{budget}</integer>{profile}{options}
    </dict></dict></dict></dict></plist>"#).into_bytes()
}
fn integer(name: &str, value: u64) -> String {
    format!("<key>{name}</key><integer>{value}</integer>")
}
const PROFILE: &str = "nextcore-irq-compat-v1";

#[test]
fn explicit_tiers_require_a_valid_ceiling_and_do_not_change_the_default_entry() {
    for maximum in [64, 256, 1024, 4096] {
        for budget in [1, 8, 63, 64, 256, 1024, 4096] {
            let input = config(Some(PROFILE), None, budget);
            assert_eq!(parse_with_limit(&input, maximum).is_ok(), budget <= maximum);
            assert_eq!(parse(&input).is_ok(), budget <= 64);
        }
    }
    for maximum in [0, 8, 65, 255, 1023, 4097, u64::MAX] {
        assert_eq!(parse_with_limit(&config(Some(PROFILE), None, 8), maximum), Err(E::InvalidTraceConfiguration));
    }
}

#[test]
fn unsupported_extended_budgets_never_become_arbitrary_long_runs() {
    for budget in [0, 65, 128, 255, 257, 512, 1023, 1025, 2048, 4095, 4097, u64::MAX] {
        assert_eq!(parse_with_limit(&config(Some(PROFILE), None, budget), 4096), Err(E::InvalidTraceConfiguration));
    }
}

#[test]
fn extended_ceiling_does_not_provision_a_profile_or_expand_its_absent_bound() {
    for maximum in [64, 256, 1024, 4096] {
        assert_eq!(parse_with_limit(&config(None, None, 8), maximum).unwrap().platform, None);
        for budget in [9, 64, 256, 1024, 4096] {
            assert_eq!(parse_with_limit(&config(None, None, budget), maximum), Err(E::InvalidTraceConfiguration));
        }
    }
}

#[test]
fn extended_configuration_retains_handoff_memory_and_platform_validation() {
    let valid = String::from_utf8(config(Some(PROFILE), None, 256)).unwrap();
    for input in [valid.replace("unprovisioned-sptm-prefix", "automatic"),
                  valid.replace("0x40000000", "0x40000001")] {
        assert_eq!(parse_with_limit(input.as_bytes(), 4096), Err(E::InvalidTraceConfiguration));
    }
    assert_eq!(parse_with_limit(&config(Some(PROFILE), Some(&integer("IrqLevel", 2)), 256), 4096), Err(E::InvalidPlatformConfiguration));
}

#[test]
fn absent_profile_retains_old_bound_and_does_not_create_a_provider() {
    assert_eq!(parse(&config(None, None, 8)).unwrap().platform, None);
    assert_eq!(
        parse(&config(None, None, 9)),
        Err(E::InvalidTraceConfiguration)
    );
    assert_eq!(
        parse(&config(None, Some(""), 8)),
        Err(E::InvalidPlatformConfiguration)
    );
    for name in ["", "automatic", "m1", "nextcore-irq-compat-v2"] {
        assert_eq!(
            parse(&config(Some(name), None, 8)),
            Err(E::InvalidPlatformConfiguration)
        );
    }
}

#[test]
fn software_profile_has_explicit_defaults_and_its_own_diagnostic_bound() {
    for options in [None, Some("")] {
        let configuration = parse(&config(Some(PROFILE), options, 64)).unwrap();
        let platform = configuration.platform.unwrap();
        assert_eq!(platform.profile, Arm64PlatformProfile::NextcoreIrqCompatV1);
        assert_eq!(
            (
                platform.initial_override,
                platform.initial_pstate,
                platform.vector_base
            ),
            (0, 0x3c5, 0)
        );
        assert!(!platform.irq_level && !platform.fiq_level);
    }
    assert!(parse(&config(Some(PROFILE), None, 1)).is_ok());
    for budget in [0, 65, u64::MAX] {
        assert_eq!(
            parse(&config(Some(PROFILE), None, budget)),
            Err(E::InvalidTraceConfiguration)
        );
    }
}

#[test]
fn only_defined_override_fields_pstate_modes_and_boolean_levels_are_accepted() {
    for irq in [0, 2] {
        for fiq in [0, 2] {
            let value = (irq << 20) | (fiq << 22);
            let options = integer("InitialOverride", value);
            assert_eq!(
                parse(&config(Some(PROFILE), Some(&options), 8))
                    .unwrap()
                    .platform
                    .unwrap()
                    .initial_override,
                value
            );
        }
    }
    for value in [1, 1 << 20, 3 << 20, 1 << 22, 3 << 22, 1 << 24, u64::MAX] {
        assert_eq!(
            parse(&config(
                Some(PROFILE),
                Some(&integer("InitialOverride", value)),
                8
            )),
            Err(E::InvalidPlatformConfiguration)
        );
    }
    for mode in [4, 5] {
        for flags in [0, 0x3c0, 0xf0000000, 0xf00003c0] {
            assert!(parse(&config(
                Some(PROFILE),
                Some(&integer("InitialPstate", mode | flags)),
                8
            ))
            .is_ok());
        }
    }
    for value in [0, 1, 0x3c0, 0x3c8, 0x3c9, 0x3d5, 0x100000005, u64::MAX] {
        assert_eq!(
            parse(&config(
                Some(PROFILE),
                Some(&integer("InitialPstate", value)),
                8
            )),
            Err(E::InvalidPlatformConfiguration)
        );
    }
    for name in ["IrqLevel", "FiqLevel"] {
        assert!(parse(&config(Some(PROFILE), Some(&integer(name, 1)), 8)).is_ok());
        assert_eq!(
            parse(&config(Some(PROFILE), Some(&integer(name, 2)), 8)),
            Err(E::InvalidPlatformConfiguration)
        );
    }
}

#[test]
fn vector_table_must_be_aligned_and_fully_inside_guest_ram() {
    for address in [0, 0x40000000, 0x43fff800] {
        assert!(parse(&config(
            Some(PROFILE),
            Some(&integer("VectorBase", address)),
            8
        ))
        .is_ok());
    }
    for address in [0x40000001, 0x3ffff800, 0x44000000, u64::MAX & !2047] {
        assert_eq!(
            parse(&config(
                Some(PROFILE),
                Some(&integer("VectorBase", address)),
                8
            )),
            Err(E::InvalidPlatformConfiguration)
        );
    }
}

#[test]
fn unknown_types_duplicates_and_unrecognized_options_are_not_silently_ignored() {
    assert_eq!(
        parse(&config(
            Some(PROFILE),
            Some(&integer("InitialPState", 5)),
            8
        )),
        Err(E::InvalidPlatformConfiguration)
    );
    assert_eq!(
        parse(&config(
            Some(PROFILE),
            Some("<key>IrqLevel</key><true/>"),
            8
        )),
        Err(E::InvalidType)
    );
    assert_eq!(
        parse(&config(
            Some(PROFILE),
            Some("<key>IrqLevel</key><integer>-1</integer>"),
            8
        )),
        Err(E::InvalidPlatformConfiguration)
    );
    let repeated = integer("IrqLevel", 0) + &integer("IrqLevel", 1);
    assert_eq!(
        parse(&config(Some(PROFILE), Some(&repeated), 8)),
        Err(E::DuplicateKey)
    );
}

fn selected(input: Vec<u8>, body: &str) -> Vec<u8> {
    String::from_utf8(input).unwrap().replace("<key>HandoffAbi</key>",
        &format!("<key>DiagnosticTier</key>{body}<key>HandoffAbi</key>")).into_bytes()
}
#[test]
fn deep_capability_preserves_existing_entries_and_unselected_tiers() {
    for profile in [None, Some(PROFILE)] {
        for budget in [1,8,9,64,65,256,512,1024,4096,8192,16384,16385] {
            let input=config(profile,None,budget);
            assert_eq!(parse_deep(&input),parse_with_limit(&input,4096));
        }
    }
    assert_eq!(parse_with_limit(&config(Some(PROFILE),None,8),16384),Err(E::InvalidTraceConfiguration));
    let input=selected(config(Some(PROFILE),None,16384),"<string>deep-16384</string>");
    assert_eq!(parse_deep(&input).unwrap().instruction_budget,16384);
    assert_eq!(parse(&input),Err(E::InvalidTraceConfiguration));
    for limit in [64,256,1024,4096] {
        assert_eq!(parse_with_limit(&input,limit),Err(E::InvalidTraceConfiguration));
        let lower=selected(config(Some(PROFILE),None,8),"<string>deep-16384</string>");
        assert_eq!(parse_with_limit(&lower,limit),Err(E::InvalidTraceConfiguration));
    }
}
#[test]
fn deep_selector_requires_exact_type_value_budget_and_profile() {
    for budget in [0,8,64,256,1024,4096,8192,16383,16385,u64::MAX] {
        let input=selected(config(Some(PROFILE),None,budget),"<string>deep-16384</string>");
        assert_eq!(parse_deep(&input),Err(E::InvalidTraceConfiguration));
    }
    assert_eq!(parse_deep(&selected(config(None,None,16384),"<string>deep-16384</string>")),Err(E::InvalidTraceConfiguration));
    for name in ["", "16384", "deep-16385", "automatic"] {
        assert_eq!(parse_deep(&selected(config(Some(PROFILE),None,16384),&format!("<string>{name}</string>"))),Err(E::InvalidTraceConfiguration));
    }
    for body in ["<integer>16384</integer>","<true/>","<dict/>"] {
        assert_eq!(parse_deep(&selected(config(Some(PROFILE),None,16384),body)),Err(E::InvalidType));
    }
    let duplicate="<string>deep-16384</string><key>DiagnosticTier</key><string>deep-16384</string>";
    assert_eq!(parse_deep(&selected(config(Some(PROFILE),None,16384),duplicate)),Err(E::DuplicateKey));
}
#[test]
fn deep_selection_retains_handoff_placement_and_platform_validation() {
    let valid=String::from_utf8(selected(config(Some(PROFILE),None,16384),"<string>deep-16384</string>")).unwrap();
    for wrong in [valid.replace("unprovisioned-sptm-prefix","automatic"),valid.replace("0x40000000","0x40000001")] {
        assert_eq!(parse_deep(wrong.as_bytes()),Err(E::InvalidTraceConfiguration));
    }
    let wrong=selected(config(Some(PROFILE),Some(&integer("IrqLevel",2)),16384),"<string>deep-16384</string>");
    assert_eq!(parse_deep(&wrong),Err(E::InvalidPlatformConfiguration));
}
