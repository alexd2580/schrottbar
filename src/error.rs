use std::fmt::Display;

#[derive(Debug)]
pub enum Error {
    Local(String),
    Io(std::io::Error),
    Tokio(tokio::task::JoinError),
    Reqwest(reqwest::Error),
    ChronoParse(chrono::ParseError),
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Local(message) => write!(f, "{message}"),
            Error::Io(err) => write!(f, "Io Error\n{err:?}"),
            Error::Tokio(err) => write!(f, "Tokio Error\n{err:}"),
            Error::Reqwest(err) => write!(f, "Reqwest Error\n{err:}"),
            Error::ChronoParse(err) => write!(f, "Chrono Parse Error\n{err:?}"),
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}
impl From<tokio::task::JoinError> for Error {
    fn from(err: tokio::task::JoinError) -> Self {
        Self::Tokio(err)
    }
}
impl From<reqwest::Error> for Error {
    fn from(err: reqwest::Error) -> Self {
        Self::Reqwest(err)
    }
}
impl From<chrono::ParseError> for Error {
    fn from(value: chrono::ParseError) -> Self {
        Self::ChronoParse(value)
    }
}
