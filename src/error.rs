use std::fmt;

/// Handler error. `Fail` has already been reported to the host (via `ctx.fail`)
/// and is not reported again; `Other` is logged and sent to the host as an
/// error frame when it escapes a handler.
#[derive(Debug)]
pub enum Error {
    Fail(String),
    Other(String),
}

pub type Result<T = ()> = std::result::Result<T, Error>;

impl Error {
    pub fn msg(m: impl Into<String>) -> Self {
        Error::Other(m.into())
    }
    pub fn message(&self) -> &str {
        match self {
            Error::Fail(m) | Error::Other(m) => m,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

// Any std error converts with `?`. (Error deliberately does not implement
// std::error::Error itself, which keeps this blanket impl coherent.)
impl<E: std::error::Error> From<E> for Error {
    fn from(e: E) -> Self {
        Error::Other(e.to_string())
    }
}
