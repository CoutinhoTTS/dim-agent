use crate::{Error, ObjectResult, Result, TaskReport, TaskState, TaskStatus};
use dim_agent_capabilities::file_storage::{FileLease, FilePath, FileStorage, FileStorageError};
use serde::de::DeserializeOwned;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub(crate) struct Ledger {
    pub files: Arc<FileStorage>,
}

impl Ledger {
    pub async fn open(files: Arc<FileStorage>) -> Result<Self> {
        let cancel = CancellationToken::new();
        for path in ["migration", "migration/tasks", "migration/snapshots"] {
            files
                .create_directory(&files.path(0, path)?, &cancel)
                .await?;
        }
        Ok(Self { files })
    }

    fn path(&self, id: &str, name: &str) -> Result<FilePath> {
        uuid::Uuid::parse_str(id).map_err(|_| Error::Object("Task id must be a UUID".into()))?;
        Ok(self.files.path(0, format!("migration/tasks/{id}/{name}"))?)
    }

    pub async fn create(&self, id: &str) -> Result<()> {
        uuid::Uuid::parse_str(id).map_err(|_| Error::Object("Task id must be a UUID".into()))?;
        let cancel = CancellationToken::new();
        self.files
            .create_directory(
                &self.files.path(0, format!("migration/tasks/{id}"))?,
                &cancel,
            )
            .await?;
        self.files
            .create_directory(&self.path(id, "results")?, &cancel)
            .await?;
        Ok(())
    }

    pub async fn lock(&self, id: &str) -> Result<FileLease> {
        Ok(self
            .files
            .lock(&self.path(id, "run")?, &CancellationToken::new())
            .await?)
    }

    pub async fn save(&self, task: &TaskState) -> Result<()> {
        let bytes = serde_json::to_vec(task)
            .map_err(|_| Error::Source("Task state cannot be encoded".into()))?;
        self.files
            .replace(
                &self.path(&task.id, "state.json")?,
                bytes,
                None,
                &CancellationToken::new(),
            )
            .await?;
        Ok(())
    }

    async fn read<T: DeserializeOwned>(&self, path: &FilePath) -> Result<T> {
        let bytes = self
            .files
            .read_limited(path, 16 * 1024 * 1024, &CancellationToken::new())
            .await?;
        serde_json::from_slice(&bytes).map_err(|_| {
            Error::Source("Migration record has an unsupported or damaged format".into())
        })
    }

    pub async fn state(&self, id: &str) -> Result<TaskState> {
        let mut task: TaskState = self.read(&self.path(id, "state.json")?).await?;
        if task.format != 1 || task.id != id {
            return Err(Error::Source(
                "Migration task identity or format does not match".into(),
            ));
        }
        if task.status == TaskStatus::Running {
            match self.lock(id).await {
                Ok(_lease) => task.status = TaskStatus::Interrupted,
                Err(Error::Storage(FileStorageError::Locked)) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(task)
    }

    pub async fn object(&self, id: &str, result: &ObjectResult) -> Result<()> {
        let digest = crate::source::fingerprint(&(result.kind.as_str(), result.id.as_str()))?;
        let path = self.path(id, &format!("results/{digest}.json"))?;
        self.files
            .replace(
                &path,
                serde_json::to_vec(result)
                    .map_err(|_| Error::Source("Object result cannot be encoded".into()))?,
                None,
                &CancellationToken::new(),
            )
            .await?;
        Ok(())
    }

    pub async fn report(&self, id: &str) -> Result<TaskReport> {
        let task = self.state(id).await?;
        let mut objects = Vec::new();
        for entry in self
            .files
            .list(&self.path(id, "results")?, &CancellationToken::new())
            .await?
        {
            let name = entry
                .name
                .to_str()
                .ok_or_else(|| Error::Source("Migration result path is not UTF-8".into()))?;
            if !name.ends_with(".json") {
                return Err(Error::Source("Unexpected migration result entry".into()));
            }
            objects.push(
                self.read(&self.path(id, &format!("results/{name}"))?)
                    .await?,
            );
        }
        objects.sort_by(|a: &ObjectResult, b| (&a.kind, &a.id).cmp(&(&b.kind, &b.id)));
        Ok(TaskReport { task, objects })
    }

    pub async fn list(&self) -> Result<Vec<TaskState>> {
        let mut tasks = Vec::new();
        for entry in self
            .files
            .list(
                &self.files.path(0, "migration/tasks")?,
                &CancellationToken::new(),
            )
            .await?
        {
            let id = entry
                .name
                .to_str()
                .ok_or_else(|| Error::Source("Migration task path is not UTF-8".into()))?;
            tasks.push(self.state(id).await?);
        }
        tasks.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(tasks)
    }

    pub async fn request_cancel(&self, id: &str) -> Result<()> {
        self.files
            .replace(
                &self.path(id, "cancel")?,
                b"true".to_vec(),
                None,
                &CancellationToken::new(),
            )
            .await?;
        Ok(())
    }

    pub async fn cancelled(&self, id: &str) -> Result<bool> {
        match self
            .files
            .read_limited(&self.path(id, "cancel")?, 16, &CancellationToken::new())
            .await
        {
            Ok(value) if value == b"true" => Ok(true),
            Ok(_) => Err(Error::Source(
                "Migration cancellation record is invalid".into(),
            )),
            Err(FileStorageError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(false)
            }
            Err(error) => Err(error.into()),
        }
    }
}
