//! File operation journaling for undo/rollback support.
//!
//! Tracks file modifications to enable rollback of agent actions.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};

const MAX_JOURNAL_SIZE: usize = 100;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OperationType {
    Write,
    Edit,
    Delete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileOperation {
    pub op_type: OperationType,
    pub path: PathBuf,
    pub timestamp: DateTime<Utc>,
    pub backup_path: Option<PathBuf>,
    pub original_content: Option<Vec<u8>>,
    pub new_content: Option<Vec<u8>>,
}

pub struct OperationJournal {
    operations: VecDeque<FileOperation>,
    journal_path: PathBuf,
}

impl OperationJournal {
    pub fn new(journal_path: PathBuf) -> Self {
        let mut journal = Self {
            operations: VecDeque::new(),
            journal_path,
        };
        journal.load().ok();
        journal
    }

    pub fn record_write(&mut self, path: &Path, content: &[u8]) -> std::io::Result<()> {
        let backup_path = path.to_path_buf();

        let original_content = if path.exists() {
            Some(fs::read(path)?)
        } else {
            None
        };

        let op = FileOperation {
            op_type: OperationType::Write,
            path: path.to_path_buf(),
            timestamp: Utc::now(),
            backup_path: Some(backup_path),
            original_content,
            new_content: Some(content.to_vec()),
        };

        self.add_operation(op);
        Ok(())
    }

    pub fn record_delete(&mut self, path: &Path) -> std::io::Result<()> {
        let original_content = if path.exists() {
            Some(fs::read(path)?)
        } else {
            None
        };

        let op = FileOperation {
            op_type: OperationType::Delete,
            path: path.to_path_buf(),
            timestamp: Utc::now(),
            backup_path: None,
            original_content,
            new_content: None,
        };

        self.add_operation(op);
        Ok(())
    }

    pub fn undo_last(&mut self) -> Result<String, String> {
        let op = self.operations.pop_back().ok_or("No operations to undo")?;
        self.save().map_err(|e| e.to_string())?;

        match op.op_type {
            OperationType::Write | OperationType::Edit => {
                if let Some(original) = op.original_content {
                    fs::write(&op.path, original).map_err(|e| e.to_string())?;
                    Ok(format!("Restored: {}", op.path.display()))
                } else if op.path.exists() {
                    fs::remove_file(&op.path).map_err(|e| e.to_string())?;
                    Ok(format!("Deleted: {}", op.path.display()))
                } else {
                    Ok(format!("File did not exist: {}", op.path.display()))
                }
            }
            OperationType::Delete => {
                if let Some(content) = op.original_content {
                    if let Some(parent) = op.path.parent() {
                        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                    }
                    fs::write(&op.path, content).map_err(|e| e.to_string())?;
                    Ok(format!("Restored: {}", op.path.display()))
                } else {
                    Ok(format!(
                        "Cannot restore: {} had no content",
                        op.path.display()
                    ))
                }
            }
        }
    }

    pub fn undo_n(&mut self, n: usize) -> Result<String, String> {
        let mut messages = Vec::new();
        for _ in 0..n {
            match self.undo_last() {
                Ok(msg) => messages.push(msg),
                Err(e) => return Err(e),
            }
        }
        Ok(messages.join("\n"))
    }

    pub fn clear(&mut self) {
        self.operations.clear();
        self.save().ok();
    }

    pub fn len(&self) -> usize {
        self.operations.len()
    }

    pub fn is_empty(&self) -> bool {
        self.operations.is_empty()
    }

    fn add_operation(&mut self, op: FileOperation) {
        if self.operations.len() >= MAX_JOURNAL_SIZE {
            self.operations.pop_front();
        }
        self.operations.push_back(op);
        self.save().ok();
    }

    fn load(&mut self) -> std::io::Result<()> {
        if self.journal_path.exists() {
            let data = fs::read(&self.journal_path)?;
            let ops: VecDeque<FileOperation> = serde_json::from_slice(&data).unwrap_or_default();
            self.operations = ops;
        }
        Ok(())
    }

    fn save(&self) -> std::io::Result<()> {
        if let Some(parent) = self.journal_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_vec(&self.operations)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        fs::write(&self.journal_path, data)
    }
}

impl Default for OperationJournal {
    fn default() -> Self {
        Self {
            operations: VecDeque::new(),
            journal_path: PathBuf::from(".rustclaw/journal.json"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_journal_write_and_undo() {
        let dir = tempdir().unwrap();
        let journal_path = dir.path().join("journal.json");
        let file_path = dir.path().join("test.txt");

        let mut journal = OperationJournal::new(journal_path);

        journal.record_write(&file_path, b"hello world").unwrap();
        assert_eq!(journal.len(), 1);

        let result = journal.undo_last();
        assert!(result.is_ok());
        assert!(!file_path.exists());
    }

    #[test]
    fn test_journal_multiple_operations() {
        let dir = tempdir().unwrap();
        let journal_path = dir.path().join("journal.json");
        let file1 = dir.path().join("test1.txt");
        let file2 = dir.path().join("test2.txt");

        let mut journal = OperationJournal::new(journal_path);

        journal.record_write(&file1, b"content1").unwrap();
        journal.record_write(&file2, b"content2").unwrap();
        assert_eq!(journal.len(), 2);

        let result = journal.undo_n(2);
        assert!(result.is_ok());
        assert!(!file1.exists() && !file2.exists());
    }
}
