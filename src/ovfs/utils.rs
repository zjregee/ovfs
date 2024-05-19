use std::io;

use super::*;

pub fn opendal_error2error(error: opendal::Error) -> io::Error {
    match error.kind() {
        opendal::ErrorKind::Unsupported => io::Error::from_raw_os_error(libc::EOPNOTSUPP),
        opendal::ErrorKind::NotFound => io::Error::from_raw_os_error(libc::ENOENT),
        _ => io::Error::from_raw_os_error(libc::ENOENT),
    }
}

pub fn opendal_metadata2stat64(path: &str, metadata: &opendal::Metadata) -> InodeData {
    let inode_type = match metadata.mode() {
        opendal::EntryMode::DIR => InodeType::DIR,
        opendal::EntryMode::FILE => InodeType::FILE,
        opendal::EntryMode::Unknown => InodeType::Unknown,
    };
    let inode_data = InodeData::new(inode_type, path);
    inode_data
}
