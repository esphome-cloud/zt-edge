use std::path::PathBuf;

#[derive(thiserror::Error, Debug)]
pub enum CodegenError {
    #[error("template error: {0}")]
    Template(#[from] tera::Error),
    #[error("I/O error writing {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
}
