use nextcore_core::boot_config::{
    parse_arm64_kernel_target as parse_arm, parse_arm64_trace_configuration,
    parse_kernel_target as parse_intel, BootConfigError as E, KernelProfile,
};

fn config(profile: &str, path: &str, args: &str) -> Vec<u8> {
    format!("<plist version=\"1.0\"><dict><key>Nextcore</key><dict><key>Kernel</key><dict><key>Profile</key><string>{profile}</string><key>Path</key><string>{path}</string><key>Arguments</key><string>{args}</string></dict></dict></dict></plist>").into_bytes()
}

fn trace_config(memory: u64, budget: u64) -> Vec<u8> {
    format!("<plist version=\"1.0\"><dict><key>Nextcore</key><dict><key>Kernel</key><dict><key>Profile</key><string>x86-efi-arm64-trace</string><key>Path</key><string>/kernel.kc</string><key>Trace</key><dict><key>HandoffAbi</key><string>unprovisioned-sptm-prefix</string><key>PhysicalBase</key><integer>0x40000000</integer><key>VirtualBase</key><integer>0xfffffe0000000000</integer><key>MemorySize</key><integer>{memory}</integer><key>ActualMemorySize</key><integer>67108864</integer><key>KernelPhysical</key><integer>0x42000000</integer><key>InstructionBudget</key><integer>{budget}</integer><key>DeviceTreePath</key><string>/dt.bin</string></dict></dict></dict></dict></plist>").into_bytes()
}

#[test]
fn original_trace_requires_explicit_bounded_coordinates_and_dt() {
    let input = trace_config(67108864, 8);
    let trace = parse_arm64_trace_configuration(&input).unwrap();
    assert_eq!(trace.physical_base, 0x40000000);
    assert_eq!(trace.kernel_phys, 0x42000000);
    assert_eq!(trace.virtual_base, 0xfffffe0000000000);
    assert_eq!(trace.device_tree_path, "\\dt.bin");
    assert_eq!(trace.instruction_budget, 8);
    assert_eq!(parse_intel(&input), Err(E::UnsupportedKernelProfile));
    for (memory, budget) in [
        (67108865, 8),
        (67108864, 0),
        (67108864, 9),
        (1 << 31, 8),
        (16 * 1024 * 1024, 8),
    ] {
        assert_eq!(
            parse_arm64_trace_configuration(&trace_config(memory, budget)),
            Err(E::InvalidTraceConfiguration)
        );
    }
    let text = String::from_utf8(input).unwrap();
    for wrong in [
        text.replace("unprovisioned-sptm-prefix", "legacy-x0-bootargs"),
        text.replace(
            "<key>HandoffAbi</key><string>unprovisioned-sptm-prefix</string>",
            "",
        ),
    ] {
        assert_eq!(
            parse_arm64_trace_configuration(wrong.as_bytes()),
            Err(E::InvalidTraceConfiguration)
        );
    }
    assert_eq!(
        parse_arm64_trace_configuration(&config("xnu-arm64-uefi", "/kernel", "")),
        Err(E::UnsupportedKernelProfile)
    );
}

#[test]
fn arm_profiles_are_explicit_and_intel_gate_remains_separate() {
    for (name, profile) in [
        ("xnu-arm64-uefi", KernelProfile::XnuArm64Uefi),
        ("qemu-virt-arm64-probe", KernelProfile::QemuVirtArm64Probe),
        (
            "x86-efi-arm64-jit-probe",
            KernelProfile::X86EfiArm64JitProbe,
        ),
    ] {
        let bytes = config(name, "/EFI/NEXTCORE/arm.kc", "-v");
        let target = parse_arm(&bytes).unwrap();
        assert_eq!(target.profile, profile);
        assert_eq!(target.path, "\\EFI\\NEXTCORE\\arm.kc");
        assert_eq!(target.arguments, "-v");
        assert_eq!(parse_intel(&bytes), Err(E::UnsupportedKernelProfile));
    }
    let intel = config("xnu-12377-pstart32", "/kernel", "");
    assert_eq!(
        parse_intel(&intel).unwrap().profile,
        KernelProfile::Xnu12377Pstart32
    );
    assert_eq!(parse_arm(&intel), Err(E::UnsupportedKernelProfile));
    assert_eq!(
        parse_arm(&config("automatic", "/kernel", "")),
        Err(E::UnsupportedKernelProfile)
    );
}

#[test]
fn arm_config_retains_bounds_and_path_rejection() {
    assert_eq!(
        parse_arm(&config("xnu-arm64-uefi", "/../kernel", "")),
        Err(E::InvalidPath)
    );
    assert_eq!(
        parse_arm(&config("xnu-arm64-uefi", "/kernel", "\n")),
        Err(E::InvalidArguments)
    );
    assert_eq!(
        parse_arm(&config("xnu-arm64-uefi", "/kernel", &"a".repeat(1024))),
        Err(E::ArgumentsTooLong)
    );
    assert_eq!(
        parse_arm(&config("xnu-arm64-uefi", "/kernel", &"a".repeat(1023)))
            .unwrap()
            .arguments
            .len(),
        1023
    );
}
