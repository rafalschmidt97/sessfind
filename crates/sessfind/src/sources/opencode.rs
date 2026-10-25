use anyhow::{Context, Result};
use chrono::{TimeZone, Utc};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

use crate::config;
use crate::models::{Message, Role, Session, Source};
use crate::sources::SessionSource;

pub struct OpenCodeSource {
    db_path: PathBuf,
}

impl OpenCodeSource {
    pub fn new() -> Self {
        Self {
            db_path: config::opencode_db_path(),
        }
    }
}

fn list_sessions_from_connection(conn: &Connection, db_path: &Path) -> Result<Vec<Session>> {
    let has_v2 = has_table(conn, "session_v2")?;
    let has_legacy = has_table(conn, "session")?;
    anyhow::ensure!(has_v2 || has_legacy, "Unrecognized OpenCode session schema");
    let mut sessions = Vec::new();
    if has_v2 {
        sessions.extend(list_session_table(conn, db_path, true, false)?);
    }
    if has_legacy {
        sessions.extend(list_session_table(conn, db_path, false, has_v2)?);
    }
    sessions.sort_by(|a, b| {
        b.started_at
            .cmp(&a.started_at)
            .then(a.session_id.cmp(&b.session_id))
    });
    Ok(sessions)
}

fn has_table(conn: &Connection, name: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [name],
        |row| row.get(0),
    )?)
}

fn list_session_table(
    conn: &Connection,
    db_path: &Path,
    v2: bool,
    exclude_migrated: bool,
) -> Result<Vec<Session>> {
    let table = if v2 { "session_v2" } else { "session" };
    // V2 owns migrated IDs, even if the legacy row has a different parent/title.
    let unmigrated = if exclude_migrated {
        "AND NOT EXISTS (SELECT 1 FROM session_v2 v WHERE v.id = s.id)"
    } else {
        ""
    };
    let mut stmt = conn.prepare(&format!(
        "SELECT s.id, s.title, s.directory, s.time_created, s.time_updated,
                p.name as project_name
         FROM {table} s
         LEFT JOIN project p ON s.project_id = p.id
         WHERE s.parent_id IS NULL {unmigrated}
         ORDER BY s.time_created DESC"
    ))?;

    let sessions = stmt
        .query_map([], |row| {
            let id: String = row.get(0)?;
            let title: Option<String> = row.get(1)?;
            let directory: Option<String> = row.get(2)?;
            let time_created: i64 = row.get(3)?;
            let time_updated: i64 = row.get(4)?;
            let project_name: Option<String> = row.get(5)?;

            Ok((
                id,
                title,
                directory,
                time_created,
                time_updated,
                project_name,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .map(
            |(id, title, directory, time_created, time_updated, project_name)| {
                let started_at = Utc
                    .timestamp_millis_opt(time_created)
                    .single()
                    .unwrap_or_else(Utc::now);
                let dir = directory.clone().unwrap_or_default();
                let project = project_name.unwrap_or_else(|| dir.clone());

                Session {
                    source: Source::OpenCode,
                    session_id: id,
                    project,
                    directory: dir,
                    title,
                    started_at,
                    model: None,
                    file_path: db_path.to_string_lossy().to_string(),
                    // Use time_updated as pseudo-mtime for change detection
                    file_mtime: time_updated / 1000,
                    // Reserve a format bit below SQLite's signed integer limit.
                    // Migration must invalidate unchanged legacy timestamps too.
                    file_size: (time_updated as u64) | if v2 { 1 << 62 } else { 0 },
                }
            },
        )
        .collect();

    Ok(sessions)
}

impl SessionSource for OpenCodeSource {
    fn name(&self) -> &'static str {
        "opencode"
    }

    fn list_sessions(&self) -> Result<Vec<Session>> {
        let db_path = &self.db_path;
        if !db_path.exists() {
            return Ok(vec![]);
        }

        let conn = Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .with_context(|| format!("Failed to open OpenCode DB: {}", db_path.display()))?;

        list_sessions_from_connection(&conn, db_path)
    }

    fn load_messages(&self, session: &Session) -> Result<Vec<Message>> {
        let db_path = &session.file_path;
        let conn =
            Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        load_messages_from_connection(&conn, &session.session_id)
    }
}

fn load_messages_from_connection(conn: &Connection, session_id: &str) -> Result<Vec<Message>> {
    if has_table(conn, "session_v2")?
        && conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM session_v2 WHERE id = ?1)",
            [session_id],
            |row| row.get::<_, bool>(0),
        )?
    {
        return load_v2_messages(conn, session_id);
    }
    // Get messages with their text parts
    let mut stmt = conn.prepare(
        "SELECT m.data, p.data as part_data, p.time_created
             FROM message m
             JOIN part p ON p.message_id = m.id
             WHERE m.session_id = ?1
             ORDER BY m.time_created ASC, m.id ASC, p.time_created ASC, p.id ASC",
    )?;

    let mut messages = Vec::new();

    let rows = stmt.query_map([session_id], |row| {
        let msg_data: String = row.get(0)?;
        let part_data: String = row.get(1)?;
        let part_time: i64 = row.get(2)?;
        Ok((msg_data, part_data, part_time))
    })?;

    for row in rows {
        let (msg_data_str, part_data_str, part_time) = row?;

        let msg_data: serde_json::Value =
            serde_json::from_str(&msg_data_str).context("Malformed OpenCode message data")?;
        let part_data: serde_json::Value =
            serde_json::from_str(&part_data_str).context("Malformed OpenCode part data")?;

        // Only index text parts
        let part_type = part_data.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if part_type != "text" && part_type != "file" {
            continue;
        }

        let mut text = if part_type == "file" {
            "[File attachment: contents not inspected]".to_string()
        } else {
            part_data
                .get("text")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string()
        };
        if text.trim().is_empty() {
            continue;
        }

        let role = match msg_data.get("role").and_then(|r| r.as_str()) {
            Some("user") => Role::User,
            Some("assistant") => Role::Assistant,
            _ => continue,
        };
        if msg_data.get("summary").and_then(|v| v.as_bool()) == Some(true) {
            text = format!("[Compaction summary]\n{text}");
        }

        let timestamp = Utc.timestamp_millis_opt(part_time).single();

        messages.push(Message {
            role,
            text,
            timestamp,
            tool_names: vec![],
        });
    }

    Ok(messages)
}

fn load_v2_messages(conn: &Connection, session_id: &str) -> Result<Vec<Message>> {
    let mut stmt = conn.prepare(
        "SELECT type, data, time_created FROM session_message
         WHERE session_id = ?1 AND type IN ('user', 'assistant', 'compaction')
         ORDER BY seq ASC",
    )?;
    let rows = stmt.query_map([session_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    let mut messages = Vec::new();
    for row in rows {
        let (kind, data, time) = row?;
        let data: serde_json::Value =
            serde_json::from_str(&data).context("Malformed OpenCode V2 message data")?;
        let mut texts = Vec::new();
        let role = match kind.as_str() {
            "user" => {
                if let Some(text) = data.get("text").and_then(|v| v.as_str()) {
                    texts.push(text.to_string());
                }
                if let Some(files) = data.get("files").and_then(|v| v.as_array()) {
                    texts.extend(
                        files
                            .iter()
                            .map(|_| "[File attachment: contents not inspected]".into()),
                    );
                }
                Role::User
            }
            "assistant" => {
                if let Some(content) = data.get("content").and_then(|v| v.as_array()) {
                    for block in content {
                        if block.get("type").and_then(|v| v.as_str()) == Some("text")
                            && let Some(text) = block.get("text").and_then(|v| v.as_str())
                        {
                            texts.push(text.to_string());
                        }
                    }
                }
                Role::Assistant
            }
            "compaction" => {
                if let Some(summary) = data
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                {
                    texts.push(format!("[Compaction summary]\n{summary}"));
                }
                Role::Assistant
            }
            _ => continue,
        };
        let text = texts.join("\n");
        if !text.trim().is_empty() {
            messages.push(Message {
                role,
                text,
                timestamp: Utc.timestamp_millis_opt(time).single(),
                tool_names: vec![],
            });
        }
    }
    Ok(messages)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mixed_database() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"CREATE TABLE project (id TEXT PRIMARY KEY, name TEXT);
             CREATE TABLE session (id TEXT PRIMARY KEY, title TEXT, directory TEXT,
                 time_created INTEGER, time_updated INTEGER, project_id TEXT, parent_id TEXT);
             CREATE TABLE session_v2 (id TEXT PRIMARY KEY, title TEXT, directory TEXT,
                 time_created INTEGER, time_updated INTEGER, project_id TEXT, parent_id TEXT);
             CREATE TABLE message (id TEXT, session_id TEXT, time_created INTEGER, data TEXT);
             CREATE TABLE part (id TEXT, message_id TEXT, time_created INTEGER, data TEXT);
             CREATE TABLE session_message (id TEXT, session_id TEXT, type TEXT, seq INTEGER,
                 time_created INTEGER, time_updated INTEGER, data TEXT);
             INSERT INTO session VALUES
                 ('legacy-only', 'Legacy', '/old', 1000, 3000, NULL, NULL),
                 ('migrated', 'Old title', '/old', 1000, 3000, NULL, NULL),
                 ('child', 'Old child', '/old', 1000, 3000, NULL, NULL);
             INSERT INTO session_v2 VALUES
                 ('greeting', 'Greeting', '/home', 4000, 5000, NULL, NULL),
                 ('migrated', 'New title', '/new', 1000, 3000, NULL, NULL),
                 ('child', 'New child', '/new', 1000, 3000, NULL, 'migrated');
             INSERT INTO message VALUES
                 ('old', 'legacy-only', 1000, '{"role":"user"}');
             INSERT INTO part VALUES
                 ('old-part', 'old', 1000, '{"type":"text","text":"legacy text"}');"#,
        )
        .unwrap();
        let rows = [
            (
                "event",
                "model-switched",
                1,
                1000,
                serde_json::json!({"model":{"id":"model"}}),
            ),
            (
                "user",
                "user",
                6,
                3000,
                serde_json::json!({"text":"Jesteś gotowy mi pomóc?","files":[{"uri":"media:do-not-read"}]}),
            ),
            (
                "assistant",
                "assistant",
                7,
                2000,
                serde_json::json!({"content":[
                    {"type":"text","text":"Tak. Co robimy?"},
                    {"type":"reasoning","text":"must not appear"},
                    {"type":"tool","name":"shell","state":{"content":[{"text":"must not appear"}]}}
                ]}),
            ),
            (
                "summary",
                "compaction",
                9,
                4000,
                serde_json::json!({"summary":"Context summary"}),
            ),
            (
                "idle",
                "idle",
                13,
                5000,
                serde_json::json!({"outcome":"succeeded"}),
            ),
        ];
        for (id, kind, seq, time, data) in rows {
            conn.execute(
                "INSERT INTO session_message VALUES (?1, 'greeting', ?2, ?3, ?4, ?4, ?5)",
                rusqlite::params![id, kind, seq, time, data.to_string()],
            )
            .unwrap();
        }
        conn
    }

    #[test]
    fn v2_sessions_are_discovered_without_duplicating_migrated_or_child_sessions() {
        let conn = mixed_database();
        let sessions = list_sessions_from_connection(&conn, Path::new("opencode.db")).unwrap();
        assert_eq!(sessions.len(), 3);
        assert_eq!(sessions[0].title.as_deref(), Some("Greeting"));
        let migrated = sessions
            .iter()
            .find(|s| s.session_id == "migrated")
            .unwrap();
        assert_eq!(migrated.title.as_deref(), Some("New title"));
        assert_eq!(migrated.directory, "/new");
        assert!(!sessions.iter().any(|s| s.session_id == "child"));
        assert!(sessions.iter().any(|s| s.session_id == "legacy-only"));
    }

    #[test]
    fn v2_conversation_uses_sequence_and_excludes_events_tools_and_reasoning() {
        let conn = mixed_database();
        let messages = load_messages_from_connection(&conn, "greeting").unwrap();
        assert_eq!(
            messages.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(),
            [
                "Jesteś gotowy mi pomóc?\n[File attachment: contents not inspected]",
                "Tak. Co robimy?",
                "[Compaction summary]\nContext summary"
            ]
        );
        assert_eq!(messages[0].role, Role::User);
        assert_eq!(messages[1].role, Role::Assistant);
        assert_eq!(
            load_messages_from_connection(&conn, "legacy-only").unwrap()[0].text,
            "legacy text"
        );
    }

    #[test]
    fn v2_only_database_does_not_require_legacy_tables() {
        let conn = mixed_database();
        conn.execute_batch("DROP TABLE session; DROP TABLE message; DROP TABLE part;")
            .unwrap();
        let sessions = list_sessions_from_connection(&conn, Path::new("v2.db")).unwrap();
        assert_eq!(sessions.len(), 2);
        assert_eq!(
            load_messages_from_connection(&conn, "greeting")
                .unwrap()
                .len(),
            3
        );
    }

    #[test]
    fn migration_reindexes_unchanged_timestamps_and_keeps_legacy_sessions() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("opencode.db");
        let fixture = mixed_database();
        fixture
            .execute("VACUUM INTO ?1", [path.to_str().unwrap()])
            .unwrap();
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            r#"ALTER TABLE session_v2 RENAME TO future_session_v2;
               INSERT INTO message VALUES ('migrated-old', 'migrated', 1000, '{"role":"user"}');
               INSERT INTO part VALUES ('old-content', 'migrated-old', 1000, '{"type":"text","text":"old content"}');
               INSERT INTO session_message VALUES ('migrated-new', 'migrated', 'user', 1, 1000, 1000, '{"text":"new migrated content"}');"#,
        ).unwrap();
        let source = OpenCodeSource { db_path: path };
        let engine = crate::indexer::engine::IndexEngine::open(&temp.path().join("index")).unwrap();
        assert_eq!(engine.index_source(&source, false).unwrap().new_sessions, 2);
        conn.execute_batch("ALTER TABLE future_session_v2 RENAME TO session_v2;")
            .unwrap();
        let stats = engine.index_source(&source, false).unwrap();
        assert_eq!(
            (
                stats.new_sessions,
                stats.updated_sessions,
                stats.unchanged_sessions
            ),
            (1, 1, 1)
        );
        assert_eq!(stats.removed_sessions, 0);
        let chunks = engine
            .get_session_chunks_for_source("migrated", Source::OpenCode)
            .unwrap();
        assert!(chunks[0].snippet.contains("new migrated content"));
        let greeting = engine
            .get_session_chunks_for_source("greeting", Source::OpenCode)
            .unwrap();
        assert!(greeting[0].snippet.contains("Jesteś gotowy mi pomóc?"));
        assert!(
            engine.conversation_preview(&greeting[0]).0[1]
                .snippet
                .contains("Tak. Co robimy?")
        );
    }

    #[test]
    fn unknown_schema_errors_instead_of_reporting_an_empty_source() {
        let conn = Connection::open_in_memory().unwrap();
        assert!(list_sessions_from_connection(&conn, Path::new("unknown.db")).is_err());
    }

    #[test]
    fn native_parts_follow_message_order_with_stable_ties_and_no_reasoning() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE message (id TEXT, session_id TEXT, time_created INTEGER, data TEXT);
            CREATE TABLE part (id TEXT, message_id TEXT, time_created INTEGER, data TEXT);
            INSERT INTO message VALUES ('m1', 's', 1, '{\"role\":\"user\"}'), ('m2', 's', 2, '{\"role\":\"assistant\"}');
            INSERT INTO part VALUES
            ('p2', 'm2', 2, '{\"type\":\"text\",\"text\":\"answer\"}'),
            ('p1b', 'm1', 3, '{\"type\":\"text\",\"text\":\"second part\"}'),
            ('p1a', 'm1', 3, '{\"type\":\"text\",\"text\":\"first part\"}'),
            ('p3', 'm2', 4, '{\"type\":\"reasoning\",\"text\":\"private\"}'),
            ('p4', 'm2', 5, '{\"type\":\"tool\",\"state\":{\"output\":\"not conversation\"}}');").unwrap();
        let messages = load_messages_from_connection(&conn, "s").unwrap();
        assert_eq!(
            messages.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(),
            ["first part", "second part", "answer"]
        );
    }

    #[test]
    fn list_sessions_excludes_child_sessions_by_parent_id() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE project (id TEXT PRIMARY KEY, name TEXT);
             CREATE TABLE session (
                 id TEXT PRIMARY KEY,
                 title TEXT,
                 directory TEXT,
                 time_created INTEGER NOT NULL,
                 time_updated INTEGER NOT NULL,
                 project_id TEXT,
                 parent_id TEXT
             );
             INSERT INTO project (id, name) VALUES ('project', 'Example');
             INSERT INTO session VALUES
                 ('root', 'Root session', '/repo', 1000, 2000, 'project', NULL),
                 ('root-with-subagent-title', 'Looks like (@general subagent)', '/repo', 2000, 3000, 'project', NULL),
                 ('child', 'Title does not identify a subagent', '/repo', 3000, 4000, 'project', 'root');",
        )
        .unwrap();

        let sessions = list_sessions_from_connection(&conn, Path::new("opencode.db")).unwrap();
        let ids: Vec<_> = sessions
            .iter()
            .map(|session| session.session_id.as_str())
            .collect();

        assert_eq!(ids, vec!["root-with-subagent-title", "root"]);
    }
}
