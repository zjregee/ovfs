use std::io;

pub type Inode = u64;
pub type Handle = u64;

#[derive(Clone)]
pub enum InodeType {
    DIR,
    FILE,
    Unknown,
}

#[derive(Clone)]
pub struct InodeData {
    pub path: String,
    pub stat: libc::stat64,
    pub inode_type: InodeType,
}

impl InodeData {
    pub fn new(inode_type: InodeType, path: String) -> InodeData {
        let mut stat: libc::stat64 = unsafe { std::mem::zeroed() };
        match inode_type {
            InodeType::DIR => stat.st_mode = libc::S_IFDIR,
            InodeType::FILE => stat.st_mode = libc::S_IFREG,
            _ => ()
        }
        InodeData {
            path,
            stat,
            inode_type,
        }
    }
}

pub struct Error(i32);

impl From<i32> for Error {
    fn from(value: i32) -> Error {
        Error(value)
    }
}

impl From<Error> for io::Error {
    fn from(err: Error) -> io::Error {
        unimplemented!()
    }
}

pub type Result<T> = std::result::Result<T, Error>;
