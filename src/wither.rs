use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TaskState {
    pub id: String,
    pub status: String,
    pub priority: i32,
    pub tags: Vec<String>,
}

pub struct TaskStateBuilder {
    id: Option<String>,
    status: Option<String>,
    priority: Option<i32>,
    tags: Option<Vec<String>>,
}

impl TaskStateBuilder {
    pub fn new() -> Self {
        Self {
            id: None,
            status: None,
            priority: None,
            tags: None,
        }
    }

    pub fn with_id(mut self, id: String) -> Self {
        self.id = Some(id);
        self
    }

    pub fn with_status(mut self, status: String) -> Self {
        self.status = Some(status);
        self
    }

    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = Some(priority);
        self
    }

    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = Some(tags);
        self
    }

    pub fn with_tag(mut self, tag: String) -> Self {
        if let Some(ref mut tags) = self.tags {
            tags.push(tag);
        } else {
            self.tags = Some(vec![tag]);
        }
        self
    }

    pub fn build(self) -> TaskState {
        TaskState {
            id: self.id.unwrap_or_default(),
            status: self.status.unwrap_or_else(|| "pending".to_string()),
            priority: self.priority.unwrap_or(0),
            tags: self.tags.unwrap_or_default(),
        }
    }

    pub fn build_from(original: &TaskState) -> Self {
        Self {
            id: Some(original.id.clone()),
            status: Some(original.status.clone()),
            priority: Some(original.priority),
            tags: Some(original.tags.clone()),
        }
    }
}

impl Default for TaskStateBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskState {
    pub fn with_id(&self, id: String) -> Self {
        let mut new = self.clone();
        new.id = id;
        new
    }

    pub fn with_status(&self, status: String) -> Self {
        let mut new = self.clone();
        new.status = status;
        new
    }

    pub fn with_priority(&self, priority: i32) -> Self {
        let mut new = self.clone();
        new.priority = priority;
        new
    }

    pub fn with_tags(&self, tags: Vec<String>) -> Self {
        let mut new = self.clone();
        new.tags = tags;
        new
    }

    pub fn with_tag(&self, tag: String) -> Self {
        let mut new = self.clone();
        new.tags.push(tag);
        new
    }

    pub fn update_status<F>(&self, f: F) -> Self
    where
        F: FnOnce(&mut String),
    {
        let mut new = self.clone();
        f(&mut new.status);
        new
    }

    pub fn update_priority<F>(&self, f: F) -> Self
    where
        F: FnOnce(&mut i32),
    {
        let mut new = self.clone();
        f(&mut new.priority);
        new
    }

    pub fn update_tags<F>(&self, f: F) -> Self
    where
        F: FnOnce(&mut Vec<String>),
    {
        let mut new = self.clone();
        f(&mut new.tags);
        new
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Notification {
    pub id: String,
    pub message: String,
    pub notification_type: String,
    pub timestamp: i64,
    pub read: bool,
}

impl Notification {
    pub fn with_id(&self, id: String) -> Self {
        let mut n = self.clone();
        n.id = id;
        n
    }

    pub fn with_message(&self, message: String) -> Self {
        let mut n = self.clone();
        n.message = message;
        n
    }

    pub fn with_type(&self, notification_type: String) -> Self {
        let mut n = self.clone();
        n.notification_type = notification_type;
        n
    }

    pub fn mark_read(&self) -> Self {
        let mut n = self.clone();
        n.read = true;
        n
    }

    pub fn mark_unread(&self) -> Self {
        let mut n = self.clone();
        n.read = false;
        n
    }

    pub fn toggle_read(&self) -> Self {
        let mut n = self.clone();
        n.read = !n.read;
        n
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AppSettings {
    pub provider: String,
    pub model: String,
    pub max_tokens: usize,
    pub auto_approve: bool,
    pub theme: String,
}

impl AppSettings {
    pub fn with_provider(&self, provider: String) -> Self {
        let mut s = self.clone();
        s.provider = provider;
        s
    }

    pub fn with_model(&self, model: String) -> Self {
        let mut s = self.clone();
        s.model = model;
        s
    }

    pub fn with_max_tokens(&self, max_tokens: usize) -> Self {
        let mut s = self.clone();
        s.max_tokens = max_tokens;
        s
    }

    pub fn with_auto_approve(&self, auto_approve: bool) -> Self {
        let mut s = self.clone();
        s.auto_approve = auto_approve;
        s
    }

    pub fn with_theme(&self, theme: String) -> Self {
        let mut s = self.clone();
        s.theme = theme;
        s
    }

    pub fn update<F>(&self, f: F) -> Self
    where
        F: FnOnce(&mut Self),
    {
        let mut new = self.clone();
        f(&mut new);
        new
    }
}

pub struct WitherExtensions;

impl WitherExtensions {
    pub fn with_opt<T: Clone>(value: Option<T>, f: impl FnOnce(&mut T)) -> Option<T> {
        let mut v = value.unwrap_or_default();
        f(&mut v);
        Some(v)
    }

    pub fn with_ref<T: Clone>(value: &T, f: impl FnOnce(&mut T)) -> T {
        let mut v = value.clone();
        f(&mut v);
        v
    }

    pub fn update_field<T, F>(value: &T, f: F) -> T
    where
        T: Clone,
        F: FnOnce(&mut T),
    {
        let mut v = value.clone();
        f(&mut v);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_withers() {
        let task = TaskState {
            id: "1".to_string(),
            status: "pending".to_string(),
            priority: 1,
            tags: vec!["bug".to_string()],
        };

        let updated = task
            .with_status("done".to_string())
            .with_priority(5)
            .with_tag("urgent");

        assert_eq!(updated.id, "1");
        assert_eq!(updated.status, "done");
        assert_eq!(updated.priority, 5);
        assert!(updated.tags.contains(&"urgent".to_string()));
    }

    #[test]
    fn test_notification_wither() {
        let notif = Notification {
            id: "1".to_string(),
            message: "Test".to_string(),
            notification_type: "info".to_string(),
            timestamp: 0,
            read: false,
        };

        let read = notif.mark_read();
        assert!(read.read);
    }

    #[test]
    fn test_settings_update() {
        let settings = AppSettings::default();

        let updated = settings
            .with_provider("openrouter".to_string())
            .with_max_tokens(8000);

        assert_eq!(updated.provider, "openrouter");
        assert_eq!(updated.max_tokens, 8000);
    }
}
