//! Windows-only process tests for the legacy PowerShell/Go engine adapter.
//! Every engine here is an argv-recording fixture in a new temporary directory.
#![cfg(windows)]

use std::path::PathBuf;
use std::process::{Command, Output};

struct Fixture {
    root: PathBuf,
    engine: PathBuf,
    marker: PathBuf,
}

impl Fixture {
    fn new(directory: &str, extension: &str) -> Self {
        let id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("burrow_batch_{}_{id}", std::process::id()));
        let engine = root.join(directory);
        let marker = root.join("invoked.txt");
        std::fs::create_dir_all(&engine).unwrap();
        std::fs::write(
            engine.join(format!("burrow-engine.{extension}")),
            "@echo off\r\npowershell.exe -NoProfile -ExecutionPolicy Bypass -File \"%~dp0echo.ps1\" %*\r\nexit /b %errorlevel%\r\n",
        )
        .unwrap();
        std::fs::write(
            engine.join("echo.ps1"),
            r#"[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
[System.IO.File]::WriteAllText($env:BURROW_INVOCATION_MARKER, 'invoked')
$items = @($args | ForEach-Object { $_ | ConvertTo-Json -Compress })
Write-Output ('{"invoked":[' + ($items -join ',') + ']}')
"#,
        )
        .unwrap();
        Self {
            root,
            engine,
            marker,
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_burrow"))
            .env("BURROW_ENGINE_DIR", &self.engine)
            .env("BURROW_CONFIG_DIR", self.root.join("config"))
            .env("BURROW_TELEMETRY", "0")
            .env("BURROW_BATCH_VALUE", "EXPANDED")
            .env("BURROW_INVOCATION_MARKER", &self.marker)
            .args(args)
            .output()
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn envelope(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "one envelope expected ({error}): {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

#[test]
fn cmd_and_bat_forward_shell_characters_as_single_arguments() {
    for extension in ["cmd", "bat"] {
        let fixture = Fixture::new("portable&review", extension);
        for argument in [
            "cache&echo.BURROW_ARG_INJECTION",
            "cache|echo.BURROW_ARG_INJECTION",
            "literal%BURROW_BATCH_VALUE%",
            "literal!BURROW_BATCH_VALUE!",
            "literal^caret",
            "space and 'single' quotes",
            "C:\\cache with spaces\\",
        ] {
            let output = fixture.run(&["uninstall", argument, "--json"]);
            assert!(output.status.success(), "{extension}: {output:?}");
            assert_eq!(
                envelope(&output)["data"]["invoked"],
                serde_json::json!(["uninstall", argument, "--dry-run"])
            );
        }
    }
}

#[test]
fn portable_paths_preserve_the_apps_preview_and_apply_mapping() {
    for extension in ["cmd", "bat"] {
        let fixture = Fixture::new("portable & (review)^!", extension);
        for command in ["clean", "optimize"] {
            let preview = fixture.run(&[command, "--json"]);
            assert!(preview.status.success(), "{preview:?}");
            assert_eq!(
                envelope(&preview)["data"]["invoked"],
                serde_json::json!([command, "--dry-run"])
            );
            let apply = fixture.run(&[command, "--apply", "--json"]);
            assert!(apply.status.success(), "{apply:?}");
            assert_eq!(
                envelope(&apply)["data"]["invoked"],
                serde_json::json!([command])
            );
        }
    }
}

#[test]
fn batch_unrepresentable_arguments_are_refused_without_launching_the_script() {
    for argument in [
        "quoted\"&echo.BURROW_ARG_INJECTION",
        "name\necho.BURROW_ARG_INJECTION",
        "name\recho.BURROW_ARG_INJECTION",
    ] {
        let fixture = Fixture::new("safe", "cmd");
        let output = fixture.run(&["uninstall", argument, "--json"]);
        assert!(!output.status.success());
        assert_eq!(envelope(&output)["ok"], false);
        assert!(envelope(&output)["error"]["message"]
            .as_str()
            .unwrap()
            .contains("batch arguments"));
        assert!(!fixture.marker.exists());
    }
}

#[test]
fn percent_bearing_batch_program_paths_are_refused_without_launch() {
    for extension in ["cmd", "bat"] {
        let fixture = Fixture::new("literal%BURROW_BATCH_VALUE%", extension);
        let output = fixture.run(&["clean", "--json"]);
        assert!(!output.status.success());
        assert_eq!(envelope(&output)["ok"], false);
        assert!(!fixture.marker.exists());
    }
}
