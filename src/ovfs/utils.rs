use std::io;
use std::time::SystemTime;

use super::*;

pub fn opendal_error2error(error: opendal::Error) -> io::Error {
    match error.kind() {
        opendal::ErrorKind::Unsupported => io::Error::from_raw_os_error(libc::EOPNOTSUPP),
        opendal::ErrorKind::NotFound => io::Error::from_raw_os_error(libc::ENOENT),
        _ => io::Error::from_raw_os_error(libc::ENOENT),
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
