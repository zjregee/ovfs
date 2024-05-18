use std::time::SystemTime;

use super::types::*;

pub fn opendal_error2error(error: opendal::Error) -> Error {
    match error.kind() {
        opendal::ErrorKind::Unsupported => Error::from(libc::EOPNOTSUPP),
        opendal::ErrorKind::NotFound => Error::from(libc::ENOENT),
        _ => Error::from(libc::ENOENT),
    }
}

pub fn opendal_metadata2stat64(metadata: &opendal::Metadata, atime: SystemTime) -> InodeData {
    let _ = metadata.last_modified().map(|t| t.into()).unwrap_or(atime);
    let _ = opendal_enrty_mode2inode_type(metadata.mode());
    unimplemented!()
}

pub fn opendal_enrty_mode2inode_type(entry_mode: opendal::EntryMode) -> InodeType {
    match entry_mode {
        opendal::EntryMode::DIR => InodeType::DIR,
        opendal::EntryMode::FILE => InodeType::FILE,
        opendal::EntryMode::Unknown => InodeType::Unknown,
    }
}
