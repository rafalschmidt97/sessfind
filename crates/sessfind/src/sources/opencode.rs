use anyhow::{Context, Result};
use chrono::{TimeZone, Utc};
use rusqlite::Connection;
use std::path::Path;

use crate::config;
use crate::models::{Message, Role, Session, Source};
use crate::sources::SessionSource;

pub struct OpenCodeSource;

impl OpenCodeSource {
    pub fn new() -> Self {
        Self
    }
}

fn list_sessions_from_connection(conn: &Connection, db_path: &Path) -> Result<Vec<Session>> {
    let mut stmt = conn.prepare(
        "SELECT s.id, s.title, s.directory, s.time_created, s.time_updated,
                p.name as project_name
         FROM session s
         LEFT JOIN project p ON s.project_id = p.id
         WHERE s.parent_id IS NULL
         ORDER BY s.time_created DESC",
    )?;

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
        .filter_map(|r| r.ok())
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
                    file_size: time_updated as u64,
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
        let db_path = config::opencode_db_path();
        if !db_path.exists() {
            return Ok(vec![]);
        }

        let conn =
            Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .with_context(|| format!("Failed to open OpenCode DB: {}", db_path.display()))?;

        list_sessions_from_connection(&conn, &db_path)
    }

    fn load_messages(&self, session: &Session) -> Result<Vec<Message>> {
        let db_path = config::opencode_db_path();
        let conn =
            Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;

        // Get messages with their text parts
        let mut stmt = conn.prepare(
            "SELECT m.data, p.data as part_data, p.time_created
             FROM message m
             JOIN part p ON p.message_id = m.id
             WHERE m.session_id = ?1
             ORDER BY p.time_created ASC",
        )?;

        let mut messages = Vec::new();

        let rows = stmt.query_map([&session.session_id], |row| {
            let msg_data: String = row.get(0)?;
            let part_data: String = row.get(1)?;
            let part_time: i64 = row.get(2)?;
            Ok((msg_data, part_data, part_time))
        })?;

        for row in rows {
            let (msg_data_str, part_data_str, part_time) = row?;

            let msg_data: serde_json::Value = match serde_json::from_str(&msg_data_str) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let part_data: serde_json::Value = match serde_json::from_str(&part_data_str) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Only index text parts
            let part_type = part_data.get("type").and_then(|t| t.as_str()).unwrap_or("");
            if part_type != "text" {
                continue;
            }

            let text = part_data
                .get("text")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            if text.trim().is_empty() {
                continue;
            }

            let role = match msg_data.get("role").and_then(|r| r.as_str()) {
                Some("user") => Role::User,
                Some("assistant") => Role::Assistant,
                _ => continue,
            };

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
}

#[cfg(test)]
mod tests {
    use super::*;

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
