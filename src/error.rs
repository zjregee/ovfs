use std::ffi::CStr;
use std::io;

use anyhow::Error as AnyError;
use log::debug;
use opendal::ErrorKind;
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

impl From<opendal::Error> for Error {
    fn from(error: opendal::Error) -> Error {
        debug!("opendal error occurred: {:?}", error);
        match error.kind() {
            ErrorKind::Unsupported => Error::from(libc::EOPNOTSUPP),
            ErrorKind::IsADirectory => Error::from(libc::EISDIR),
            ErrorKind::NotFound => Error::from(libc::ENOENT),
            ErrorKind::PermissionDenied => Error::from(libc::EACCES),
            ErrorKind::AlreadyExists => Error::from(libc::EEXIST),
            ErrorKind::NotADirectory => Error::from(libc::ENOTDIR),
            ErrorKind::RangeNotSatisfied => Error::from(libc::EINVAL),
            ErrorKind::RateLimited => Error::from(libc::EBUSY),
            _ => Error::from(libc::ENOENT),
        }
    }
}

impl From<libc::c_int> for Error {
    fn from(errno: libc::c_int) -> Error {
        let err_str = unsafe { libc::strerror(errno) };
        let message = if err_str.is_null() {
            format!("errno: {}", errno)
        } else {
            let c_str = unsafe { CStr::from_ptr(err_str) };
            c_str.to_string_lossy().into_owned()
        };
        Error::VhostUserFsError {
            message,
            source: None,
        }
    }
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
