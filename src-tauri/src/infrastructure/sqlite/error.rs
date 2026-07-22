use std::{error::Error, fmt};

pub type RepositoryResult<T> = Result<T, RepositoryError>;

#[derive(Debug)]
pub enum RepositoryError {
    Database(sqlx::Error),
    Migration(sqlx::migrate::MigrateError),
    NotFound { entity: &'static str, id: String },
    Conflict(String),
    InvalidInput(String),
    InvalidStoredValue(String),
}

impl fmt::Display for RepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(error) => write!(formatter, "database error: {error}"),
            Self::Migration(error) => write!(formatter, "database migration error: {error}"),
            Self::NotFound { entity, id } => write!(formatter, "{entity} `{id}` was not found"),
            Self::Conflict(message) => write!(formatter, "repository conflict: {message}"),
            Self::InvalidInput(message) => write!(formatter, "invalid repository input: {message}"),
            Self::InvalidStoredValue(message) => {
                write!(
                    formatter,
                    "invalid value persisted in repository: {message}"
                )
            }
        }
    }
}

impl Error for RepositoryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Database(error) => Some(error),
            Self::Migration(error) => Some(error),
            _ => None,
        }
    }
}

impl From<sqlx::Error> for RepositoryError {
    fn from(value: sqlx::Error) -> Self {
        Self::Database(value)
    }
}

impl From<sqlx::migrate::MigrateError> for RepositoryError {
    fn from(value: sqlx::migrate::MigrateError) -> Self {
        Self::Migration(value)
    }
}
