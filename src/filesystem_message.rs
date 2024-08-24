use vm_memory::ByteValued;

use crate::error::*;

#[non_exhaustive]
#[derive(Debug)]
pub enum Opcode {
    Lookup = 1,
    Forget = 2,
    Getattr = 3,
    Setattr = 4,
    Mkdir = 9,
    Unlink = 10,
    Rmdir = 11,
    Open = 14,
    Read = 15,
    Write = 16,
    Release = 18,
    Flush = 25,
    Init = 26,
    Opendir = 27,
    Readdir = 28,
    Releasedir = 29,
    Fsyncdir = 30,
    Create = 35,
    Destroy = 38,
}

impl TryFrom<u32> for Opcode {
    type Error = Error;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Opcode::Lookup),
            2 => Ok(Opcode::Forget),
            3 => Ok(Opcode::Getattr),
            4 => Ok(Opcode::Setattr),
            9 => Ok(Opcode::Mkdir),
            10 => Ok(Opcode::Unlink),
            11 => Ok(Opcode::Rmdir),
            14 => Ok(Opcode::Open),
            15 => Ok(Opcode::Read),
            16 => Ok(Opcode::Write),
            18 => Ok(Opcode::Release),
            25 => Ok(Opcode::Flush),
            26 => Ok(Opcode::Init),
            27 => Ok(Opcode::Opendir),
            28 => Ok(Opcode::Readdir),
            29 => Ok(Opcode::Releasedir),
            30 => Ok(Opcode::Fsyncdir),
            35 => Ok(Opcode::Create),
            38 => Ok(Opcode::Destroy),
            _ => Err(new_vhost_user_fs_error("failed to decode opcode", None)),
        }
    }
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct Attr {
    pub ino: u64,
    pub size: u64,
    pub blocks: u64,
    pub atime: u64,
    pub mtime: u64,
    pub ctime: u64,
    pub atimensec: u32,
    pub mtimensec: u32,
    pub ctimensec: u32,
    pub mode: u32,
    pub nlink: u32,
    pub uid: u32,
    pub gid: u32,
    pub rdev: u32,
    pub blksize: u32,
    pub flags: u32,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct InHeader {
    pub len: u32,
    pub opcode: u32,
    pub unique: u64,
    pub nodeid: u64,
    pub uid: u32,
    pub gid: u32,
    pub pid: u32,
    pub total_extlen: u16,
    pub padding: u16,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct OutHeader {
    pub len: u32,
    pub error: i32,
    pub unique: u64,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct InitIn {
    pub major: u32,
    pub minor: u32,
    pub max_readahead: u32,
    pub flags: u32,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct InitOut {
    pub major: u32,
    pub minor: u32,
    pub max_readahead: u32,
    pub flags: u32,
    pub max_background: u16,
    pub congestion_threshold: u16,
    pub max_write: u32,
    pub time_gran: u32,
    pub max_pages: u16,
    pub map_alignment: u16,
    pub flags2: u32,
    pub unused: [u32; 7],
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct AttrOut {
    pub attr_valid: u64,
    pub attr_valid_nsec: u32,
    pub dummy: u32,
    pub attr: Attr,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct EntryOut {
    pub nodeid: u64,
    pub generation: u64,
    pub entry_valid: u64,
    pub attr_valid: u64,
    pub entry_valid_nsec: u32,
    pub attr_valid_nsec: u32,
    pub attr: Attr,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct DirEntryOut {
    pub ino: u64,
    pub off: u64,
    pub namelen: u32,
    pub type_: u32,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct CreateIn {
    pub flags: u32,
    pub mode: u32,
    pub umask: u32,
    pub open_flags: u32,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct MkdirIn {
    pub mode: u32,
    pub umask: u32,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct OpenIn {
    pub flags: u32,
    pub open_flags: u32,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct OpenOut {
    pub fh: u64,
    pub open_flags: u32,
    pub padding: u32,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct WriteIn {
    pub fh: u64,
    pub offset: u64,
    pub size: u32,
    pub write_flags: u32,
    pub lock_owner: u64,
    pub flags: u32,
    pub padding: u32,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct WriteOut {
    pub size: u32,
    pub padding: u32,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct ReadIn {
    pub fh: u64,
    pub offset: u64,
    pub size: u32,
    pub read_flags: u32,
    pub lock_owner: u64,
    pub flags: u32,
    pub padding: u32,
}

unsafe impl ByteValued for InHeader {}
unsafe impl ByteValued for OutHeader {}
unsafe impl ByteValued for InitIn {}
unsafe impl ByteValued for InitOut {}
unsafe impl ByteValued for AttrOut {}
unsafe impl ByteValued for EntryOut {}
unsafe impl ByteValued for DirEntryOut {}
unsafe impl ByteValued for CreateIn {}
unsafe impl ByteValued for MkdirIn {}
unsafe impl ByteValued for OpenIn {}
unsafe impl ByteValued for OpenOut {}
unsafe impl ByteValued for WriteIn {}
unsafe impl ByteValued for WriteOut {}
unsafe impl ByteValued for ReadIn {}
