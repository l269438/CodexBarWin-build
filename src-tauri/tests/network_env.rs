use std::fs;

use codex_api_switcher::network_env::{
    apply_network_proxy_env, load_network_env_status, restore_network_proxy_env,
};

#[test]
fn apply_creates_codex_env_and_restore_removes_it_when_original_was_missing() {
    let temp = tempfile::tempdir().unwrap();
    let codex_home = temp.path().join("codex");
    let backup_root = temp.path().join("backup");

    let status = apply_network_proxy_env(&codex_home, &backup_root, "127.0.0.1:7890").unwrap();

    assert!(status.exists);
    assert!(status.managed);
    assert!(status.backup_exists);
    assert_eq!(status.proxy_endpoint.as_deref(), Some("127.0.0.1:7890"));
    let env = fs::read_to_string(codex_home.join(".env")).unwrap();
    assert!(env.contains("HTTP_PROXY=http://127.0.0.1:7890"));
    assert!(env.contains("ALL_PROXY=socks5://127.0.0.1:7890"));
    assert!(env.contains("NO_PROXY=localhost,127.0.0.1,::1"));

    restore_network_proxy_env(&codex_home, &backup_root).unwrap();

    assert!(!codex_home.join(".env").exists());
}

#[test]
fn restore_returns_existing_env_to_exact_original_contents() {
    let temp = tempfile::tempdir().unwrap();
    let codex_home = temp.path().join("codex");
    let backup_root = temp.path().join("backup");
    fs::create_dir_all(&codex_home).unwrap();
    let original = "CUSTOM_FLAG=1\nHTTPS_PROXY=http://old.proxy:8080\n";
    fs::write(codex_home.join(".env"), original).unwrap();

    apply_network_proxy_env(&codex_home, &backup_root, "127.0.0.1:7890").unwrap();
    fs::write(codex_home.join(".env"), "BROKEN=1\n").unwrap();
    restore_network_proxy_env(&codex_home, &backup_root).unwrap();

    assert_eq!(
        fs::read_to_string(codex_home.join(".env")).unwrap(),
        original
    );
}

#[test]
fn repeated_apply_replaces_the_managed_block_instead_of_appending_duplicates() {
    let temp = tempfile::tempdir().unwrap();
    let codex_home = temp.path().join("codex");
    let backup_root = temp.path().join("backup");

    apply_network_proxy_env(&codex_home, &backup_root, "127.0.0.1:7890").unwrap();
    let status = apply_network_proxy_env(&codex_home, &backup_root, "127.0.0.1:7891").unwrap();

    assert_eq!(status.proxy_endpoint.as_deref(), Some("127.0.0.1:7891"));
    let env = fs::read_to_string(codex_home.join(".env")).unwrap();
    assert_eq!(env.matches("BEGIN CODEXPILOT NETWORK PROXY").count(), 1);
    assert_eq!(env.matches("END CODEXPILOT NETWORK PROXY").count(), 1);
    assert!(!env.contains("127.0.0.1:7890"));
    assert!(env.contains("127.0.0.1:7891"));
}

#[test]
fn status_reads_existing_managed_proxy_configuration() {
    let temp = tempfile::tempdir().unwrap();
    let codex_home = temp.path().join("codex");
    let backup_root = temp.path().join("backup");

    apply_network_proxy_env(&codex_home, &backup_root, "127.0.0.1:7890").unwrap();
    let status = load_network_env_status(&codex_home, &backup_root).unwrap();

    assert!(status.configured);
    assert!(status.has_no_proxy);
    assert_eq!(status.proxy_endpoint.as_deref(), Some("127.0.0.1:7890"));
}
