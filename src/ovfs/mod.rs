mod types;
mod utils;
mod config;
mod consts;
mod ovfs_filesystem;

use types::*;
use crate::filesystem;

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
