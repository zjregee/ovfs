use std::io;

use anyhow::Error as AnyError;
use snafu::prelude::Snafu;

#[derive(Debug, Snafu)]
#[non_exhaustive]
pub enum Error {
    #[snafu(display("Vhost user fs error: {}, source: {:?}", message, source))]
    VhostUserFsError {
        message: String,
        #[snafu(source(false))]
        source: Option<AnyError>,
    },
    #[snafu(display("Unexpected error: {}, source: {:?}", message, source))]
    Unexpected {
        message: String,
        #[snafu(source(false))]
        source: Option<AnyError>,
    },
}

impl From<Error> for io::Error {
    fn from(error: Error) -> io::Error {
        match error {
            Error::VhostUserFsError { message, source } => {
                let message = format!("Vhost user fs error: {}", message);
                match source {
                    Some(source) => io::Error::new(
                        io::ErrorKind::Other,
                        format!("{}, source: {:?}", message, source),
                    ),
                    None => io::Error::new(io::ErrorKind::Other, message),
                }
            }
            Error::Unexpected { message, source } => {
                let message = format!("Unexpected error: {}", message);
                match source {
                    Some(source) => io::Error::new(
                        io::ErrorKind::Other,
                        format!("{}, source: {:?}", message, source),
                    ),
                    None => io::Error::new(io::ErrorKind::Other, message),
                }
            }
        }
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

pub fn new_vhost_user_fs_error(message: &str, source: Option<AnyError>) -> Error {
    Error::VhostUserFsError {
        message: message.to_string(),
        source,
    }
}

pub fn new_unexpected_error(message: &str, source: Option<AnyError>) -> Error {
    Error::Unexpected {
        message: message.to_string(),
        source,
    }
}
