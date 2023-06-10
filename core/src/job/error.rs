use crate::{
	location::indexer::IndexerError,
	object::{file_identifier::FileIdentifierJobError, preview::ThumbnailerError},
	util::error::FileIOError,
};

use std::{fmt::Debug, hash::Hasher, path::PathBuf};

use rmp_serde::{decode::Error as DecodeError, encode::Error as EncodeError};
use sd_crypto::Error as CryptoError;
use thiserror::Error;
use uuid::Uuid;

use super::JobRunErrors;

#[derive(Error, Debug)]
pub enum JobError {
	// General errors
	#[error("database error")]
	DatabaseError(#[from] prisma_client_rust::QueryError),
	#[error("Failed to join Tokio spawn blocking: {0}")]
	JoinTaskError(#[from] tokio::task::JoinError),
	#[error("Job state encode error: {0}")]
	StateEncode(#[from] EncodeError),
	#[error("Job state decode error: {0}")]
	StateDecode(#[from] DecodeError),
	#[error("Job metadata serialization error: {0}")]
	MetadataSerialization(#[from] serde_json::Error),
	#[error("Tried to resume a job with unknown name: job <name='{1}', uuid='{0}'>")]
	UnknownJobName(Uuid, String),
	#[error(
		"Tried to resume a job that doesn't have saved state data: job <name='{1}', uuid='{0}'>"
	)]
	MissingJobDataState(Uuid, String),
	#[error("missing report field: job <uuid='{id}', name='{name}'>")]
	MissingReport { id: Uuid, name: String },
	#[error("missing some job data: '{value}'")]
	MissingData { value: String },
	#[error("error converting/handling OS strings")]
	OsStr,
	#[error("error converting/handling paths")]
	Path,
	#[error("invalid job status integer")]
	InvalidJobStatusInt(i32),
	#[error(transparent)]
	FileIO(#[from] FileIOError),
	#[error("job failed to pause: {0}")]
	PauseFailed(String),
	#[error("failed to send command to worker")]
	WorkerCommandSendFailed,

	// Specific job errors
	#[error("Indexer error: {0}")]
	IndexerError(#[from] IndexerError),
	#[error("Thumbnailer error: {0}")]
	ThumbnailError(#[from] ThumbnailerError),
	#[error("Identifier error: {0}")]
	IdentifierError(#[from] FileIdentifierJobError),
	#[error("Crypto error: {0}")]
	CryptoError(#[from] CryptoError),
	#[error("source and destination path are the same: {}", .0.display())]
	MatchingSrcDest(PathBuf),
	#[error("action would overwrite another file: {}", .0.display())]
	WouldOverwrite(PathBuf),
	#[error("item of type '{0}' with id '{1}' is missing from the db")]
	MissingFromDb(&'static str, String),
	#[error("the cas id is not set on the path data")]
	MissingCasId,

	// Not errors
	#[error("step completed with errors")]
	StepCompletedWithErrors(JobRunErrors),
	#[error("job had a early finish: <name='{name}', reason='{reason}'>")]
	EarlyFinish { name: String, reason: String },
	#[error("data needed for job execution not found: job <name='{0}'>")]
	JobDataNotFound(String),
	#[error("job paused")]
	Paused(Vec<u8>),
}

#[derive(Error, Debug)]
pub enum JobManagerError {
	#[error("Tried to dispatch a job that is already running: Job <name='{name}', hash='{hash}'>")]
	AlreadyRunningJob { name: &'static str, hash: u64 },

	#[error("Failed to fetch job data from database: {0}")]
	Database(#[from] prisma_client_rust::QueryError),

	#[error("job not found: {0}")]
	NotFound(Uuid),

	#[error("Job error: {0}")]
	Job(#[from] JobError),
}

impl From<JobManagerError> for rspc::Error {
	fn from(value: JobManagerError) -> Self {
		match value {
			JobManagerError::AlreadyRunningJob { .. } => Self::with_cause(
				rspc::ErrorCode::BadRequest,
				"Tried to spawn a job that is already running!".to_string(),
				value,
			),
			JobManagerError::Database(_) => Self::with_cause(
				rspc::ErrorCode::InternalServerError,
				"Error accessing the database".to_string(),
				value,
			),
			JobManagerError::NotFound(_) => Self::with_cause(
				rspc::ErrorCode::NotFound,
				"Job not found".to_string(),
				value,
			),
			JobManagerError::Job(_) => Self::with_cause(
				rspc::ErrorCode::InternalServerError,
				"Job error".to_string(),
				value,
			),
		}
	}
}
