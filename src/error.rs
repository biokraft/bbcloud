use thiserror::Error;

#[derive(Debug, Error)]
pub enum BbError {
    #[error("not authenticated — run `bb auth login`")]
    Auth,
    #[error("not found")]
    NotFound,
    #[error("bitbucket api error {status}: {message}")]
    Api { status: u16, message: String },
    #[error("config error: {0}")]
    Config(String),
    #[error("git error: {0}")]
    Git(String),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl BbError {
    pub fn exit_code(&self) -> i32 {
        match self {
            BbError::Auth => 2,
            BbError::NotFound => 3,
            _ => 1,
        }
    }
}

pub type Result<T> = std::result::Result<T, BbError>;
