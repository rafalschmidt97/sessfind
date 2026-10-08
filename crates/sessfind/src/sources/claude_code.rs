use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::collections::HashSet;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::config;
use crate::models::{Message, Role, Session, Source};
use crate::sources::SessionSource;

pub struct ClaudeCodeSource {
    projects_dir: PathBuf,
}

impl ClaudeCodeSource {
    pub fn new() -> Self {
        Self {
            projects_dir: config::claude_projects_dir(),
        }
    }
}

// JSONL line types we care about
#[derive(Deserialize)]
#[allow(dead_code)]
struct RawEntry {
    #[serde(rename = "type")]
    entry_type: Option<String>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    cwd: Option<String>,
    slug: Option<String>,
    #[serde(rename = "gitBranch")]
    git_branch: Option<String>,
    timestamp: Option<String>,
    message: Option<RawMessage>,
    uuid: Option<String>,
    attachment: Option<serde_json::Value>,
    #[serde(rename = "isSidechain", default)]
    sidechain: bool,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct RawMessage {
    role: Option<String>,
    content: Option<serde_json::Value>,
    model: Option<String>,
}

/// Storage folder encoding is lossy (dots, slashes and hyphens can collide).
/// Only explicit main-session cwd metadata is a usable resume directory.
pub fn recorded_directory(path: &Path) -> Result<String> {
    let reader = BufReader::new(fs::File::open(path)?);
    let mut directory = None;
    for line in reader.lines() {
        let line = line?;
        if let Ok(entry) = serde_json::from_str::<RawEntry>(&line)
            && !entry.sidechain
            && let Some(cwd) = entry.cwd.filter(|cwd| !cwd.trim().is_empty())
        {
            directory = Some(cwd);
        }
    }
    directory.ok_or_else(|| {
        anyhow::anyhow!(
            "No recorded working directory in Claude transcript {}",
            path.display()
        )
    })
}

fn extract_text_from_content(content: &serde_json::Value) -> (String, Vec<String>) {
    let mut texts = Vec::new();
    let mut tool_names = Vec::new();

    match content {
        serde_json::Value::String(s) => {
            texts.push(s.clone());
        }
        serde_json::Value::Array(blocks) => {
            for block in blocks {
                if let Some(obj) = block.as_object() {
                    match obj.get("type").and_then(|t| t.as_str()) {
                        Some("text") => {
                            if let Some(text) = obj.get("text").and_then(|t| t.as_str()) {
                                texts.push(text.to_string());
                            }
                        }
                        Some("tool_use") => {
                            if let Some(name) = obj.get("name").and_then(|n| n.as_str()) {
                                tool_names.push(name.to_string());
                            }
                        }
                        Some("image") => {
                            texts.push("[Image attachment: contents not inspected]".into())
                        }
                        // Skip thinking, tool_result, etc.
                        _ => {}
                    }
                }
            }
        }
        _ => {}
    }

    (texts.join("\n"), tool_names)
}

/// Strip internal XML tags and meta content from Claude Code messages.
fn clean_message_text(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut remaining = text;

    while let Some(start) = remaining.find('<') {
        // Add text before the tag
        result.push_str(&remaining[..start]);

        if let Some(end) = remaining[start..].find('>') {
            let tag_content = &remaining[start + 1..start + end];
            let tag_name = tag_content
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_start_matches('/');

            // Skip known internal tags and their content
            match tag_name {
                "local-command-caveat"
                | "local-command-stdout"
                | "command-name"
                | "command-message"
                | "command-args"
                | "system-reminder"
                | "user-prompt-submit-hook"
                | "antml:thinking" => {
                    // Find closing tag and skip everything
                    let close_tag = format!("</{tag_name}>");
                    if let Some(close_pos) = remaining.find(&close_tag) {
                        remaining = &remaining[close_pos + close_tag.len()..];
                    } else {
                        remaining = &remaining[start + end + 1..];
                    }
                    continue;
                }
                _ => {
                    // Unknown tag — keep as-is
                    result.push_str(&remaining[start..start + end + 1]);
                    remaining = &remaining[start + end + 1..];
                    continue;
                }
            }
        } else {
            // No closing '>' — keep the rest
            result.push_str(&remaining[start..]);
            break;
        }
    }
    result.push_str(remaining);

    // Clean up common meta lines
    result
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty()
                && !trimmed.starts_with("[Request interrupted by user")
                && !trimmed.starts_with("[Response interrupted by")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn find_session_files(projects_dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in WalkDir::new(projects_dir)
        .min_depth(2)
        .max_depth(2)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "jsonl")
            && !path.to_str().is_some_and(|s| s.contains("subagents"))
        {
            files.push(path.to_path_buf());
        }
    }
    files
}

impl SessionSource for ClaudeCodeSource {
    fn name(&self) -> &'static str {
        "claude"
    }

    fn list_sessions(&self) -> Result<Vec<Session>> {
        if !self.projects_dir.exists() {
            return Ok(vec![]);
        }

        let mut sessions = Vec::new();

        for file_path in find_session_files(&self.projects_dir) {
            let directory = recorded_directory(&file_path).unwrap_or_default();
            let project = if directory.is_empty() {
                "Unknown directory".to_string()
            } else {
                directory.clone()
            };

            // Session ID from filename
            let session_id = file_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();

            // File metadata for incremental indexing
            let metadata = fs::metadata(&file_path)?;
            let file_mtime = metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let file_size = metadata.len();

            // Read first few lines to get metadata
            let file = fs::File::open(&file_path)?;
            let reader = BufReader::new(file);
            let mut started_at = None;
            let mut model = None;
            let mut title = None;

            for line in reader.lines().take(10) {
                let line = line?;
                if line.is_empty() {
                    continue;
                }
                let entry: RawEntry = match serde_json::from_str(&line) {
                    Ok(e) => e,
                    Err(_) => continue,
                };

                if entry.entry_type.as_deref() == Some("user")
                    || entry.entry_type.as_deref() == Some("assistant")
                {
                    if started_at.is_none()
                        && let Some(ts) = &entry.timestamp
                    {
                        started_at = ts.parse::<DateTime<Utc>>().ok();
                    }
                    if title.is_none() {
                        title = entry.slug.clone();
                    }
                    if model.is_none()
                        && let Some(msg) = &entry.message
                    {
                        model = msg.model.clone();
                    }
                }
            }

            sessions.push(Session {
                source: Source::ClaudeCode,
                session_id,
                project: project.clone(),
                directory,
                title,
                started_at: started_at.unwrap_or_else(Utc::now),
                model,
                file_path: file_path.to_string_lossy().to_string(),
                file_mtime,
                file_size,
            });
        }

        Ok(sessions)
    }

    fn load_messages(&self, session: &Session) -> Result<Vec<Message>> {
        let file = fs::File::open(&session.file_path)
            .with_context(|| format!("Failed to open {}", session.file_path))?;
        read_messages(BufReader::new(file), true)
    }

    fn load_conversation(&self, session: &Session) -> Result<Vec<Message>> {
        let file = fs::File::open(&session.file_path)
            .with_context(|| format!("Failed to open {}", session.file_path))?;
        read_messages(BufReader::new(file), false)
    }
}

fn read_messages(reader: impl BufRead, for_index: bool) -> Result<Vec<Message>> {
    let mut messages = Vec::new();
    let mut seen = HashSet::new();
    for (line_number, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let entry: RawEntry = serde_json::from_str(&line)
            .with_context(|| format!("Malformed Claude transcript at line {}", line_number + 1))?;

        let (role, content, identity) = match entry.entry_type.as_deref() {
            Some("attachment") => {
                let Some(attachment) = &entry.attachment else {
                    continue;
                };
                if attachment.get("type").and_then(|v| v.as_str()) != Some("queued_command") {
                    continue;
                }
                (
                    Role::User,
                    attachment.get("prompt"),
                    attachment
                        .get("source_uuid")
                        .or_else(|| attachment.get("delivery_id"))
                        .and_then(|v| v.as_str())
                        .or(entry.uuid.as_deref()),
                )
            }
            Some("user") => (
                Role::User,
                entry.message.as_ref().and_then(|m| m.content.as_ref()),
                entry.uuid.as_deref(),
            ),
            Some("assistant") => (
                Role::Assistant,
                entry.message.as_ref().and_then(|m| m.content.as_ref()),
                entry.uuid.as_deref(),
            ),
            _ => continue,
        };
        let content = match content {
            Some(c) => c,
            None => continue,
        };

        let (mut text, tool_names) = extract_text_from_content(content);
        if for_index {
            text = clean_message_text(&text);
        }

        // Skip empty or system-only messages
        if text.trim().is_empty() {
            continue;
        }
        if let Some(identity) = identity
            && !seen.insert(identity.to_string())
        {
            continue;
        }
        if entry.sidechain {
            text = format!("[Sidechain record]\n{text}");
        }

        let timestamp = entry
            .timestamp
            .as_deref()
            .or_else(|| {
                entry
                    .attachment
                    .as_ref()
                    .and_then(|a| a.get("timestamp"))
                    .and_then(|t| t.as_str())
            })
            .and_then(|ts| ts.parse::<DateTime<Utc>>().ok());

        messages.push(Message {
            role,
            text,
            timestamp,
            tool_names,
        });
    }

    Ok(messages)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_and_directory_use_recorded_cwd_not_lossy_folder_name() {
        let temp = tempfile::TempDir::new().unwrap();
        let project = temp.path().join("-Users-rafal-schmidt-Developer-personal");
        fs::create_dir(&project).unwrap();
        let path = project.join("session.jsonl");
        let mut content = "{\"type\":\"mode\"}\n".repeat(15);
        content.push_str("{\"type\":\"user\",\"cwd\":\"/Users/rafal.schmidt/Developer/personal\",\"message\":{\"content\":\"hi\"}}\n");
        content.push_str(
            "{\"type\":\"assistant\",\"isSidechain\":true,\"cwd\":\"/wrong/sidechain\"}\n",
        );
        fs::write(&path, content).unwrap();
        let source = ClaudeCodeSource {
            projects_dir: temp.path().to_owned(),
        };
        let sessions = source.list_sessions().unwrap();
        assert_eq!(
            sessions[0].project,
            "/Users/rafal.schmidt/Developer/personal"
        );
        assert_eq!(sessions[0].directory, sessions[0].project);
        fs::write(
            &path,
            "{\"type\":\"user\",\"message\":{\"content\":\"no cwd\"}}\n",
        )
        .unwrap();
        assert!(recorded_directory(&path).is_err());
        let sessions = source.list_sessions().unwrap();
        assert_eq!(sessions[0].project, "Unknown directory");
        assert!(sessions[0].directory.is_empty());
    }

    #[test]
    fn queued_user_messages_are_ordered_deduplicated_and_searchable() {
        let raw = concat!(
            "{\"type\":\"user\",\"uuid\":\"first\",\"message\":{\"content\":\"start\"}}\n",
            "{\"type\":\"attachment\",\"attachment\":{\"type\":\"queued_command\",\"source_uuid\":\"queued\",\"prompt\":\"correction\"}}\n",
            "{\"type\":\"user\",\"uuid\":\"queued\",\"message\":{\"content\":\"correction\"}}\n",
            "{\"type\":\"assistant\",\"message\":{\"content\":\"latest endpoint\"}}\n"
        );
        for for_index in [true, false] {
            let messages = read_messages(raw.as_bytes(), for_index).unwrap();
            assert_eq!(
                messages.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(),
                ["start", "correction", "latest endpoint"]
            );
        }
    }

    #[test]
    fn conversation_preserves_text_and_marks_attachments_and_sidechains() {
        let text = format!(
            "<system-reminder>retain in native view</system-reminder>\n{}",
            "żółć\n".repeat(3000)
        );
        let raw = serde_json::json!({"type":"assistant", "isSidechain":true,
            "message":{"content":[{"type":"text","text":text},
              {"type":"image","source":{"data":"not-for-display"}},
              {"type":"thinking","thinking":"not-for-display"},
              {"type":"tool_result","content":"not-for-display"}]}})
        .to_string();
        let messages = read_messages(raw.as_bytes(), false).unwrap();
        assert!(messages[0].text.contains(&text));
        assert!(messages[0].text.contains("[Sidechain record]"));
        assert!(messages[0].text.contains("Image attachment"));
        assert!(!messages[0].text.contains("not-for-display"));
        assert!(
            read_messages(b"{}\n{bad".as_slice(), false)
                .unwrap_err()
                .to_string()
                .contains("line 2")
        );
    }
}
