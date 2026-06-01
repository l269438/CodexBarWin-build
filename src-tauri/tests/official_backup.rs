use std::fs;

use codex_api_switcher::official_backup::{
    create_original_backup, load_original_backup_status, restore_original_backup,
};

#[test]
fn backup_and_restore_returns_codex_files_to_original_contents() {
    let temp = tempfile::tempdir().unwrap();
    let codex_home = temp.path().join("codex");
    let backup_root = temp.path().join("backup");
    fs::create_dir_all(&codex_home).unwrap();
    fs::write(codex_home.join("auth.json"), r#"{"token":"official"}"#).unwrap();
    fs::write(codex_home.join("config.toml"), "model = \"official\"\n").unwrap();

    let status = create_original_backup(&codex_home, &backup_root).unwrap();
    assert!(status.exists);
    assert!(status.auth_json_backed_up);
    assert!(status.config_toml_backed_up);

    fs::write(codex_home.join("auth.json"), r#"{"token":"managed"}"#).unwrap();
    fs::write(
        codex_home.join("config.toml"),
        "model_provider = \"codex-api-switcher\"\n",
    )
    .unwrap();

    restore_original_backup(&codex_home, &backup_root).unwrap();

    assert_eq!(
        fs::read_to_string(codex_home.join("auth.json")).unwrap(),
        r#"{"token":"official"}"#
    );
    assert_eq!(
        fs::read_to_string(codex_home.join("config.toml")).unwrap(),
        "model = \"official\"\n"
    );
}

#[test]
fn restore_removes_files_that_were_missing_when_backup_was_created() {
    let temp = tempfile::tempdir().unwrap();
    let codex_home = temp.path().join("codex");
    let backup_root = temp.path().join("backup");
    fs::create_dir_all(&codex_home).unwrap();
    fs::write(codex_home.join("auth.json"), r#"{"token":"official"}"#).unwrap();

    create_original_backup(&codex_home, &backup_root).unwrap();
    fs::write(
        codex_home.join("config.toml"),
        "model_provider = \"codex-api-switcher\"\n",
    )
    .unwrap();

    restore_original_backup(&codex_home, &backup_root).unwrap();

    assert!(codex_home.join("auth.json").exists());
    assert!(!codex_home.join("config.toml").exists());
}

#[test]
fn backup_is_not_overwritten_once_original_snapshot_exists() {
    let temp = tempfile::tempdir().unwrap();
    let codex_home = temp.path().join("codex");
    let backup_root = temp.path().join("backup");
    fs::create_dir_all(&codex_home).unwrap();
    fs::write(codex_home.join("auth.json"), r#"{"token":"official"}"#).unwrap();

    create_original_backup(&codex_home, &backup_root).unwrap();
    fs::write(codex_home.join("auth.json"), r#"{"token":"modified"}"#).unwrap();
    create_original_backup(&codex_home, &backup_root).unwrap();
    restore_original_backup(&codex_home, &backup_root).unwrap();

    assert_eq!(
        fs::read_to_string(codex_home.join("auth.json")).unwrap(),
        r#"{"token":"official"}"#
    );
    assert!(load_original_backup_status(&backup_root).unwrap().exists);
}
