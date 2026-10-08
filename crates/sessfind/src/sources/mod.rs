pub mod claude_code;
pub mod codex;
pub mod copilot;
pub mod cursor;
pub mod opencode;

use crate::models::{Message, Session, Source};
use anyhow::Result;

pub fn validate_resume_directory(directory: &str) -> Result<()> {
    let path = std::path::Path::new(directory);
    if !path.is_absolute() || !path.is_dir() {
        anyhow::bail!(
            "Recorded session directory is unavailable: {directory:?}. Restore the original directory to resume; sessfind will not create or substitute one."
        );
    }
    Ok(())
}

pub trait SessionSource {
    fn name(&self) -> &'static str;
    fn list_sessions(&self) -> Result<Vec<Session>>;
    fn load_messages(&self, session: &Session) -> Result<Vec<Message>>;
    fn load_conversation(&self, session: &Session) -> Result<Vec<Message>> {
        self.load_messages(session)
    }
}

pub fn source_for(source: Source) -> Box<dyn SessionSource> {
    match source {
        Source::ClaudeCode => Box::new(claude_code::ClaudeCodeSource::new()),
        Source::OpenCode => Box::new(opencode::OpenCodeSource::new()),
        Source::Copilot => Box::new(copilot::CopilotSource::new()),
        Source::Cursor => Box::new(cursor::CursorSource::new()),
        Source::Codex => Box::new(codex::CodexSource::new()),
    }
}

pub fn all_sources() -> Vec<Box<dyn SessionSource>> {
    [
        Source::ClaudeCode,
        Source::OpenCode,
        Source::Copilot,
        Source::Cursor,
        Source::Codex,
    ]
    .into_iter()
    .map(source_for)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_requires_an_existing_absolute_directory_and_never_creates_it() {
        let temp = tempfile::TempDir::new().unwrap();
        assert!(validate_resume_directory(temp.path().to_str().unwrap()).is_ok());
        let missing = temp.path().join("must-not-create");
        assert!(validate_resume_directory(missing.to_str().unwrap()).is_err());
        assert!(!missing.exists());
        assert!(validate_resume_directory("").is_err());
        assert!(validate_resume_directory(".").is_err());
        let file = temp.path().join("file");
        std::fs::write(&file, "").unwrap();
        assert!(validate_resume_directory(file.to_str().unwrap()).is_err());
    }
}
