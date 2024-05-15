pub mod utils;
pub mod config;
pub mod ovfs_filesystem;

#[derive(Clone)]
enum InodeType {
    DIR,
    FILE,
    Unknown,
}

struct InodeKey(usize);

impl InodeKey {
    fn to_inode(self) -> u64 {
        self.0 as u64 + 1
    }
}

#[derive(Clone)]
struct InodeData {
    mode: InodeType,
    path: String,
    stat: libc::stat64,
}

struct Error(i32);

impl From<i32> for Error {
    fn from(value: i32) -> Error {
        Error(value)
    }
}

type Result<T> = std::result::Result<T, Error>;
