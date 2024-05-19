mod utils;
mod config;
mod consts;
mod ovfs_filesystem;

use crate::filesystem;

pub type Inode = u64;
pub type Handle = u64;

#[derive(Clone, PartialEq)]
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
    pub fn new(inode_type: InodeType, path: &str) -> InodeData {
        let mut stat: libc::stat64 = unsafe { std::mem::zeroed() };
        match inode_type {
            InodeType::DIR => stat.st_mode = libc::S_IFDIR,
            InodeType::FILE => stat.st_mode = libc::S_IFREG,
            _ => ()
        }
        InodeData {
            stat,
            inode_type,
            path: path.to_string(),
        }
    }
}

pub struct ReadDir {
    data: Vec<InodeData>,
    index: usize,
}

impl ReadDir {
    pub fn new(data: Vec<InodeData>) -> ReadDir {
        ReadDir {
            data,
            index: 0,
        }
    }
}

impl filesystem::DirectoryIterator for ReadDir {
    fn next(&mut self) -> Option<filesystem::DirEntry> {
        if self.index >= self.data.len() {
            None
        } else {
            let data = self.data[self.index].clone();
            Some(filesystem::DirEntry {
                ino: 0,
                type_: 0,
                offset: 0,
                name: data.path,
            })
        }
    }
}
