use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use codex_api_switcher::sessions::{
    copy_session_to_codex_home, delete_project_sessions, delete_session, group_sessions_by_project,
    group_sessions_by_project_with_codex_order, group_visible_sessions_by_project_with_codex_order,
    list_deleted_session_entries, list_sessions, preview_session, restore_deleted_sessions,
};
use rusqlite::Connection;

fn write_session(codex_home: &std::path::Path, relative: &str, body: &str) -> std::path::PathBuf {
    let path = codex_home.join("sessions").join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, body).unwrap();
    path
}

fn write_thread_index(
    codex_home: &std::path::Path,
    rows: &[(&str, &str, &str, &str, i64, i64, i64)],
) {
    let db_path = codex_home.join("state_5.sqlite");
    fs::create_dir_all(codex_home).unwrap();
    let connection = Connection::open(db_path).unwrap();
    connection
        .execute_batch(
            r#"
            CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                rollout_path TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                cwd TEXT NOT NULL DEFAULT '',
                title TEXT NOT NULL,
                source TEXT NOT NULL DEFAULT 'vscode',
                model_provider TEXT NOT NULL DEFAULT 'openai',
                cli_version TEXT NOT NULL DEFAULT '0.135.0-alpha.1',
                archived INTEGER NOT NULL DEFAULT 0,
                preview TEXT NOT NULL DEFAULT '',
                created_at_ms INTEGER,
                updated_at_ms INTEGER,
                thread_source TEXT
            );
            "#,
        )
        .unwrap();
    for (id, title, relative_path, cwd, created_at, updated_at, archived) in rows {
        let rollout_path = codex_home
            .join("sessions")
            .join(relative_path)
            .to_string_lossy()
            .into_owned();
        connection
            .execute(
                "INSERT INTO threads (id, rollout_path, created_at, updated_at, cwd, title, archived, preview, created_at_ms, updated_at_ms, thread_source) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                rusqlite::params![
                    id,
                    rollout_path,
                    created_at,
                    updated_at,
                    cwd,
                    title,
                    archived,
                    "indexed preview",
                    created_at * 1000,
                    updated_at * 1000,
                    "user",
                ],
            )
            .unwrap();
    }
}

fn write_session_name_index(codex_home: &std::path::Path, rows: &[(&str, &str)]) {
    fs::create_dir_all(codex_home).unwrap();
    let body = rows
        .iter()
        .map(|(id, name)| {
            serde_json::json!({
                "id": id,
                "thread_name": name,
                "updated_at": "2026-06-06T00:00:00Z"
            })
            .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(codex_home.join("session_index.jsonl"), format!("{body}\n")).unwrap();
}

fn write_process_activity_index(codex_home: &std::path::Path, rows: &[(&str, i64)]) {
    let process_dir = codex_home.join("process_manager");
    fs::create_dir_all(&process_dir).unwrap();
    let body = rows
        .iter()
        .map(|(id, updated_at_ms)| {
            serde_json::json!({
                "conversationId": id,
                "updatedAtMs": updated_at_ms,
                "startedAtMs": updated_at_ms - 1000
            })
        })
        .collect::<Vec<_>>();
    fs::write(
        process_dir.join("chat_processes.json"),
        serde_json::to_string(&body).unwrap(),
    )
    .unwrap();
}

fn write_project_order(codex_home: &std::path::Path, rows: &[&str]) {
    fs::create_dir_all(codex_home).unwrap();
    let body = serde_json::json!({
        "project-order": rows,
    });
    fs::write(
        codex_home.join(".codex-global-state.json"),
        serde_json::to_string(&body).unwrap(),
    )
    .unwrap();
}

fn write_saved_workspace_roots(codex_home: &std::path::Path, rows: &[&str]) {
    fs::create_dir_all(codex_home).unwrap();
    let body = serde_json::json!({
        "electron-saved-workspace-roots": rows,
        "project-order": rows,
    });
    fs::write(
        codex_home.join(".codex-global-state.json"),
        serde_json::to_string(&body).unwrap(),
    )
    .unwrap();
}

fn write_saved_workspace_roots_with_labels(
    codex_home: &std::path::Path,
    rows: &[&str],
    labels: &[(&str, &str)],
) {
    fs::create_dir_all(codex_home).unwrap();
    let labels = labels
        .iter()
        .map(|(path, label)| {
            (
                path.to_string(),
                serde_json::Value::String(label.to_string()),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    let body = serde_json::json!({
        "electron-saved-workspace-roots": rows,
        "project-order": rows,
        "active-workspace-roots": rows,
        "electron-workspace-root-labels": labels,
        "unrelated-setting": true,
    });
    fs::write(
        codex_home.join(".codex-global-state.json"),
        serde_json::to_string(&body).unwrap(),
    )
    .unwrap();
}

fn test_now_unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

#[test]
fn list_sessions_extracts_metadata_from_jsonl_files() {
    let temp = tempfile::tempdir().unwrap();
    let codex_home = temp.path().join("codex");
    write_session(
        &codex_home,
        "2026/06/06/rollout-2026-06-06T09-00-00-session-a.jsonl",
        r#"{"timestamp":"2026-06-06T01:00:00Z","type":"session_meta","payload":{"id":"session-a","timestamp":"2026-06-06T01:00:00Z","cwd":"/tmp/project"}}
{"timestamp":"2026-06-06T01:00:02Z","type":"event_msg","payload":{"type":"user_message","message":"Manage Codex sessions"}}
{"timestamp":"2026-06-06T01:03:04Z","type":"event_msg","payload":{"type":"agent_message","message":"Done."}}
"#,
    );

    let sessions = list_sessions(&codex_home).unwrap();

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, "session-a");
    assert_eq!(sessions[0].title, "Manage Codex sessions");
    assert_eq!(sessions[0].created_at, 1_780_707_600);
    assert_eq!(sessions[0].updated_at, 1_780_707_784);
    assert_eq!(sessions[0].message_count, 2);
    assert!(sessions[0].relative_path.ends_with("session-a.jsonl"));
}

#[test]
fn list_sessions_prefers_codex_thread_index_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let codex_home = temp.path().join("codex");
    write_session(
        &codex_home,
        "2026/06/06/rollout-2026-06-06T09-00-00-session-old.jsonl",
        r#"{"timestamp":"2026-06-06T01:00:00Z","type":"session_meta","payload":{"id":"session-old"}}
{"timestamp":"2026-06-06T01:00:01Z","type":"event_msg","payload":{"type":"user_message","message":"Raw title one"}}
"#,
    );
    write_session(
        &codex_home,
        "2026/06/06/rollout-2026-06-06T10-00-00-session-new.jsonl",
        r#"{"timestamp":"2026-06-06T02:00:00Z","type":"session_meta","payload":{"id":"session-new"}}
{"timestamp":"2026-06-06T02:00:01Z","type":"event_msg","payload":{"type":"user_message","message":"Raw title two"}}
"#,
    );
    write_session(
        &codex_home,
        "2026/06/06/rollout-2026-06-06T11-00-00-session-archived.jsonl",
        r#"{"timestamp":"2026-06-06T03:00:00Z","type":"session_meta","payload":{"id":"session-archived"}}
{"timestamp":"2026-06-06T03:00:01Z","type":"event_msg","payload":{"type":"user_message","message":"Archived raw title"}}
"#,
    );
    write_thread_index(
        &codex_home,
        &[
            (
                "session-old",
                "Left sidebar older title",
                "2026/06/06/rollout-2026-06-06T09-00-00-session-old.jsonl",
                "/tmp/project-a",
                1_780_707_600,
                1_780_707_700,
                0,
            ),
            (
                "session-new",
                "Left sidebar newest title",
                "2026/06/06/rollout-2026-06-06T10-00-00-session-new.jsonl",
                "/tmp/project-a",
                1_780_707_800,
                1_780_708_000,
                0,
            ),
            (
                "session-archived",
                "Archived sidebar title",
                "2026/06/06/rollout-2026-06-06T11-00-00-session-archived.jsonl",
                "/tmp/project-a",
                1_780_708_100,
                1_780_708_200,
                1,
            ),
        ],
    );

    let sessions = list_sessions(&codex_home).unwrap();

    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0].id, "session-new");
    assert_eq!(sessions[0].title, "Left sidebar newest title");
    assert_eq!(sessions[0].updated_at, 1_780_708_000);
    assert_eq!(sessions[0].project_path, "/tmp/project-a");
    assert_eq!(sessions[0].project_name, "project-a");
    assert_eq!(sessions[1].id, "session-old");
    assert_eq!(sessions[1].title, "Left sidebar older title");
}

#[test]
fn list_sessions_prefers_codex_sidebar_title_from_session_index() {
    let temp = tempfile::tempdir().unwrap();
    let codex_home = temp.path().join("codex");
    write_thread_index(
        &codex_home,
        &[(
            "session-a",
            "[repo/link] Clone this repository",
            "2026/06/06/rollout-session-a.jsonl",
            "/tmp/project-a",
            1_780_707_600,
            1_780_707_700,
            0,
        )],
    );
    write_session_name_index(&codex_home, &[("session-a", "克隆 CodexBarWin-build")]);

    let sessions = list_sessions(&codex_home).unwrap();

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].title, "克隆 CodexBarWin-build");
    assert_eq!(sessions[0].updated_at, 1_780_707_700);
}

#[test]
fn list_sessions_filters_non_user_sidebar_threads() {
    let temp = tempfile::tempdir().unwrap();
    let codex_home = temp.path().join("codex");
    write_thread_index(
        &codex_home,
        &[
            (
                "visible-session",
                "Visible user session",
                "2026/06/06/rollout-visible.jsonl",
                "/tmp/project-a",
                1_780_707_600,
                1_780_707_900,
                0,
            ),
            (
                "hidden-session",
                "Hidden non-user session",
                "2026/06/06/rollout-hidden.jsonl",
                "/tmp/project-a",
                1_780_707_600,
                1_780_708_000,
                0,
            ),
        ],
    );
    let connection = Connection::open(codex_home.join("state_5.sqlite")).unwrap();
    connection
        .execute(
            "UPDATE threads SET thread_source = '' WHERE id = ?1",
            rusqlite::params!["hidden-session"],
        )
        .unwrap();

    let sessions = list_sessions(&codex_home).unwrap();

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, "visible-session");
}

#[test]
fn list_sessions_matches_codex_sidebar_index_when_available() {
    let temp = tempfile::tempdir().unwrap();
    let codex_home = temp.path().join("codex");
    write_thread_index(
        &codex_home,
        &[
            (
                "visible-user-session",
                "Raw visible user title",
                "2026/06/06/rollout-visible-user.jsonl",
                "/tmp/project-a",
                1_780_707_600,
                1_780_708_200,
                0,
            ),
            (
                "legacy-null-source",
                "Raw legacy title",
                "2026/06/06/rollout-visible.jsonl",
                "/tmp/project-a",
                1_780_707_600,
                1_780_708_100,
                0,
            ),
            (
                "hidden-user-session",
                "1",
                "2026/06/06/rollout-hidden-user.jsonl",
                "/tmp/project-a",
                1_780_707_600,
                1_780_708_000,
                0,
            ),
            (
                "hidden-subagent-session",
                "Review hidden task",
                "2026/06/06/rollout-hidden-subagent.jsonl",
                "/tmp/project-a",
                1_780_707_600,
                1_780_707_900,
                0,
            ),
        ],
    );
    let connection = Connection::open(codex_home.join("state_5.sqlite")).unwrap();
    connection
        .execute(
            "UPDATE threads SET thread_source = NULL WHERE id = ?1",
            rusqlite::params!["legacy-null-source"],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE threads SET thread_source = ?1 WHERE id = ?2",
            rusqlite::params!["subagent", "hidden-subagent-session"],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE threads SET model_provider = ?1 WHERE id = ?2",
            rusqlite::params!["custom", "hidden-user-session"],
        )
        .unwrap();
    write_session_name_index(
        &codex_home,
        &[
            ("visible-user-session", "Codex sidebar title"),
            ("legacy-null-source", "Legacy sidebar title"),
            ("hidden-subagent-session", "Subagent sidebar title"),
        ],
    );

    let sessions = list_sessions(&codex_home).unwrap();

    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0].id, "visible-user-session");
    assert_eq!(sessions[0].title, "Codex sidebar title");
    assert_eq!(sessions[1].id, "legacy-null-source");
    assert_eq!(sessions[1].title, "Legacy sidebar title");
}

#[test]
fn list_sessions_includes_unindexed_official_user_threads() {
    let temp = tempfile::tempdir().unwrap();
    let codex_home = temp.path().join("codex");
    write_thread_index(
        &codex_home,
        &[
            (
                "indexed-session",
                "Raw indexed title",
                "2026/06/06/rollout-indexed.jsonl",
                "/tmp/project-a",
                1_780_707_600,
                1_780_708_100,
                0,
            ),
            (
                "official-user-session",
                "你好",
                "2026/06/06/rollout-official-user.jsonl",
                "/tmp/project-a",
                1_780_707_600,
                1_780_708_000,
                0,
            ),
            (
                "custom-user-session",
                "1",
                "2026/06/06/rollout-custom-user.jsonl",
                "/tmp/project-a",
                1_780_707_600,
                1_780_707_900,
                0,
            ),
        ],
    );
    let connection = Connection::open(codex_home.join("state_5.sqlite")).unwrap();
    connection
        .execute(
            "UPDATE threads SET model_provider = ?1 WHERE id = ?2",
            rusqlite::params!["custom", "custom-user-session"],
        )
        .unwrap();
    write_session_name_index(&codex_home, &[("indexed-session", "Indexed sidebar title")]);

    let sessions = list_sessions(&codex_home).unwrap();
    let session_ids = sessions
        .iter()
        .map(|session| session.id.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        session_ids,
        vec!["indexed-session", "official-user-session"]
    );
    assert_eq!(sessions[0].title, "Indexed sidebar title");
    assert_eq!(sessions[1].title, "你好");
}

#[test]
fn list_sessions_includes_legacy_openai_sidebar_threads_without_user_source() {
    let temp = tempfile::tempdir().unwrap();
    let codex_home = temp.path().join("codex");
    let now = test_now_unix_timestamp();
    let recent = now - 60;
    let stale = now - 31 * 24 * 60 * 60;
    write_thread_index(
        &codex_home,
        &[
            (
                "current-legacy-session",
                "Current legacy sidebar title",
                "2026/06/06/rollout-current-legacy.jsonl",
                "/tmp/project-a",
                recent - 10,
                recent,
                0,
            ),
            (
                "old-legacy-session",
                "Old legacy sidebar title",
                "2026/06/06/rollout-old-legacy.jsonl",
                "/tmp/project-a",
                recent - 20,
                recent - 10,
                0,
            ),
            (
                "stale-legacy-session",
                "Stale legacy sidebar title",
                "2026/06/06/rollout-stale-legacy.jsonl",
                "/tmp/project-a",
                stale - 10,
                stale,
                0,
            ),
            (
                "legacy-subagent-session",
                "Legacy subagent sidebar title",
                "2026/06/06/rollout-legacy-subagent.jsonl",
                "/tmp/project-a",
                recent - 30,
                recent - 20,
                0,
            ),
        ],
    );
    let connection = Connection::open(codex_home.join("state_5.sqlite")).unwrap();
    for session_id in [
        "current-legacy-session",
        "old-legacy-session",
        "stale-legacy-session",
        "legacy-subagent-session",
    ] {
        connection
            .execute(
                "UPDATE threads SET thread_source = NULL WHERE id = ?1",
                rusqlite::params![session_id],
            )
            .unwrap();
    }
    connection
        .execute(
            "UPDATE threads SET source = ?1 WHERE id = ?2",
            rusqlite::params![
                r#"{"subagent":{"thread_spawn":{"parent_thread_id":"parent"}}}"#,
                "legacy-subagent-session"
            ],
        )
        .unwrap();
    write_session_name_index(
        &codex_home,
        &[
            ("current-legacy-session", "Current sidebar title"),
            ("old-legacy-session", "Old sidebar title"),
            ("stale-legacy-session", "Stale sidebar title"),
            ("legacy-subagent-session", "Subagent sidebar title"),
        ],
    );

    let sessions = list_sessions(&codex_home).unwrap();
    let session_ids = sessions
        .iter()
        .map(|session| session.id.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        session_ids,
        vec!["current-legacy-session", "old-legacy-session"]
    );
    assert_eq!(sessions[0].title, "Current sidebar title");
}

#[test]
fn group_sessions_uses_updated_time_when_no_codex_activity_exists() {
    let temp = tempfile::tempdir().unwrap();
    let codex_home = temp.path().join("codex");
    write_thread_index(
        &codex_home,
        &[
            (
                "recent-sqlite",
                "Recent sqlite title",
                "2026/06/06/rollout-recent.jsonl",
                "/tmp/project-a",
                1_780_707_600,
                1_780_708_000,
                0,
            ),
            (
                "active-sidebar",
                "Active sqlite title",
                "2026/06/06/rollout-active.jsonl",
                "/tmp/project-a",
                1_780_707_600,
                1_780_707_900,
                0,
            ),
        ],
    );
    write_session_name_index(
        &codex_home,
        &[
            ("recent-sqlite", "最近更新时间"),
            ("active-sidebar", "当前侧边栏会话"),
        ],
    );

    let sessions = list_sessions(&codex_home).unwrap();
    let projects = group_sessions_by_project(&sessions);

    assert_eq!(projects[0].sessions[0].id, "recent-sqlite");
    assert_eq!(projects[0].sessions[1].id, "active-sidebar");
}

#[test]
fn group_sessions_prioritizes_codex_activity_before_plain_updated_time() {
    let temp = tempfile::tempdir().unwrap();
    let codex_home = temp.path().join("codex");
    write_thread_index(
        &codex_home,
        &[
            (
                "recent-sqlite",
                "Recent sqlite title",
                "2026/06/06/rollout-recent.jsonl",
                "/tmp/project-a",
                1_780_707_600,
                1_780_708_000,
                0,
            ),
            (
                "active-process",
                "Active process title",
                "2026/06/06/rollout-active.jsonl",
                "/tmp/project-a",
                1_780_707_600,
                1_780_707_900,
                0,
            ),
        ],
    );
    write_process_activity_index(&codex_home, &[("active-process", 1_780_709_000_000)]);

    let sessions = list_sessions(&codex_home).unwrap();
    let projects = group_sessions_by_project(&sessions);

    assert_eq!(projects[0].sessions[0].id, "active-process");
    assert_eq!(projects[0].sessions[1].id, "recent-sqlite");
}

#[test]
fn group_sessions_merges_projects_with_equivalent_paths() {
    let temp = tempfile::tempdir().unwrap();
    let codex_home = temp.path().join("codex");
    write_thread_index(
        &codex_home,
        &[
            (
                "project-normal",
                "Normalized path session",
                "2026/06/06/rollout-normalized.jsonl",
                "/tmp/project-a",
                1_780_707_600,
                1_780_708_000,
                0,
            ),
            (
                "project-trailing",
                "Trailing slash session",
                "2026/06/06/rollout-trailing.jsonl",
                "/tmp/project-a/",
                1_780_707_800,
                1_780_708_200,
                0,
            ),
        ],
    );

    let sessions = list_sessions(&codex_home).unwrap();
    let projects = group_sessions_by_project(&sessions);

    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].project_path, "/tmp/project-a");
    assert_eq!(projects[0].sessions.len(), 2);
    assert_eq!(projects[0].sessions[0].id, "project-trailing");
    assert_eq!(projects[0].sessions[1].id, "project-normal");
}

#[test]
fn group_sessions_orders_active_codex_entries_by_sidebar_index() {
    let temp = tempfile::tempdir().unwrap();
    let codex_home = temp.path().join("codex");
    write_thread_index(
        &codex_home,
        &[
            (
                "regular-recent",
                "Regular recent title",
                "2026/06/06/rollout-regular.jsonl",
                "/tmp/project-a",
                1_780_707_600,
                1_780_708_200,
                0,
            ),
            (
                "active-old-index",
                "Active old index title",
                "2026/06/06/rollout-active-old-index.jsonl",
                "/tmp/project-a",
                1_780_707_600,
                1_780_708_100,
                0,
            ),
            (
                "active-new-index",
                "Active new index title",
                "2026/06/06/rollout-active-new-index.jsonl",
                "/tmp/project-a",
                1_780_707_600,
                1_780_708_000,
                0,
            ),
        ],
    );
    write_session_name_index(
        &codex_home,
        &[
            ("active-old-index", "较早活动会话"),
            ("active-new-index", "较新活动会话"),
            ("regular-recent", "普通最近会话"),
        ],
    );
    write_process_activity_index(
        &codex_home,
        &[
            ("active-old-index", 1_780_709_500_000),
            ("active-new-index", 1_780_709_000_000),
        ],
    );

    let sessions = list_sessions(&codex_home).unwrap();
    let projects = group_sessions_by_project(&sessions);

    assert_eq!(projects[0].sessions[0].id, "active-new-index");
    assert_eq!(projects[0].sessions[1].id, "active-old-index");
    assert_eq!(projects[0].sessions[2].id, "regular-recent");
}

#[test]
fn list_sessions_groups_indexed_sessions_by_project() {
    let temp = tempfile::tempdir().unwrap();
    let codex_home = temp.path().join("codex");
    write_thread_index(
        &codex_home,
        &[
            (
                "project-a-old",
                "Project A older session",
                "2026/06/06/rollout-a-old.jsonl",
                "/tmp/project-a",
                1_780_707_600,
                1_780_707_700,
                0,
            ),
            (
                "project-b-new",
                "Project B newest session",
                "2026/06/06/rollout-b-new.jsonl",
                "/tmp/project-b",
                1_780_707_800,
                1_780_708_100,
                0,
            ),
            (
                "project-a-new",
                "Project A newer session",
                "2026/06/06/rollout-a-new.jsonl",
                "/tmp/project-a",
                1_780_707_900,
                1_780_708_000,
                0,
            ),
        ],
    );

    let sessions = list_sessions(&codex_home).unwrap();
    let projects = group_sessions_by_project(&sessions);

    assert_eq!(sessions.len(), 3);
    assert_eq!(projects.len(), 2);
    assert_eq!(projects[0].project_name, "project-b");
    assert_eq!(projects[0].project_path, "/tmp/project-b");
    assert_eq!(projects[0].sessions[0].id, "project-b-new");
    assert_eq!(projects[1].project_name, "project-a");
    assert_eq!(projects[1].sessions.len(), 2);
    assert_eq!(projects[1].sessions[0].id, "project-a-new");
    assert_eq!(projects[1].sessions[1].id, "project-a-old");
}

#[test]
fn group_sessions_can_follow_codex_project_order() {
    let temp = tempfile::tempdir().unwrap();
    let codex_home = temp.path().join("codex");
    write_thread_index(
        &codex_home,
        &[
            (
                "project-cp",
                "CP recent session",
                "2026/06/06/rollout-cp.jsonl",
                "/tmp/cp",
                1_780_707_600,
                1_780_708_300,
                0,
            ),
            (
                "project-design",
                "Product Design session",
                "2026/06/06/rollout-product-design.jsonl",
                "/tmp/Product Design",
                1_780_707_600,
                1_780_708_000,
                0,
            ),
            (
                "project-api",
                "New API session",
                "2026/06/06/rollout-new-api.jsonl",
                "/tmp/new-api",
                1_780_707_600,
                1_780_708_100,
                0,
            ),
        ],
    );
    write_project_order(
        &codex_home,
        &["/tmp/Product Design", "/tmp/new-api", "/tmp/cp"],
    );

    let sessions = list_sessions(&codex_home).unwrap();
    let projects = group_sessions_by_project_with_codex_order(&codex_home, &sessions);

    assert_eq!(projects[0].project_path, "/tmp/Product Design");
    assert_eq!(projects[1].project_path, "/tmp/new-api");
    assert_eq!(projects[2].project_path, "/tmp/cp");
}

#[test]
fn group_sessions_keeps_codex_saved_workspace_roots_without_sessions() {
    let temp = tempfile::tempdir().unwrap();
    let codex_home = temp.path().join("codex");
    write_thread_index(
        &codex_home,
        &[(
            "project-cp",
            "CP session",
            "2026/06/06/rollout-cp.jsonl",
            "/tmp/cp",
            1_780_707_600,
            1_780_708_300,
            0,
        )],
    );
    write_saved_workspace_roots(&codex_home, &["/tmp/empty-project", "/tmp/cp"]);

    let sessions = list_sessions(&codex_home).unwrap();
    let projects = group_sessions_by_project_with_codex_order(&codex_home, &sessions);

    assert_eq!(projects.len(), 2);
    assert_eq!(projects[0].project_path, "/tmp/empty-project");
    assert_eq!(projects[0].project_name, "empty-project");
    assert!(projects[0].sessions.is_empty());
    assert_eq!(projects[1].project_path, "/tmp/cp");
    assert_eq!(projects[1].sessions.len(), 1);
}

#[test]
fn delete_project_sessions_can_remove_empty_saved_workspace_root() {
    let temp = tempfile::tempdir().unwrap();
    let codex_home = temp.path().join("codex");
    let backup_root = codex_home.join("backups").join("sessions");
    write_saved_workspace_roots_with_labels(
        &codex_home,
        &["/tmp/empty-project", "/tmp/cp"],
        &[("/tmp/empty-project", "Empty Project"), ("/tmp/cp", "CP")],
    );

    assert!(
        delete_project_sessions(&codex_home, &backup_root, "/tmp/empty-project", false).is_err()
    );

    let result =
        delete_project_sessions(&codex_home, &backup_root, "/tmp/empty-project", true).unwrap();

    assert_eq!(result.deleted_count, 0);
    assert!(result.backup_paths.is_empty());
    let projects = group_sessions_by_project_with_codex_order(&codex_home, &[]);
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].project_path, "/tmp/cp");

    let state = fs::read_to_string(codex_home.join(".codex-global-state.json")).unwrap();
    let state: serde_json::Value = serde_json::from_str(&state).unwrap();
    assert_eq!(
        state
            .get("unrelated-setting")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        state
            .get("electron-workspace-root-labels")
            .and_then(serde_json::Value::as_object)
            .unwrap()
            .get("/tmp/empty-project"),
        None
    );

    let deleted_entries = list_deleted_session_entries(&backup_root).unwrap();
    assert_eq!(deleted_entries.len(), 1);
    assert_eq!(deleted_entries[0].kind, "project");
    assert_eq!(deleted_entries[0].sessions.len(), 0);

    let restored =
        restore_deleted_sessions(&codex_home, &backup_root, &deleted_entries[0].id, true).unwrap();

    assert_eq!(restored.restored_count, 0);
    let projects = group_sessions_by_project_with_codex_order(&codex_home, &[]);
    assert_eq!(projects.len(), 2);
    assert_eq!(projects[0].project_path, "/tmp/cp");
    assert_eq!(projects[1].project_path, "/tmp/empty-project");
    assert_eq!(projects[1].project_name, "Empty Project");
}

#[test]
fn visible_project_groups_hide_deleted_project_even_if_codex_rewrites_global_state() {
    let temp = tempfile::tempdir().unwrap();
    let codex_home = temp.path().join("codex");
    let backup_root = codex_home.join("backups").join("sessions");
    write_saved_workspace_roots_with_labels(
        &codex_home,
        &["/tmp/empty-project", "/tmp/cp"],
        &[("/tmp/empty-project", "Empty Project"), ("/tmp/cp", "CP")],
    );

    delete_project_sessions(&codex_home, &backup_root, "/tmp/empty-project", true).unwrap();
    write_saved_workspace_roots_with_labels(
        &codex_home,
        &["/tmp/empty-project", "/tmp/cp"],
        &[("/tmp/empty-project", "Empty Project"), ("/tmp/cp", "CP")],
    );

    let projects =
        group_visible_sessions_by_project_with_codex_order(&codex_home, &backup_root, &[]).unwrap();

    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].project_path, "/tmp/cp");
}

#[test]
fn list_sessions_uses_thread_index_without_touching_session_file() {
    let temp = tempfile::tempdir().unwrap();
    let codex_home = temp.path().join("codex");
    let relative = "2026/06/06/rollout-2026-06-06T09-00-00-indexed-only.jsonl";
    write_thread_index(
        &codex_home,
        &[(
            "indexed-session",
            "Indexed title without body scan",
            relative,
            "/tmp/project-a",
            1_780_707_600,
            1_780_707_700,
            0,
        )],
    );

    let sessions = list_sessions(&codex_home).unwrap();

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, "indexed-session");
    assert_eq!(sessions[0].title, "Indexed title without body scan");
    assert_eq!(sessions[0].file_size_bytes, 0);
}

#[test]
fn preview_session_returns_message_entries_without_tool_noise() {
    let temp = tempfile::tempdir().unwrap();
    let codex_home = temp.path().join("codex");
    write_session(
        &codex_home,
        "2026/06/06/rollout-2026-06-06T09-00-00-session-b.jsonl",
        r#"{"timestamp":"2026-06-06T01:00:00Z","type":"session_meta","payload":{"id":"session-b","timestamp":"2026-06-06T01:00:00Z"}}
{"timestamp":"2026-06-06T01:00:01Z","type":"event_msg","payload":{"type":"user_message","message":"First message"}}
{"timestamp":"2026-06-06T01:00:02Z","type":"response_item","payload":{"type":"function_call","name":"exec_command"}}
{"timestamp":"2026-06-06T01:00:03Z","type":"event_msg","payload":{"type":"agent_message","message":"Second message"}}
"#,
    );

    let preview = preview_session(&codex_home, "session-b").unwrap();

    assert_eq!(preview.summary.id, "session-b");
    assert_eq!(preview.entries.len(), 2);
    assert_eq!(preview.entries[0].role, "user");
    assert_eq!(preview.entries[0].text, "First message");
    assert_eq!(preview.entries[1].role, "assistant");
    assert_eq!(preview.entries[1].text, "Second message");
}

#[test]
fn copy_session_to_other_codex_home_preserves_relative_path() {
    let temp = tempfile::tempdir().unwrap();
    let source_home = temp.path().join("source");
    let target_home = temp.path().join("target");
    let relative = "2026/06/06/rollout-2026-06-06T09-00-00-session-c.jsonl";
    write_session(
        &source_home,
        relative,
        r#"{"timestamp":"2026-06-06T01:00:00Z","type":"session_meta","payload":{"id":"session-c"}}"#,
    );

    let result = copy_session_to_codex_home(&source_home, &target_home, "session-c").unwrap();

    assert_eq!(
        result.target_path,
        target_home.join("sessions").join(relative)
    );
    assert_eq!(
        fs::read_to_string(target_home.join("sessions").join(relative)).unwrap(),
        r#"{"timestamp":"2026-06-06T01:00:00Z","type":"session_meta","payload":{"id":"session-c"}}"#
    );
}

#[test]
fn delete_session_requires_confirmation_then_backs_up_before_removing() {
    let temp = tempfile::tempdir().unwrap();
    let codex_home = temp.path().join("codex");
    let backup_root = codex_home.join("backups").join("sessions");
    let relative = "2026/06/06/rollout-2026-06-06T09-00-00-session-d.jsonl";
    let session_path = write_session(
        &codex_home,
        relative,
        r#"{"timestamp":"2026-06-06T01:00:00Z","type":"session_meta","payload":{"id":"session-d"}}"#,
    );

    assert!(delete_session(&codex_home, &backup_root, "session-d", false).is_err());
    assert!(session_path.exists());

    let result = delete_session(&codex_home, &backup_root, "session-d", true).unwrap();

    assert!(!session_path.exists());
    assert!(result.backup_path.starts_with(&backup_root));
    assert_eq!(
        fs::read_to_string(result.backup_path).unwrap(),
        r#"{"timestamp":"2026-06-06T01:00:00Z","type":"session_meta","payload":{"id":"session-d"}}"#
    );
}

#[test]
fn delete_project_sessions_requires_confirmation_then_backs_up_every_session() {
    let temp = tempfile::tempdir().unwrap();
    let codex_home = temp.path().join("codex");
    let backup_root = codex_home.join("backups").join("sessions");
    let project_a_one = "2026/06/06/rollout-project-a-one.jsonl";
    let project_a_two = "2026/06/06/rollout-project-a-two.jsonl";
    let project_b = "2026/06/06/rollout-project-b.jsonl";
    let project_a_one_path = write_session(
        &codex_home,
        project_a_one,
        r#"{"timestamp":"2026-06-06T01:00:00Z","type":"session_meta","payload":{"id":"project-a-one","cwd":"/tmp/project-a"}}"#,
    );
    let project_a_two_path = write_session(
        &codex_home,
        project_a_two,
        r#"{"timestamp":"2026-06-06T01:00:00Z","type":"session_meta","payload":{"id":"project-a-two","cwd":"/tmp/project-a"}}"#,
    );
    let project_b_path = write_session(
        &codex_home,
        project_b,
        r#"{"timestamp":"2026-06-06T01:00:00Z","type":"session_meta","payload":{"id":"project-b","cwd":"/tmp/project-b"}}"#,
    );
    write_thread_index(
        &codex_home,
        &[
            (
                "project-a-one",
                "Project A one",
                project_a_one,
                "/tmp/project-a",
                1_780_707_600,
                1_780_707_900,
                0,
            ),
            (
                "project-a-two",
                "Project A two",
                project_a_two,
                "/tmp/project-a/",
                1_780_707_600,
                1_780_708_000,
                0,
            ),
            (
                "project-b",
                "Project B",
                project_b,
                "/tmp/project-b",
                1_780_707_600,
                1_780_708_100,
                0,
            ),
        ],
    );

    assert!(delete_project_sessions(&codex_home, &backup_root, "/tmp/project-a", false).is_err());
    assert!(project_a_one_path.exists());
    assert!(project_a_two_path.exists());

    let result =
        delete_project_sessions(&codex_home, &backup_root, "/tmp/project-a", true).unwrap();

    assert_eq!(result.deleted_count, 2);
    assert_eq!(result.backup_paths.len(), 2);
    assert!(!project_a_one_path.exists());
    assert!(!project_a_two_path.exists());
    assert!(project_b_path.exists());
    assert!(
        result
            .backup_paths
            .iter()
            .all(|path| path.starts_with(&backup_root))
    );
    let sessions = list_sessions(&codex_home).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, "project-b");

    let deleted_entries = list_deleted_session_entries(&backup_root).unwrap();
    assert_eq!(deleted_entries.len(), 1);
    assert_eq!(deleted_entries[0].kind, "project");
    assert_eq!(deleted_entries[0].sessions.len(), 2);

    let restored =
        restore_deleted_sessions(&codex_home, &backup_root, &deleted_entries[0].id, true).unwrap();

    assert_eq!(restored.restored_count, 2);
    assert!(project_a_one_path.exists());
    assert!(project_a_two_path.exists());
    let sessions = list_sessions(&codex_home).unwrap();
    assert_eq!(sessions.len(), 3);
    assert!(
        list_deleted_session_entries(&backup_root)
            .unwrap()
            .is_empty()
    );
}
