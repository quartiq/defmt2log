use std::process::Command;

// libtest executables are not reliable `defmt2log` targets, so this test
// exercises the real host-binary path by spawning the internal helper binary.
#[test]
fn helper_binary_decodes_typed_hints_and_location() {
    let output = Command::new(env!("CARGO_BIN_EXE_smoke_helper"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");

    let stderr = String::from_utf8(output.stderr).unwrap();
    let lines: Vec<_> = stderr.trim().lines().collect();
    let line = lines[0];
    let parts: Vec<_> = line.split('|').collect();
    assert_eq!(parts.len(), 6, "unexpected output: {line}");
    assert_eq!(parts[0], "INFO");
    assert!(
        parts[1].contains("smoke_helper"),
        "unexpected target: {line}"
    );
    assert!(
        parts[2].contains("smoke_helper"),
        "unexpected module: {line}"
    );
    assert!(
        parts[3].ends_with("defmt2log/src/bin/smoke_helper.rs"),
        "unexpected file: {line}"
    );
    assert!(
        parts[4].parse::<u32>().unwrap() > 0,
        "unexpected line: {line}"
    );

    assert_eq!(parts[5], "word 0x00001234");
}

#[test]
fn invalid_elf_is_rejected() {
    let err = defmt2log::init_from_merged_elf_bytes(b"not-an-elf").unwrap_err();
    match err {
        defmt2log::InitError::ParseTable(_) => {}
        other => panic!("unexpected init error: {other}"),
    }
}

#[test]
fn explicit_elf_path_helper_still_works() {
    let helper = env!("CARGO_BIN_EXE_smoke_helper");
    let output = Command::new(helper)
        .env("DEFMT2LOG_SELF_CHECK", helper)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
}

#[test]
fn empty_helper_initializes_with_no_live_entries() {
    let output = Command::new(env!("CARGO_BIN_EXE_empty_helper"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
}
