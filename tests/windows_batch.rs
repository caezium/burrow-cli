//! The std batch adapter escapes arguments but cannot preserve percent sequences
//! in the program path. Refuse that path before cmd can select a different file.
#![cfg(windows)]

#[test]
fn a_batch_program_path_cannot_select_a_different_engine_by_expansion() {
    let root = std::env::temp_dir().join(format!("burrow_batch_path_{}", std::process::id()));
    let original = root.join("literal%BURROW_BATCH_VALUE%");
    let expanded = root.join("literalEXPANDED");
    let marker = root.join("unexpected.txt");
    for directory in [&original, &expanded] {
        std::fs::create_dir_all(directory).unwrap();
        std::fs::write(
            directory.join("burrow-engine.cmd"),
            "@echo off\r\necho invoked>\"%BURROW_BATCH_MARKER%\"\r\necho {\"ok\":true,\"data\":{\"unexpected\":true}}\r\n",
        )
        .unwrap();
    }
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_burrow"))
        .env("BURROW_ENGINE_DIR", &original)
        .env_remove("BURROW_ENGINE")
        .env("BURROW_BATCH_VALUE", "EXPANDED")
        .env("BURROW_BATCH_MARKER", &marker)
        .env("BURROW_TELEMETRY", "0")
        .env("BURROW_CONFIG_DIR", root.join("config"))
        .args(["clean", "--json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let envelope: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["ok"], false);
    assert!(envelope["error"]["message"]
        .as_str()
        .unwrap()
        .contains("batch program path"));
    assert!(
        !marker.exists(),
        "neither the requested nor expanded script may launch"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_batch_argument_cannot_absorb_the_following_preview_guard() {
    let root = std::env::temp_dir().join(format!("burrow_batch_arg_{}", std::process::id()));
    let marker = root.join("unexpected.txt");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("burrow-engine.cmd"),
        "@echo off\r\necho invoked>\"%BURROW_BATCH_MARKER%\"\r\necho {\"ok\":true,\"data\":{}}\r\n",
    )
    .unwrap();
    for argument in ["quoted\"&echo.unexpected", "name\nline", "name\rline"] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_burrow"))
            .env("BURROW_ENGINE_DIR", &root)
            .env_remove("BURROW_ENGINE")
            .env("BURROW_BATCH_MARKER", &marker)
            .env("BURROW_TELEMETRY", "0")
            .env("BURROW_CONFIG_DIR", root.join("config"))
            .args(["uninstall", argument, "--json"])
            .output()
            .unwrap();
        assert!(!output.status.success());
        let envelope: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(envelope["ok"], false);
        assert!(envelope["error"]["message"]
            .as_str()
            .unwrap()
            .contains("batch arguments"));
        assert!(!marker.exists());
    }
    std::fs::remove_dir_all(root).unwrap();
}
