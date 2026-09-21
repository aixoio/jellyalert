use std::{fs, process::Command};

#[test]
fn file_logs_survive_restarts_and_match_console_output() {
    let directory = std::env::temp_dir().join(format!(
        "jellyalert-logs-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap()
    ));
    fs::create_dir_all(&directory).unwrap();
    let config = directory.join("config.toml");
    // A directory cannot be opened as SQLite: fail before starting network workers.
    fs::write(
        &config,
        format!(
            r#"
sonarr_url = "http://127.0.0.1:8989"
sonarr_api_key = "unused"
discord_webhook_url = "https://discord.com/api/webhooks/unused/unused"
sqlite_database_path = {:?}
log_level = "info"
"#,
            directory.to_str().unwrap()
        ),
    )
    .unwrap();
    let logs = directory.join("logs");
    for _ in 0..2 {
        let output = Command::new(env!("CARGO_BIN_EXE_server-core"))
            .arg(&config)
            .env("LOG_DIRECTORY", &logs)
            .env_remove("RUST_LOG")
            .output()
            .unwrap();
        assert!(!output.status.success());
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.contains("starting Jelly Alert server core"));
        assert!(!stdout.contains('\u{1b}'));
        assert!(
            fs::read_dir(&logs)
                .unwrap()
                .any(|entry| { fs::read_to_string(entry.unwrap().path()).unwrap() == stdout })
        );
    }
    let files: Vec<_> = fs::read_dir(&logs).unwrap().collect();
    assert_eq!(files.len(), 2, "restarting must retain previous logs");
    for file in files {
        let name = file.unwrap().file_name().to_string_lossy().into_owned();
        assert!(name.starts_with("backend-"));
        assert!(name.contains("Z-"));
        assert!(name.ends_with(".log"));
    }
    fs::remove_dir_all(directory).unwrap();
}
