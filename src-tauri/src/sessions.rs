use std::{
    cmp::Ordering,
    collections::HashMap,
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const MAX_PREVIEW_ENTRIES: usize = 18;
const LEGACY_OPENAI_THREAD_MAX_AGE_SECONDS: i64 = 30 * 24 * 60 * 60;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionSummary {
    pub id: String,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub activity_rank: i64,
    pub sidebar_order: i64,
    pub message_count: usize,
    pub file_size_bytes: u64,
    pub relative_path: String,
    pub project_path: String,
    pub project_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionProjectGroup {
    pub project_name: String,
    pub project_path: String,
    pub updated_at: i64,
    pub sessions: Vec<CodexSessionSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionPreviewEntry {
    pub role: String,
    pub text: String,
    pub timestamp: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionPreview {
    pub summary: CodexSessionSummary,
    pub entries: Vec<CodexSessionPreviewEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionList {
    pub sessions: Vec<CodexSessionSummary>,
    pub projects: Vec<CodexSessionProjectGroup>,
    pub codex_running: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeleteSessionResult {
    pub backup_path: PathBuf,
    pub codex_running: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeleteProjectSessionsResult {
    pub backup_paths: Vec<PathBuf>,
    pub deleted_count: usize,
    pub codex_running: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeletedSessionBackup {
    pub id: String,
    pub title: String,
    pub relative_path: String,
    pub backup_path: PathBuf,
    pub project_path: String,
    pub project_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeletedSessionEntry {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub project_path: String,
    pub project_name: String,
    pub deleted_at: i64,
    pub restored_at: Option<i64>,
    pub sessions: Vec<DeletedSessionBackup>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RestoreDeletedSessionsResult {
    pub restored_paths: Vec<PathBuf>,
    pub restored_count: usize,
    pub codex_running: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CopySessionResult {
    pub target_path: PathBuf,
    pub codex_running: bool,
}

#[derive(Debug, Clone)]
struct SessionRecord {
    summary: CodexSessionSummary,
    absolute_path: PathBuf,
    relative_path: PathBuf,
    sort_updated_at_ms: i64,
}

#[derive(Debug, Clone)]
struct ThreadIndexRecord {
    id: String,
    title: String,
    created_at: i64,
    updated_at: i64,
    updated_at_ms: i64,
    thread_source: Option<String>,
    model_provider: String,
    source: String,
    activity_rank: i64,
    sidebar_order: i64,
    rollout_path: PathBuf,
    project_path: String,
}

#[derive(Debug, Clone)]
struct SidebarTitleRecord {
    title: String,
    order: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProjectWorkspaceRemoval {
    removed: bool,
    project_name: String,
}

pub fn default_session_backup_root(codex_home: &Path) -> PathBuf {
    codex_home.join("backups").join("sessions")
}

pub fn list_sessions(codex_home: &Path) -> anyhow::Result<Vec<CodexSessionSummary>> {
    let mut records = load_session_records(codex_home)?;
    sort_session_records(&mut records);
    Ok(records.into_iter().map(|record| record.summary).collect())
}

pub fn group_sessions_by_project(
    sessions: &[CodexSessionSummary],
) -> Vec<CodexSessionProjectGroup> {
    let mut projects: Vec<CodexSessionProjectGroup> = Vec::new();
    for session in sessions {
        let project_path = normalize_project_path(&session.project_path);
        if let Some(project) = projects
            .iter_mut()
            .find(|project| normalize_project_path(&project.project_path) == project_path)
        {
            project.updated_at = project.updated_at.max(session.updated_at);
            project.sessions.push(session.clone());
        } else {
            projects.push(CodexSessionProjectGroup {
                project_name: session.project_name.clone(),
                project_path,
                updated_at: session.updated_at,
                sessions: vec![session.clone()],
            });
        }
    }
    for project in &mut projects {
        project.sessions = sorted_project_sessions(&project.sessions);
    }
    projects.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.project_name.cmp(&right.project_name))
    });
    projects
}

pub fn group_sessions_by_project_with_codex_order(
    codex_home: &Path,
    sessions: &[CodexSessionSummary],
) -> Vec<CodexSessionProjectGroup> {
    let mut projects = group_sessions_by_project(sessions);
    let project_roots = read_saved_project_roots(codex_home).unwrap_or_default();
    if project_roots.is_empty() {
        return projects;
    }
    let project_labels = read_project_labels(codex_home).unwrap_or_default();
    let project_order = project_roots
        .iter()
        .enumerate()
        .map(|(index, project_path)| (normalize_project_path(project_path), index))
        .collect::<HashMap<_, _>>();
    projects.retain(|project| {
        project_order.contains_key(&normalize_project_path(&project.project_path))
    });
    for project_path in &project_roots {
        let project_path = normalize_project_path(project_path);
        if project_path.is_empty() {
            continue;
        }
        if projects
            .iter()
            .any(|project| normalize_project_path(&project.project_path) == project_path)
        {
            continue;
        }
        let project_name = project_labels
            .get(&project_path)
            .cloned()
            .unwrap_or_else(|| project_name_from_path(&project_path));
        projects.push(CodexSessionProjectGroup {
            project_name,
            project_path,
            updated_at: 0,
            sessions: Vec::new(),
        });
    }
    projects.sort_by(|left, right| {
        let left_order = project_order.get(&normalize_project_path(&left.project_path));
        let right_order = project_order.get(&normalize_project_path(&right.project_path));
        match (left_order, right_order) {
            (Some(left_order), Some(right_order)) => left_order
                .cmp(right_order)
                .then_with(|| left.project_name.cmp(&right.project_name)),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| left.project_name.cmp(&right.project_name)),
        }
    });
    projects
}

pub fn group_visible_sessions_by_project_with_codex_order(
    codex_home: &Path,
    backup_root: &Path,
    sessions: &[CodexSessionSummary],
) -> anyhow::Result<Vec<CodexSessionProjectGroup>> {
    let hidden_projects = active_deleted_project_paths(backup_root)?;
    let visible_sessions = sessions
        .iter()
        .filter(|session| !hidden_projects.contains(&normalize_project_path(&session.project_path)))
        .cloned()
        .collect::<Vec<_>>();
    let mut projects = group_sessions_by_project_with_codex_order(codex_home, &visible_sessions);
    projects.retain(|project| {
        !hidden_projects.contains(&normalize_project_path(&project.project_path))
    });
    Ok(projects)
}

fn sort_session_records(records: &mut [SessionRecord]) {
    records.sort_by(compare_session_records);
}

fn sorted_project_sessions(sessions: &[CodexSessionSummary]) -> Vec<CodexSessionSummary> {
    let mut sessions = sessions.to_vec();
    sessions.sort_by(compare_session_summaries);
    sessions
}

fn compare_session_records(left: &SessionRecord, right: &SessionRecord) -> Ordering {
    compare_session_ordering(
        &left.summary,
        left.sort_updated_at_ms,
        &right.summary,
        right.sort_updated_at_ms,
    )
}

fn compare_session_summaries(left: &CodexSessionSummary, right: &CodexSessionSummary) -> Ordering {
    compare_session_ordering(left, left.updated_at * 1000, right, right.updated_at * 1000)
}

fn compare_session_ordering(
    left: &CodexSessionSummary,
    left_updated_at_ms: i64,
    right: &CodexSessionSummary,
    right_updated_at_ms: i64,
) -> Ordering {
    let left_active = left.activity_rank > 0;
    let right_active = right.activity_rank > 0;
    let active_order = right_active.cmp(&left_active);
    if active_order != Ordering::Equal {
        return active_order;
    }
    if left_active && right_active {
        let sidebar_order = right.sidebar_order.cmp(&left.sidebar_order);
        if sidebar_order != Ordering::Equal {
            return sidebar_order;
        }
        let activity_order = right.activity_rank.cmp(&left.activity_rank);
        if activity_order != Ordering::Equal {
            return activity_order;
        }
    }
    right_updated_at_ms
        .cmp(&left_updated_at_ms)
        .then_with(|| right.sidebar_order.cmp(&left.sidebar_order))
        .then_with(|| left.title.cmp(&right.title))
}

pub fn preview_session(codex_home: &Path, session_id: &str) -> anyhow::Result<CodexSessionPreview> {
    let record = find_session_record(codex_home, session_id)?;
    let entries = read_preview_entries(&record.absolute_path)?;
    Ok(CodexSessionPreview {
        summary: record.summary,
        entries,
    })
}

pub fn copy_session_to_codex_home(
    source_codex_home: &Path,
    target_codex_home: &Path,
    session_id: &str,
) -> anyhow::Result<CopySessionResult> {
    let record = find_session_record(source_codex_home, session_id)?;
    let target_path = target_codex_home
        .join("sessions")
        .join(&record.relative_path);
    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent)?;
    }
    if !target_path.exists() {
        fs::copy(&record.absolute_path, &target_path)?;
    }
    Ok(CopySessionResult {
        target_path,
        codex_running: false,
    })
}

pub fn delete_session(
    codex_home: &Path,
    backup_root: &Path,
    session_id: &str,
    confirmed: bool,
) -> anyhow::Result<DeleteSessionResult> {
    if !confirmed {
        return Err(anyhow::anyhow!(
            "session deletion requires explicit confirmation"
        ));
    }
    let record = find_session_record(codex_home, session_id)?;
    let backup_folder = timestamp_folder();
    let backup_path = backup_and_remove_record(backup_root, &backup_folder, &record)?;
    append_deleted_session_entry(
        backup_root,
        DeletedSessionEntry {
            id: uuid::Uuid::new_v4().to_string(),
            kind: "session".to_string(),
            name: record.summary.title.clone(),
            project_path: record.summary.project_path.clone(),
            project_name: record.summary.project_name.clone(),
            deleted_at: current_unix_timestamp(),
            restored_at: None,
            sessions: vec![deleted_backup_from_record(&record, backup_path.clone())],
        },
    )?;
    archive_thread_index_records(codex_home, &[record.summary.id.clone()])?;
    Ok(DeleteSessionResult {
        backup_path,
        codex_running: false,
    })
}

pub fn delete_project_sessions(
    codex_home: &Path,
    backup_root: &Path,
    project_path: &str,
    confirmed: bool,
) -> anyhow::Result<DeleteProjectSessionsResult> {
    if !confirmed {
        return Err(anyhow::anyhow!(
            "project deletion requires explicit confirmation"
        ));
    }
    let project_path = normalize_project_path(project_path);
    if project_path.is_empty() {
        return Err(anyhow::anyhow!("project path is required"));
    }
    let records = load_session_records(codex_home)?
        .into_iter()
        .filter(|record| normalize_project_path(&record.summary.project_path) == project_path)
        .collect::<Vec<_>>();
    if records.is_empty() {
        let removal = remove_project_from_saved_workspaces(codex_home, &project_path)?;
        if !removal.removed {
            return Err(anyhow::anyhow!("project `{project_path}` has no sessions"));
        }
        append_deleted_session_entry(
            backup_root,
            DeletedSessionEntry {
                id: uuid::Uuid::new_v4().to_string(),
                kind: "project".to_string(),
                name: removal.project_name.clone(),
                project_path: project_path.clone(),
                project_name: removal.project_name,
                deleted_at: current_unix_timestamp(),
                restored_at: None,
                sessions: Vec::new(),
            },
        )?;
        return Ok(DeleteProjectSessionsResult {
            deleted_count: 0,
            backup_paths: Vec::new(),
            codex_running: false,
        });
    }

    let backup_folder = timestamp_folder();
    let project_name = records
        .first()
        .map(|record| record.summary.project_name.clone())
        .unwrap_or_else(|| project_name_from_path(&project_path));
    let mut backup_paths = Vec::with_capacity(records.len());
    let mut deleted_backups = Vec::with_capacity(records.len());
    let mut deleted_ids = Vec::with_capacity(records.len());
    for record in records {
        let backup_path = backup_and_remove_record(backup_root, &backup_folder, &record)?;
        backup_paths.push(backup_path.clone());
        deleted_backups.push(deleted_backup_from_record(&record, backup_path));
        deleted_ids.push(record.summary.id);
    }
    let removal = remove_project_from_saved_workspaces(codex_home, &project_path)?;
    let project_name = if removal.removed {
        removal.project_name
    } else {
        project_name
    };
    append_deleted_session_entry(
        backup_root,
        DeletedSessionEntry {
            id: uuid::Uuid::new_v4().to_string(),
            kind: "project".to_string(),
            name: project_name.clone(),
            project_path: project_path.clone(),
            project_name,
            deleted_at: current_unix_timestamp(),
            restored_at: None,
            sessions: deleted_backups,
        },
    )?;
    archive_thread_index_records(codex_home, &deleted_ids)?;

    Ok(DeleteProjectSessionsResult {
        deleted_count: backup_paths.len(),
        backup_paths,
        codex_running: false,
    })
}

pub fn list_deleted_session_entries(
    backup_root: &Path,
) -> anyhow::Result<Vec<DeletedSessionEntry>> {
    let manifest_path = deleted_sessions_manifest_path(backup_root);
    if !manifest_path.exists() {
        return Ok(Vec::new());
    }
    let file = fs::File::open(manifest_path)?;
    let mut entries = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<DeletedSessionEntry>(&line) else {
            continue;
        };
        if entry.restored_at.is_none() {
            entries.push(entry);
        }
    }
    entries.sort_by(|left, right| {
        right
            .deleted_at
            .cmp(&left.deleted_at)
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(entries)
}

pub fn restore_deleted_sessions(
    codex_home: &Path,
    backup_root: &Path,
    deletion_id: &str,
    confirmed: bool,
) -> anyhow::Result<RestoreDeletedSessionsResult> {
    if !confirmed {
        return Err(anyhow::anyhow!(
            "session restore requires explicit confirmation"
        ));
    }
    let deletion_id = deletion_id.trim();
    if deletion_id.is_empty() {
        return Err(anyhow::anyhow!("deletion id is required"));
    }

    let mut entries = read_all_deleted_session_entries(backup_root)?;
    let Some(entry_index) = entries
        .iter()
        .position(|entry| entry.id == deletion_id && entry.restored_at.is_none())
    else {
        return Err(anyhow::anyhow!(
            "deleted session entry `{deletion_id}` does not exist"
        ));
    };
    let entry = entries[entry_index].clone();
    let sessions_root = codex_home.join("sessions");

    for session in &entry.sessions {
        if !session.backup_path.exists() {
            return Err(anyhow::anyhow!(
                "backup file `{}` does not exist",
                session.backup_path.display()
            ));
        }
        let target_path = sessions_root.join(&session.relative_path);
        if target_path.exists() {
            return Err(anyhow::anyhow!(
                "target session `{}` already exists",
                target_path.display()
            ));
        }
    }

    let mut restored_paths = Vec::with_capacity(entry.sessions.len());
    for session in &entry.sessions {
        let target_path = sessions_root.join(&session.relative_path);
        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&session.backup_path, &target_path)?;
        restored_paths.push(target_path);
    }
    if entry.kind == "project" && !normalize_project_path(&entry.project_path).is_empty() {
        add_project_to_saved_workspaces(codex_home, &entry.project_path, &entry.project_name)?;
    }
    let restored_ids = entry
        .sessions
        .iter()
        .map(|session| session.id.clone())
        .collect::<Vec<_>>();
    set_thread_index_archived(codex_home, &restored_ids, false)?;

    entries[entry_index].restored_at = Some(current_unix_timestamp());
    write_deleted_session_entries(backup_root, &entries)?;

    Ok(RestoreDeletedSessionsResult {
        restored_count: restored_paths.len(),
        restored_paths,
        codex_running: false,
    })
}

pub fn is_codex_running() -> bool {
    is_named_process_running(&["Codex", "codex"])
}

fn backup_and_remove_record(
    backup_root: &Path,
    backup_folder: &str,
    record: &SessionRecord,
) -> anyhow::Result<PathBuf> {
    let backup_path = backup_root.join(backup_folder).join(&record.relative_path);
    if let Some(parent) = backup_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(&record.absolute_path, &backup_path)?;
    fs::remove_file(&record.absolute_path)?;
    Ok(backup_path)
}

fn deleted_backup_from_record(
    record: &SessionRecord,
    backup_path: PathBuf,
) -> DeletedSessionBackup {
    DeletedSessionBackup {
        id: record.summary.id.clone(),
        title: record.summary.title.clone(),
        relative_path: record.summary.relative_path.clone(),
        backup_path,
        project_path: record.summary.project_path.clone(),
        project_name: record.summary.project_name.clone(),
    }
}

fn deleted_sessions_manifest_path(backup_root: &Path) -> PathBuf {
    backup_root.join("deletions.jsonl")
}

fn append_deleted_session_entry(
    backup_root: &Path,
    entry: DeletedSessionEntry,
) -> anyhow::Result<()> {
    fs::create_dir_all(backup_root)?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(deleted_sessions_manifest_path(backup_root))?;
    writeln!(file, "{}", serde_json::to_string(&entry)?)?;
    Ok(())
}

fn read_all_deleted_session_entries(
    backup_root: &Path,
) -> anyhow::Result<Vec<DeletedSessionEntry>> {
    let manifest_path = deleted_sessions_manifest_path(backup_root);
    if !manifest_path.exists() {
        return Ok(Vec::new());
    }
    let file = fs::File::open(manifest_path)?;
    let mut entries = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(entry) = serde_json::from_str::<DeletedSessionEntry>(&line) {
            entries.push(entry);
        }
    }
    Ok(entries)
}

fn write_deleted_session_entries(
    backup_root: &Path,
    entries: &[DeletedSessionEntry],
) -> anyhow::Result<()> {
    fs::create_dir_all(backup_root)?;
    let body = entries
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()?
        .join("\n");
    fs::write(
        deleted_sessions_manifest_path(backup_root),
        if body.is_empty() {
            String::new()
        } else {
            format!("{body}\n")
        },
    )?;
    Ok(())
}

fn active_deleted_project_paths(
    backup_root: &Path,
) -> anyhow::Result<std::collections::HashSet<String>> {
    Ok(read_all_deleted_session_entries(backup_root)?
        .into_iter()
        .filter(|entry| entry.kind == "project" && entry.restored_at.is_none())
        .map(|entry| normalize_project_path(&entry.project_path))
        .filter(|project_path| !project_path.is_empty())
        .collect())
}

fn archive_thread_index_records(codex_home: &Path, session_ids: &[String]) -> anyhow::Result<()> {
    set_thread_index_archived(codex_home, session_ids, true)
}

fn set_thread_index_archived(
    codex_home: &Path,
    session_ids: &[String],
    archived: bool,
) -> anyhow::Result<()> {
    if session_ids.is_empty() {
        return Ok(());
    }
    let db_path = codex_home.join("state_5.sqlite");
    if !db_path.exists() {
        return Ok(());
    }
    let mut connection = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.busy_timeout(std::time::Duration::from_millis(350))?;
    let transaction = connection.transaction()?;
    {
        let mut statement =
            transaction.prepare("UPDATE threads SET archived = ?1 WHERE id = ?2")?;
        for session_id in session_ids {
            statement.execute(rusqlite::params![if archived { 1 } else { 0 }, session_id])?;
        }
    }
    transaction.commit()?;
    Ok(())
}

fn load_session_records(codex_home: &Path) -> anyhow::Result<Vec<SessionRecord>> {
    if let Ok(index_records) = read_thread_index(codex_home) {
        if !index_records.is_empty() {
            let indexed_records = load_indexed_session_records(codex_home, index_records)?;
            if !indexed_records.is_empty() {
                return Ok(indexed_records);
            }
        }
    }
    load_raw_session_records(codex_home)
}

fn load_raw_session_records(codex_home: &Path) -> anyhow::Result<Vec<SessionRecord>> {
    let sessions_root = codex_home.join("sessions");
    if !sessions_root.exists() {
        return Ok(Vec::new());
    }
    let mut records = Vec::new();
    collect_session_files(&sessions_root, &mut records)?;
    let mut parsed = Vec::new();
    for path in records {
        if let Some(record) = parse_session_file(&sessions_root, path)? {
            parsed.push(record);
        }
    }
    Ok(parsed)
}

fn load_indexed_session_records(
    codex_home: &Path,
    index_records: Vec<ThreadIndexRecord>,
) -> anyhow::Result<Vec<SessionRecord>> {
    let sessions_root = codex_home.join("sessions");
    let mut indexed_records = Vec::new();

    for index_record in index_records {
        let relative_path = match index_record.rollout_path.strip_prefix(&sessions_root) {
            Ok(path) => path.to_path_buf(),
            Err(_) => continue,
        };
        indexed_records.push(SessionRecord {
            summary: CodexSessionSummary {
                id: index_record.id,
                title: index_record.title,
                created_at: index_record.created_at,
                updated_at: index_record.updated_at,
                activity_rank: index_record.activity_rank,
                sidebar_order: index_record.sidebar_order,
                message_count: 0,
                file_size_bytes: 0,
                relative_path: relative_path.to_string_lossy().into_owned(),
                project_name: project_name_from_path(&index_record.project_path),
                project_path: index_record.project_path,
            },
            absolute_path: index_record.rollout_path,
            relative_path,
            sort_updated_at_ms: index_record.updated_at_ms,
        });
    }

    Ok(indexed_records)
}

fn read_thread_index(codex_home: &Path) -> anyhow::Result<Vec<ThreadIndexRecord>> {
    let db_path = codex_home.join("state_5.sqlite");
    if !db_path.exists() {
        return Ok(Vec::new());
    }
    let sidebar_titles = read_sidebar_title_index(codex_home).unwrap_or_default();
    let has_sidebar_index = !sidebar_titles.is_empty();
    let connection = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.busy_timeout(std::time::Duration::from_millis(350))?;
    let mut statement = connection.prepare(
        r#"
        SELECT
            id,
            title,
            created_at,
            updated_at,
            COALESCE(updated_at_ms, updated_at * 1000),
            rollout_path,
            cwd,
            thread_source,
            model_provider,
            source
        FROM threads
        WHERE archived = 0
          AND rollout_path <> ''
        ORDER BY COALESCE(updated_at_ms, updated_at * 1000) DESC, id DESC
        "#,
    )?;
    let rows = statement.query_map([], |row| {
        let title: String = row.get(1)?;
        let updated_at: i64 = row.get(3)?;
        Ok(ThreadIndexRecord {
            id: row.get(0)?,
            title: if title.trim().is_empty() {
                "Untitled Codex session".to_string()
            } else {
                title
            },
            created_at: row.get(2)?,
            updated_at,
            updated_at_ms: row.get::<_, Option<i64>>(4)?.unwrap_or(updated_at * 1000),
            thread_source: row.get(7)?,
            model_provider: row.get(8)?,
            source: row.get(9)?,
            activity_rank: 0,
            sidebar_order: 0,
            rollout_path: PathBuf::from(row.get::<_, String>(5)?),
            project_path: normalize_project_path(row.get::<_, String>(6)?.as_str()),
        })
    })?;

    let activity_ranks = read_process_activity_index(codex_home).unwrap_or_default();
    let mut records = Vec::new();
    for row in rows {
        let mut record = row?;
        if is_subagent_thread(&record) {
            continue;
        }
        if !is_visible_user_thread(&record) {
            continue;
        }
        if has_sidebar_index {
            if let Some(title) = sidebar_titles.get(&record.id) {
                record.title = title.title.clone();
                record.sidebar_order = title.order;
            }
        }
        if let Some(activity_rank) = activity_ranks.get(&record.id) {
            record.activity_rank = *activity_rank;
        }
        records.push(record);
    }
    Ok(records)
}

fn is_visible_user_thread(record: &ThreadIndexRecord) -> bool {
    if record.model_provider != "openai" {
        return false;
    }
    let explicit_source = record.thread_source.as_deref().map(str::trim);
    if explicit_source == Some("user") {
        return true;
    }
    if record.thread_source.is_some() {
        return false;
    }
    let cutoff = current_unix_timestamp().saturating_sub(LEGACY_OPENAI_THREAD_MAX_AGE_SECONDS);
    record.updated_at >= cutoff
}

fn is_subagent_thread(record: &ThreadIndexRecord) -> bool {
    normalized_thread_source(record).as_deref() == Some("subagent")
        || record.source.contains("\"subagent\"")
        || record.source.contains("thread_spawn")
}

fn normalized_thread_source(record: &ThreadIndexRecord) -> Option<String> {
    record
        .thread_source
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn read_process_activity_index(codex_home: &Path) -> anyhow::Result<HashMap<String, i64>> {
    let process_path = codex_home
        .join("process_manager")
        .join("chat_processes.json");
    if !process_path.exists() {
        return Ok(HashMap::new());
    }
    let text = fs::read_to_string(process_path)?;
    let Value::Array(items) = serde_json::from_str::<Value>(&text)? else {
        return Ok(HashMap::new());
    };
    let mut ranks: HashMap<String, i64> = HashMap::new();
    for item in items {
        let Some(id) = item.get("conversationId").and_then(Value::as_str) else {
            continue;
        };
        let rank = item
            .get("updatedAtMs")
            .and_then(Value::as_i64)
            .or_else(|| item.get("startedAtMs").and_then(Value::as_i64))
            .unwrap_or(0);
        ranks
            .entry(id.to_string())
            .and_modify(|current| *current = (*current).max(rank))
            .or_insert(rank);
    }
    Ok(ranks)
}

fn read_sidebar_title_index(
    codex_home: &Path,
) -> anyhow::Result<HashMap<String, SidebarTitleRecord>> {
    let index_path = codex_home.join("session_index.jsonl");
    if !index_path.exists() {
        return Ok(HashMap::new());
    }
    let file = fs::File::open(index_path)?;
    let mut titles = HashMap::new();
    for (line_index, line) in BufReader::new(file).lines().enumerate() {
        let line = line?;
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(id) = value.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some(title) = value.get("thread_name").and_then(Value::as_str) else {
            continue;
        };
        let title = clean_text(title);
        if !title.is_empty() {
            titles.insert(
                id.to_string(),
                SidebarTitleRecord {
                    title,
                    order: (line_index + 1) as i64,
                },
            );
        }
    }
    Ok(titles)
}

fn read_saved_project_roots(codex_home: &Path) -> anyhow::Result<Vec<String>> {
    let state_path = codex_home.join(".codex-global-state.json");
    if !state_path.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(state_path)?;
    let value = serde_json::from_str::<Value>(&text)?;
    let projects = value
        .get("electron-saved-workspace-roots")
        .and_then(Value::as_array)
        .or_else(|| value.get("project-order").and_then(Value::as_array));
    let Some(projects) = projects else {
        return Ok(Vec::new());
    };
    let mut roots = Vec::new();
    for project in projects {
        let Some(project_path) = project.as_str() else {
            continue;
        };
        let project_path = normalize_project_path(project_path);
        if !project_path.is_empty() {
            roots.push(project_path);
        }
    }
    Ok(roots)
}

fn read_project_labels(codex_home: &Path) -> anyhow::Result<HashMap<String, String>> {
    let state_path = codex_home.join(".codex-global-state.json");
    if !state_path.exists() {
        return Ok(HashMap::new());
    }
    let text = fs::read_to_string(state_path)?;
    let value = serde_json::from_str::<Value>(&text)?;
    let Some(labels) = value
        .get("electron-workspace-root-labels")
        .and_then(Value::as_object)
    else {
        return Ok(HashMap::new());
    };
    Ok(labels
        .iter()
        .filter_map(|(project_path, label)| {
            let label = label.as_str()?.trim();
            if label.is_empty() {
                None
            } else {
                Some((normalize_project_path(project_path), label.to_string()))
            }
        })
        .collect())
}

fn remove_project_from_saved_workspaces(
    codex_home: &Path,
    project_path: &str,
) -> anyhow::Result<ProjectWorkspaceRemoval> {
    let project_path = normalize_project_path(project_path);
    let project_name = read_project_labels(codex_home)
        .ok()
        .and_then(|labels| labels.get(&project_path).cloned())
        .unwrap_or_else(|| project_name_from_path(&project_path));
    let Some(mut state) = read_global_state_object(codex_home)? else {
        return Ok(ProjectWorkspaceRemoval {
            removed: false,
            project_name,
        });
    };

    let mut removed = false;
    for key in [
        "electron-saved-workspace-roots",
        "project-order",
        "active-workspace-roots",
    ] {
        if remove_project_from_array(&mut state, key, &project_path) {
            removed = true;
        }
    }
    if remove_project_label(&mut state, &project_path) {
        removed = true;
    }

    if removed {
        write_global_state_object(codex_home, state)?;
    }
    Ok(ProjectWorkspaceRemoval {
        removed,
        project_name,
    })
}

fn add_project_to_saved_workspaces(
    codex_home: &Path,
    project_path: &str,
    project_name: &str,
) -> anyhow::Result<()> {
    let project_path = normalize_project_path(project_path);
    if project_path.is_empty() {
        return Ok(());
    }
    let mut state = read_global_state_object(codex_home)?.unwrap_or_default();
    for key in [
        "electron-saved-workspace-roots",
        "project-order",
        "active-workspace-roots",
    ] {
        add_project_to_array(&mut state, key, &project_path);
    }
    let labels = state
        .entry("electron-workspace-root-labels".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if let Value::Object(labels) = labels {
        let label = project_name.trim();
        if !label.is_empty() {
            labels.insert(project_path, Value::String(label.to_string()));
        }
    }
    write_global_state_object(codex_home, state)
}

fn read_global_state_object(codex_home: &Path) -> anyhow::Result<Option<Map<String, Value>>> {
    let state_path = codex_home.join(".codex-global-state.json");
    if !state_path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(state_path)?;
    let value = serde_json::from_str::<Value>(&text)?;
    let Value::Object(state) = value else {
        return Err(anyhow::anyhow!("Codex global state is not a JSON object"));
    };
    Ok(Some(state))
}

fn write_global_state_object(codex_home: &Path, state: Map<String, Value>) -> anyhow::Result<()> {
    fs::create_dir_all(codex_home)?;
    fs::write(
        codex_home.join(".codex-global-state.json"),
        serde_json::to_string_pretty(&Value::Object(state))?,
    )?;
    Ok(())
}

fn remove_project_from_array(
    state: &mut Map<String, Value>,
    key: &str,
    project_path: &str,
) -> bool {
    let Some(Value::Array(items)) = state.get_mut(key) else {
        return false;
    };
    let before = items.len();
    items.retain(|item| {
        item.as_str()
            .map(|path| normalize_project_path(path) != project_path)
            .unwrap_or(true)
    });
    items.len() != before
}

fn add_project_to_array(state: &mut Map<String, Value>, key: &str, project_path: &str) {
    let value = state
        .entry(key.to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    let Value::Array(items) = value else {
        return;
    };
    if items
        .iter()
        .filter_map(Value::as_str)
        .any(|path| normalize_project_path(path) == project_path)
    {
        return;
    }
    items.push(Value::String(project_path.to_string()));
}

fn remove_project_label(state: &mut Map<String, Value>, project_path: &str) -> bool {
    let Some(Value::Object(labels)) = state.get_mut("electron-workspace-root-labels") else {
        return false;
    };
    let keys = labels
        .keys()
        .filter(|key| normalize_project_path(key) == project_path)
        .cloned()
        .collect::<Vec<_>>();
    let removed = !keys.is_empty();
    for key in keys {
        labels.remove(&key);
    }
    removed
}

fn normalize_project_path(path: &str) -> String {
    path.trim().trim_end_matches(['/', '\\']).to_string()
}

fn collect_session_files(root: &Path, files: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_session_files(&path, files)?;
        } else if path.extension().and_then(|value| value.to_str()) == Some("jsonl") {
            files.push(path);
        }
    }
    Ok(())
}

fn parse_session_file(
    sessions_root: &Path,
    absolute_path: PathBuf,
) -> anyhow::Result<Option<SessionRecord>> {
    let relative_path = absolute_path
        .strip_prefix(sessions_root)
        .map_err(|_| anyhow::anyhow!("session path is outside the sessions directory"))?
        .to_path_buf();
    let file = fs::File::open(&absolute_path)?;
    let metadata = file.metadata()?;
    let fallback_time = metadata_time(&metadata);
    let mut id = None;
    let mut title = None;
    let mut created_at = None;
    let mut updated_at = None;
    let mut message_count = 0;
    let mut project_path = String::new();

    for line in BufReader::new(file).lines() {
        let line = line?;
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let timestamp = timestamp_from_value(&value);
        if created_at.is_none() {
            created_at = timestamp;
        }
        if timestamp.is_some() {
            updated_at = timestamp;
        }

        if value.get("type").and_then(Value::as_str) == Some("session_meta") {
            if let Some(meta_id) = value
                .get("payload")
                .and_then(|payload| payload.get("id"))
                .and_then(Value::as_str)
            {
                id = Some(meta_id.to_string());
            }
            if created_at.is_none() {
                created_at = value
                    .get("payload")
                    .and_then(|payload| payload.get("timestamp"))
                    .and_then(Value::as_str)
                    .and_then(parse_rfc3339_timestamp);
            }
            if project_path.is_empty() {
                project_path = value
                    .get("payload")
                    .and_then(|payload| payload.get("cwd"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
            }
            project_path = normalize_project_path(&project_path);
        }

        if let Some(entry) = preview_entry_from_event(&value) {
            message_count += 1;
            if title.is_none() && entry.role == "user" {
                title = Some(title_from_text(&entry.text));
            }
        }
    }

    let id = id.or_else(|| filename_session_id(&absolute_path));
    let Some(id) = id else {
        return Ok(None);
    };
    let created_at = created_at.unwrap_or(fallback_time);
    let updated_at = updated_at.unwrap_or(created_at);
    let title = title.unwrap_or_else(|| "Untitled Codex session".to_string());
    Ok(Some(SessionRecord {
        summary: CodexSessionSummary {
            id,
            title,
            created_at,
            updated_at,
            sidebar_order: 0,
            activity_rank: 0,
            message_count,
            file_size_bytes: metadata.len(),
            relative_path: relative_path.to_string_lossy().into_owned(),
            project_name: project_name_from_path(&project_path),
            project_path,
        },
        absolute_path,
        relative_path,
        sort_updated_at_ms: updated_at * 1000,
    }))
}

fn find_session_record(codex_home: &Path, session_id: &str) -> anyhow::Result<SessionRecord> {
    if session_id.trim().is_empty() {
        return Err(anyhow::anyhow!("session id is required"));
    }
    load_session_records(codex_home)?
        .into_iter()
        .find(|record| record.summary.id == session_id)
        .ok_or_else(|| anyhow::anyhow!("session `{session_id}` does not exist"))
}

fn read_preview_entries(path: &Path) -> anyhow::Result<Vec<CodexSessionPreviewEntry>> {
    let file = fs::File::open(path)?;
    let mut entries = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line?;
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(entry) = preview_entry_from_event(&value) {
            entries.push(entry);
            if entries.len() >= MAX_PREVIEW_ENTRIES {
                break;
            }
        }
    }
    Ok(entries)
}

fn preview_entry_from_event(value: &Value) -> Option<CodexSessionPreviewEntry> {
    let payload = value.get("payload")?;
    let payload_type = payload.get("type").and_then(Value::as_str);
    let timestamp = timestamp_from_value(value);
    match (
        value.get("type").and_then(Value::as_str),
        payload_type,
        payload.get("role").and_then(Value::as_str),
    ) {
        (Some("event_msg"), Some("user_message"), _) => {
            let text = payload.get("message").and_then(Value::as_str)?;
            Some(CodexSessionPreviewEntry {
                role: "user".to_string(),
                text: clean_text(text),
                timestamp,
            })
        }
        (Some("event_msg"), Some("agent_message"), _) => {
            let text = payload.get("message").and_then(Value::as_str)?;
            Some(CodexSessionPreviewEntry {
                role: "assistant".to_string(),
                text: clean_text(text),
                timestamp,
            })
        }
        (Some("response_item"), Some("message"), Some(role)) => {
            let text = message_content_text(payload)?;
            Some(CodexSessionPreviewEntry {
                role: normalize_role(role).to_string(),
                text,
                timestamp,
            })
        }
        _ => None,
    }
}

fn message_content_text(payload: &Value) -> Option<String> {
    let content = payload.get("content")?;
    let mut parts = Vec::new();
    collect_text(content, &mut parts);
    let text = clean_text(&parts.join(" "));
    if text.is_empty() { None } else { Some(text) }
}

fn collect_text(value: &Value, parts: &mut Vec<String>) {
    match value {
        Value::String(text) => parts.push(text.clone()),
        Value::Array(items) => {
            for item in items {
                collect_text(item, parts);
            }
        }
        Value::Object(map) => {
            for key in ["text", "message", "content"] {
                if let Some(value) = map.get(key) {
                    collect_text(value, parts);
                }
            }
        }
        _ => {}
    }
}

fn timestamp_from_value(value: &Value) -> Option<i64> {
    value
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(parse_rfc3339_timestamp)
}

fn parse_rfc3339_timestamp(value: &str) -> Option<i64> {
    OffsetDateTime::parse(value, &Rfc3339)
        .ok()
        .map(|time| time.unix_timestamp())
}

fn metadata_time(metadata: &fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_else(current_unix_timestamp)
}

fn current_unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn timestamp_folder() -> String {
    let now = OffsetDateTime::from_unix_timestamp(current_unix_timestamp())
        .unwrap_or(OffsetDateTime::UNIX_EPOCH);
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    )
}

fn title_from_text(text: &str) -> String {
    let text = clean_text(text);
    if text.chars().count() <= 72 {
        return text;
    }
    let mut title: String = text.chars().take(72).collect();
    title.push('…');
    title
}

fn project_name_from_path(path: &str) -> String {
    let trimmed = path.trim().trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return "未指定项目".to_string();
    }
    trimmed
        .rsplit(|ch| ch == '/' || ch == '\\')
        .find(|part| !part.is_empty())
        .unwrap_or(trimmed)
        .to_string()
}

fn clean_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_role(role: &str) -> &str {
    match role {
        "assistant" => "assistant",
        "user" => "user",
        "system" => "system",
        _ => "message",
    }
}

fn filename_session_id(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|value| value.to_str())
        .map(ToString::to_string)
}

fn is_named_process_running(names: &[&str]) -> bool {
    #[cfg(target_os = "windows")]
    {
        let Ok(output) = Command::new("tasklist").output() else {
            return false;
        };
        let text = String::from_utf8_lossy(&output.stdout).to_lowercase();
        return names
            .iter()
            .any(|name| text.contains(&format!("{}.exe", name.to_lowercase())));
    }

    #[cfg(not(target_os = "windows"))]
    {
        names.iter().any(|name| {
            Command::new("pgrep")
                .arg("-x")
                .arg(name)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|status| status.success())
                .unwrap_or(false)
        })
    }
}
