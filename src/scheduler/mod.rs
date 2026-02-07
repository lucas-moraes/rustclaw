pub mod task;
pub mod heartbeat;

use crate::memory::store::MemoryStore;
use crate::scheduler::task::ScheduledTask;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_cron_scheduler::{Job, JobScheduler};
use tracing::info;

pub struct SchedulerService {
    scheduler: JobScheduler,
    store: Arc<Mutex<MemoryStore>>,
    chat_id: i64,
}

impl SchedulerService {
    pub async fn new(memory_path: &Path, chat_id: i64) -> anyhow::Result<Self> {
        let scheduler = JobScheduler::new().await?;
        let store = Arc::new(Mutex::new(MemoryStore::new(memory_path)?));

        Ok(Self {
            scheduler,
            store,
            chat_id,
        })
    }

    pub async fn init_default_tasks(&self) -> anyhow::Result<()> {
        // Check if heartbeat task already exists
        let store = self.store.lock().await;
        let tasks = store.get_all_tasks()?;
        
        let has_heartbeat = tasks.iter().any(|t| {
            matches!(t.task_type, crate::scheduler::task::TaskType::Heartbeat)
        });

        if !has_heartbeat {
            info!("Creating default Heartbeat task");
            let heartbeat = ScheduledTask::new(
                "Heartbeat Diário".to_string(),
                "0 8 * * *".to_string(), // 8:00 AM every day
                crate::scheduler::task::TaskType::Heartbeat,
            );
            store.save_task(&heartbeat)?;
        }

        Ok(())
    }

    pub async fn load_and_schedule_tasks<F>(&self, callback: F) -> anyhow::Result<()>
    where
        F: Fn(i64, String) + Send + Sync + Clone + 'static,
    {
        let store = self.store.lock().await;
        let tasks = store.get_all_tasks()?;
        drop(store); // Release lock

        for task in tasks {
            if !task.is_active {
                continue;
            }

            let chat_id = self.chat_id;
            let callback = callback.clone();
            
            match &task.task_type {
                crate::scheduler::task::TaskType::Heartbeat => {
                    let job = Job::new_async(&task.cron_expression, move |_uuid, _l| {
                        let callback = callback.clone();
                        Box::pin(async move {
                            let message = heartbeat::generate_heartbeat_message().await;
                            callback(chat_id, message);
                        })
                    })?;
                    
                    self.scheduler.add(job).await?;
                    info!("Scheduled Heartbeat task: {}", task.cron_expression);
                }
                crate::scheduler::task::TaskType::SystemCheck => {
                    let job = Job::new_async(&task.cron_expression, move |_uuid, _l| {
                        let callback = callback.clone();
                        Box::pin(async move {
                            let message = heartbeat::generate_system_check_message().await;
                            callback(chat_id, message);
                        })
                    })?;
                    
                    self.scheduler.add(job).await?;
                    info!("Scheduled SystemCheck task: {}", task.cron_expression);
                }
                crate::scheduler::task::TaskType::Reminder(msg) => {
                    let msg = msg.clone();
                    let job = Job::new_async(&task.cron_expression, move |_uuid, _l| {
                        let msg = msg.clone();
                        let callback = callback.clone();
                        Box::pin(async move {
                            callback(chat_id, format!("🔔 Lembrete: {}", msg));
                        })
                    })?;
                    
                    self.scheduler.add(job).await?;
                    info!("Scheduled Reminder task: {}", task.cron_expression);
                }
                _ => {}
            }
        }

        Ok(())
    }

    pub async fn start(&self) -> anyhow::Result<()> {
        info!("Starting scheduler...");
        self.scheduler.start().await?;
        Ok(())
    }

    pub async fn add_task(&self, task: ScheduledTask) -> anyhow::Result<()> {
        let store = self.store.lock().await;
        store.save_task(&task)?;
        info!("Added task: {} ({})", task.name, task.cron_expression);
        Ok(())
    }

    pub async fn remove_task(&self, task_id: &str) -> anyhow::Result<()> {
        let store = self.store.lock().await;
        store.delete_task(task_id)?;
        info!("Removed task: {}", task_id);
        Ok(())
    }

    pub async fn toggle_task(&self, task_id: &str, is_active: bool) -> anyhow::Result<()> {
        let store = self.store.lock().await;
        store.toggle_task(task_id, is_active)?;
        info!("Toggled task {} to {}", task_id, is_active);
        Ok(())
    }

    pub async fn get_tasks(&self) -> anyhow::Result<Vec<ScheduledTask>> {
        let store = self.store.lock().await;
        let tasks = store.get_all_tasks()?;
        Ok(tasks)
    }
}
