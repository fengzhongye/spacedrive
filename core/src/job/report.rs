use crate::{
	library::Library,
	prisma::{job, node},
	util,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::{
	fmt::Debug,
	fmt::{Display, Formatter},
};
use tracing::error;
use uuid::Uuid;

use super::JobError;

#[derive(Debug)]
pub enum JobReportUpdate {
	TaskCount(usize),
	CompletedTaskCount(usize),
	Message(String),
}

#[derive(Debug, Serialize, Deserialize, Type, Clone)]
pub struct JobReport {
	pub id: Uuid,
	pub name: String,
	pub action: Option<String>,
	pub data: Option<Vec<u8>>,
	pub metadata: Option<serde_json::Value>,
	pub is_background: bool,
	pub errors_text: Vec<String>,

	pub created_at: Option<DateTime<Utc>>,
	pub started_at: Option<DateTime<Utc>>,
	pub completed_at: Option<DateTime<Utc>>,

	pub parent_id: Option<Uuid>,

	pub status: JobStatus,
	pub task_count: i32,
	pub completed_task_count: i32,

	pub message: String,
	pub estimated_completion: DateTime<Utc>,
	// pub percentage_complete: f64,
}

impl Display for JobReport {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"Job <name='{}', uuid='{}'> {:#?}",
			self.name, self.id, self.status
		)
	}
}

// convert database struct into a resource struct
impl From<job::Data> for JobReport {
	fn from(data: job::Data) -> Self {
		Self {
			id: Uuid::from_slice(&data.id).expect("corrupted database"),
			is_background: false, // deprecated
			name: data.name,
			action: data.action,
			data: data.data,
			metadata: data.metadata.and_then(|m| {
				serde_json::from_slice(&m).unwrap_or_else(|e| -> Option<serde_json::Value> {
					error!("Failed to deserialize job metadata: {}", e);
					None
				})
			}),
			errors_text: data
				.errors_text
				.map(|errors_str| errors_str.split("\n\n").map(str::to_string).collect())
				.unwrap_or_default(),
			created_at: Some(data.date_created.into()),
			started_at: data.date_started.map(DateTime::into),
			completed_at: data.date_completed.map(DateTime::into),
			parent_id: data
				.parent_id
				.map(|id| Uuid::from_slice(&id).expect("corrupted database")),
			status: JobStatus::try_from(data.status).expect("corrupted database"),
			task_count: data.task_count,
			completed_task_count: data.completed_task_count,
			message: String::new(),
			estimated_completion: data
				.date_estimated_completion
				.map_or(Utc::now(), DateTime::into),
		}
	}
}

impl JobReport {
	pub fn new(uuid: Uuid, name: String) -> Self {
		Self {
			id: uuid,
			is_background: false, // deprecated
			name,
			action: None,
			created_at: None,
			started_at: None,
			completed_at: None,
			status: JobStatus::Queued,
			errors_text: vec![],
			task_count: 0,
			data: None,
			metadata: None,
			parent_id: None,
			completed_task_count: 0,
			message: String::new(),
			estimated_completion: Utc::now(),
		}
	}

	pub fn new_with_action(uuid: Uuid, name: String, action: impl AsRef<str>) -> Self {
		let mut report = Self::new(uuid, name);
		report.action = Some(action.as_ref().to_string());
		report
	}

	pub fn new_with_parent(
		uuid: Uuid,
		name: String,
		parent_id: Uuid,
		action: Option<String>,
	) -> Self {
		let mut report = Self::new(uuid, name);
		report.parent_id = Some(parent_id);
		report.action = action;
		report
	}

	pub async fn create(&mut self, library: &Library) -> Result<(), JobError> {
		let now = Utc::now();
		self.created_at = Some(now);

		library
			.db
			.job()
			.create(
				self.id.as_bytes().to_vec(),
				self.name.clone(),
				node::id::equals(library.node_local_id),
				util::db::chain_optional_iter(
					[
						job::action::set(self.action.clone()),
						job::data::set(self.data.clone()),
						job::date_created::set(now.into()),
						job::status::set(self.status as i32),
						job::date_started::set(self.started_at.map(|d| d.into())),
					],
					[self
						.parent_id
						.map(|id| job::parent::connect(job::id::equals(id.as_bytes().to_vec())))],
				),
			)
			.exec()
			.await?;
		Ok(())
	}

	pub async fn update(&mut self, library: &Library) -> Result<(), JobError> {
		library
			.db
			.job()
			.update(
				job::id::equals(self.id.as_bytes().to_vec()),
				vec![
					job::status::set(self.status as i32),
					job::errors_text::set(
						(!self.errors_text.is_empty()).then(|| self.errors_text.join("\n\n")),
					),
					job::data::set(self.data.clone()),
					job::metadata::set(serde_json::to_vec(&self.metadata).ok()),
					job::task_count::set(self.task_count),
					job::completed_task_count::set(self.completed_task_count),
					job::date_started::set(self.started_at.map(|v| v.into())),
					job::date_completed::set(self.completed_at.map(|v| v.into())),
				],
			)
			.exec()
			.await?;
		Ok(())
	}
}

#[repr(i32)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, Eq, PartialEq)]
pub enum JobStatus {
	Queued = 0,
	Running = 1,
	Completed = 2,
	Canceled = 3,
	Failed = 4,
	Paused = 5,
	CompletedWithErrors = 6,
}

impl TryFrom<i32> for JobStatus {
	type Error = JobError;

	fn try_from(value: i32) -> Result<Self, Self::Error> {
		let s = match value {
			0 => Self::Queued,
			1 => Self::Running,
			2 => Self::Completed,
			3 => Self::Canceled,
			4 => Self::Failed,
			5 => Self::Paused,
			6 => Self::CompletedWithErrors,
			_ => return Err(JobError::InvalidJobStatusInt(value)),
		};

		Ok(s)
	}
}
