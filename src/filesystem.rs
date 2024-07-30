use std::collections::HashMap;
use std::ffi::CStr;
use std::io::Read;
use std::io::Write;
use std::mem::size_of;
use std::path::PathBuf;
use std::sync::RwLock;
use std::time::Duration;

use log::debug;
use opendal::{Buffer, Operator};
use sharded_slab::Slab;
use tokio::runtime::{Builder, Runtime};
use vm_memory::ByteValued;

use crate::buffer::BufferWrapper;
use crate::error::*;
use crate::filesystem_message::*;
use crate::util::{Reader, Writer};

const KERNEL_VERSION: u32 = 7;
const KERNEL_MINOR_VERSION: u32 = 38;
const MIN_KERNEL_MINOR_VERSION: u32 = 27;
const BUFFER_HEADER_SIZE: u32 = 4096;
const MAX_BUFFER_SIZE: u32 = 1 << 20;

enum FileType {
    Dir,
    File,
    Unknown,
}

#[derive(Clone)]
struct OpenedFile {
    path: String,
    stat: libc::stat64,
}

impl OpenedFile {
    fn new(file_type: FileType, path: &str) -> OpenedFile {
        let mut stat: libc::stat64 = unsafe { std::mem::zeroed() };
        stat.st_uid = 1000;
        stat.st_gid = 1000;
        match file_type {
            FileType::Dir => {
                stat.st_nlink = 2;
                stat.st_mode = libc::S_IFDIR | 0o755;
            }
            FileType::File => {
                stat.st_nlink = 1;
                stat.st_mode = libc::S_IFREG | 0o755;
            }
            FileType::Unknown => (),
        }
        OpenedFile {
            stat,
            path: path.to_string(),
        }
    }
}

fn opendal_error2error(error: opendal::Error) -> Error {
    match error.kind() {
        opendal::ErrorKind::Unsupported => {
            new_vhost_user_fs_error("unsupported error occurred in backend storage system", None)
        }
        opendal::ErrorKind::NotFound => {
            new_vhost_user_fs_error("notfound error occurred in backend storage system", None)
        }
        _ => new_vhost_user_fs_error("unexpected error occurred in backend storage system", None),
    }
}

fn opendal_metadata2opened_file(path: &str, metadata: &opendal::Metadata) -> OpenedFile {
    let file_type = match metadata.mode() {
        opendal::EntryMode::DIR => FileType::Dir,
        opendal::EntryMode::FILE => FileType::File,
        opendal::EntryMode::Unknown => FileType::Unknown,
    };
    OpenedFile::new(file_type, path)
}

pub struct Filesystem {
    rt: Runtime,
    core: Operator,
    opened_files: Slab<RwLock<OpenedFile>>,
    opened_files_map: RwLock<HashMap<String, usize>>,
}

impl Filesystem {
    pub fn new(core: Operator) -> Filesystem {
        let rt = Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .unwrap();

        Filesystem {
            rt,
            core,
            opened_files: Slab::default(),
            opened_files_map: RwLock::new(HashMap::default()),
        }
    }

    pub fn handle_message(&self, mut r: Reader, w: Writer) -> Result<usize> {
        let in_header: InHeader = r.read_obj().map_err(|e| {
            new_vhost_user_fs_error("failed to decode protocol messages", Some(e.into()))
        })?;
        if in_header.len > (MAX_BUFFER_SIZE + BUFFER_HEADER_SIZE) {
            return Filesystem::reply_error(in_header.unique, w);
        }
        if let Ok(opcode) = Opcode::try_from(in_header.opcode) {
            debug!(
                "[Filesystem] received request: opcode={:?} ({}), inode={}",
                opcode, in_header.opcode, in_header.nodeid
            );
            match opcode {
                Opcode::Init => self.init(in_header, r, w),
                Opcode::Lookup => self.lookup(in_header, r, w),
                Opcode::Getattr => self.getattr(in_header, r, w),
                Opcode::Setattr => self.setattr(in_header, r, w),
                Opcode::Create => self.create(in_header, r, w),
                Opcode::Unlink => self.unlink(in_header, r, w),
                Opcode::Mkdir => self.mkdir(in_header, r, w),
                Opcode::Rmdir => self.rmdir(in_header, r, w),
                Opcode::Open => self.open(in_header, r, w),
                Opcode::Opendir => self.opendir(in_header, r, w),
                Opcode::Read => self.read(in_header, r, w),
                Opcode::Write => self.write(in_header, r, w),
                Opcode::Destroy => self.destory(),
                Opcode::Access => self.access(in_header, r, w),
                Opcode::Forget => self.forget(in_header, r),
                Opcode::Release => self.release(in_header, r, w),
                Opcode::Releasedir => self.releasedir(in_header, r, w),
                Opcode::Flush => self.flush(in_header, r, w),
                Opcode::Fsyncdir => self.fsyncdir(in_header, r, w),
                Opcode::Getxattr => Filesystem::reply_unimplemented(in_header.unique, w),
            }
        } else {
            debug!(
                "[Filesystem] received unknown request: opcode={}, inode={}",
                in_header.opcode, in_header.nodeid
            );
            Filesystem::reply_error(in_header.unique, w)
        }
    }
}

impl Filesystem {
    fn init(&self, in_header: InHeader, mut r: Reader, w: Writer) -> Result<usize> {
        let InitIn { major, minor, .. } = r.read_obj().map_err(|e| {
            new_vhost_user_fs_error("failed to decode protocol messages", Some(e.into()))
        })?;

        if major != KERNEL_VERSION || minor < MIN_KERNEL_MINOR_VERSION {
            return Filesystem::reply_error(in_header.unique, w);
        }

        let file = OpenedFile::new(FileType::Dir, "/");
        let file_key = self.insert_opened_inode(file.clone());
        self.insert_opened("/", file_key);
        let file_key = self.insert_opened_inode(file.clone());
        self.insert_opened("/", file_key);

        let out = InitOut {
            major: KERNEL_VERSION,
            minor: KERNEL_MINOR_VERSION,
            max_write: MAX_BUFFER_SIZE,
            ..Default::default()
        };
        Filesystem::reply_ok(Some(out), None, in_header.unique, w)
    }

    fn destory(&self) -> Result<usize> {
        Ok(0)
    }

    fn access(&self, _in_header: InHeader, _r: Reader, _w: Writer) -> Result<usize> {
        Ok(0)
    }

    fn forget(&self, _in_header: InHeader, _r: Reader) -> Result<usize> {
        Ok(0)
    }

    fn release(&self, _in_header: InHeader, _r: Reader, _w: Writer) -> Result<usize> {
        Ok(0)
    }

    fn releasedir(&self, _in_header: InHeader, _r: Reader, _w: Writer) -> Result<usize> {
        Ok(0)
    }

    fn flush(&self, _in_header: InHeader, _r: Reader, _w: Writer) -> Result<usize> {
        Ok(0)
    }

    fn fsyncdir(&self, _in_header: InHeader, _r: Reader, _w: Writer) -> Result<usize> {
        Ok(0)
    }

    fn lookup(&self, in_header: InHeader, mut r: Reader, w: Writer) -> Result<usize> {
        let name_len = in_header.len as usize - size_of::<InHeader>();
        let mut buf = vec![0u8; name_len];
        r.read_exact(&mut buf).map_err(|e| {
            new_unexpected_error("failed to decode protocol messages", Some(e.into()))
        })?;
        let name = match Filesystem::bytes_to_str(buf.as_ref()) {
            Ok(name) => name,
            Err(_) => return Filesystem::reply_error(in_header.unique, w),
        };

        debug!(
            "[Filesystem] lookup: parent_key={} name={}",
            in_header.nodeid, name
        );

        let parent_file = self.get_opened_inode(in_header.nodeid as usize);
        let parent_path = match parent_file {
            Ok(parent_file) => parent_file.path,
            Err(_) => return Filesystem::reply_error(in_header.unique, w),
        };

        let path = PathBuf::from(parent_path)
            .join(name)
            .to_string_lossy()
            .to_string();

        if let Some(file_key) = self.get_opened(&path) {
            let mut file = self.get_opened_inode(file_key).unwrap();
            file.stat.st_ino = file_key as u64;
            file.stat.st_size = 8;
            let out = EntryOut {
                nodeid: file_key as u64,
                generation: 0,
                entry_valid: Duration::from_secs(5).as_secs(),
                attr_valid: Duration::from_secs(5).as_secs(),
                entry_valid_nsec: Duration::from_secs(5).subsec_nanos(),
                attr_valid_nsec: Duration::from_secs(5).subsec_nanos(),
                attr: file.stat.into(),
            };
            return Filesystem::reply_ok(Some(out), None, in_header.unique, w);
        }

        let mut file = match self.rt.block_on(self.do_get_stat(&path)) {
            Ok(stat) => stat,
            Err(_) => return Filesystem::reply_error(in_header.unique, w),
        };
        let file_key = self.insert_opened_inode(file.clone());
        self.insert_opened(&path, file_key);
        file.stat.st_ino = file_key as u64;
        file.stat.st_size = 8;

        let out = EntryOut {
            nodeid: file_key as u64,
            generation: 0,
            entry_valid: Duration::from_secs(5).as_secs(),
            attr_valid: Duration::from_secs(5).as_secs(),
            entry_valid_nsec: Duration::from_secs(5).subsec_nanos(),
            attr_valid_nsec: Duration::from_secs(5).subsec_nanos(),
            attr: file.stat.into(),
        };
        return Filesystem::reply_ok(Some(out), None, in_header.unique, w);
    }

    fn getattr(&self, in_header: InHeader, _r: Reader, w: Writer) -> Result<usize> {
        debug!("[Filesystem] getattr: key={}", in_header.nodeid);

        let file = self.get_opened_inode(in_header.nodeid as usize);
        let mut stat = match file {
            Ok(file) => file.stat,
            Err(_) => return Filesystem::reply_error(in_header.unique, w),
        };
        stat.st_ino = in_header.nodeid;
        stat.st_size = 8;

        let out = AttrOut {
            attr_valid: Duration::from_secs(5).as_secs(),
            attr_valid_nsec: Duration::from_secs(5).subsec_nanos(),
            dummy: 0,
            attr: stat.into(),
        };
        Filesystem::reply_ok(Some(out), None, in_header.unique, w)
    }

    fn setattr(&self, in_header: InHeader, _r: Reader, w: Writer) -> Result<usize> {
        debug!("[Filesystem] setattr: key={}", in_header.nodeid);

        let file = self.get_opened_inode(in_header.nodeid as usize);
        let mut stat = match file {
            Ok(file) => file.stat,
            Err(_) => return Filesystem::reply_error(in_header.unique, w),
        };
        stat.st_ino = in_header.nodeid;

        let out = AttrOut {
            attr_valid: Duration::from_secs(5).as_secs(),
            attr_valid_nsec: Duration::from_secs(5).subsec_nanos(),
            dummy: 0,
            attr: stat.into(),
        };
        Filesystem::reply_ok(Some(out), None, in_header.unique, w)
    }

    fn create(&self, in_header: InHeader, mut r: Reader, w: Writer) -> Result<usize> {
        let CreateIn { .. } = r.read_obj().map_err(|e| {
            new_vhost_user_fs_error("failed to decode protocol messages", Some(e.into()))
        })?;

        let name_len = in_header.len as usize - size_of::<InHeader>() - size_of::<CreateIn>();
        let mut buf = vec![0u8; name_len];
        r.read_exact(&mut buf).map_err(|e| {
            new_unexpected_error("failed to decode protocol messages", Some(e.into()))
        })?;
        let mut components = buf.split_inclusive(|c| *c == b'\0');
        let buf = components.next().ok_or(new_unexpected_error(
            "one or more parameters are missing",
            None,
        ))?;
        let name = match Filesystem::bytes_to_str(buf.as_ref()) {
            Ok(name) => name,
            Err(_) => return Filesystem::reply_error(in_header.unique, w),
        };

        debug!(
            "[Filesystem] create: parent_key={} name={}",
            in_header.nodeid, name
        );

        let parent_file = self.get_opened_inode(in_header.nodeid as usize);
        let parent_path = match parent_file {
            Ok(parent_file) => parent_file.path,
            Err(_) => return Filesystem::reply_error(in_header.unique, w),
        };

        let path = PathBuf::from(parent_path)
            .join(name)
            .to_string_lossy()
            .to_string();

        if self.rt.block_on(self.do_create_file(&path)).is_err() {
            return Filesystem::reply_error(in_header.unique, w);
        }

        let file = OpenedFile::new(FileType::File, &path);
        let file_key = self.insert_opened_inode(file.clone());
        self.insert_opened(&path, file_key);

        let mut stat = file.stat;
        stat.st_ino = file_key as u64;

        let entry_out = EntryOut {
            nodeid: file_key as u64,
            generation: 0,
            entry_valid: Duration::from_secs(5).as_secs(),
            attr_valid: Duration::from_secs(5).as_secs(),
            entry_valid_nsec: Duration::from_secs(5).subsec_nanos(),
            attr_valid_nsec: Duration::from_secs(5).subsec_nanos(),
            attr: stat.into(),
        };
        let open_out = OpenOut {
            ..Default::default()
        };
        return Filesystem::reply_ok(
            Some(entry_out),
            Some(open_out.as_slice()),
            in_header.unique,
            w,
        );
    }

    fn unlink(&self, in_header: InHeader, mut r: Reader, w: Writer) -> Result<usize> {
        let name_len = in_header.len as usize - size_of::<InHeader>();
        let mut buf = vec![0u8; name_len];
        r.read_exact(&mut buf).map_err(|e| {
            new_unexpected_error("failed to decode protocol messages", Some(e.into()))
        })?;
        let mut components = buf.split_inclusive(|c| *c == b'\0');
        let buf = components.next().ok_or(new_unexpected_error(
            "one or more parameters are missing",
            None,
        ))?;
        let name = match Filesystem::bytes_to_str(buf.as_ref()) {
            Ok(name) => name,
            Err(_) => return Filesystem::reply_error(in_header.unique, w),
        };

        debug!(
            "[Filesystem] unlink: parent_key={} name={}",
            in_header.nodeid, name
        );

        let parent_file = self.get_opened_inode(in_header.nodeid as usize);
        let parent_path = match parent_file {
            Ok(parent_file) => parent_file.path,
            Err(_) => return Filesystem::reply_error(in_header.unique, w),
        };

        let path = PathBuf::from(parent_path)
            .join(name)
            .to_string_lossy()
            .to_string();

        if self.rt.block_on(self.do_delete(&path)).is_err() {
            return Filesystem::reply_error(in_header.unique, w);
        }

        Ok(0)
    }

    fn mkdir(&self, in_header: InHeader, mut r: Reader, w: Writer) -> Result<usize> {
        let MkdirIn { .. } = r.read_obj().map_err(|e| {
            new_vhost_user_fs_error("failed to decode protocol messages", Some(e.into()))
        })?;

        let name_len = in_header.len as usize - size_of::<InHeader>() - size_of::<CreateIn>();
        let mut buf = vec![0u8; name_len];
        r.read_exact(&mut buf).map_err(|e| {
            new_unexpected_error("failed to decode protocol messages", Some(e.into()))
        })?;
        let mut components = buf.split_inclusive(|c| *c == b'\0');
        let buf = components.next().ok_or(new_unexpected_error(
            "one or more parameters are missing",
            None,
        ))?;
        let name = match Filesystem::bytes_to_str(buf.as_ref()) {
            Ok(name) => name,
            Err(_) => return Filesystem::reply_error(in_header.unique, w),
        };

        debug!(
            "[Filesystem] mkdir: parent_key={} name={}",
            in_header.nodeid, name
        );

        let parent_file = self.get_opened_inode(in_header.nodeid as usize);
        let parent_path = match parent_file {
            Ok(parent_file) => parent_file.path,
            Err(_) => return Filesystem::reply_error(in_header.unique, w),
        };

        let path = PathBuf::from(parent_path)
            .join(name)
            .to_string_lossy()
            .to_string();

        if self.rt.block_on(self.do_create_dir(&path)).is_err() {
            return Filesystem::reply_error(in_header.unique, w);
        }

        let file = OpenedFile::new(FileType::Dir, &path);
        let file_key = self.insert_opened_inode(file.clone());
        self.insert_opened(&path, file_key);

        let mut stat = file.stat;
        stat.st_ino = file_key as u64;

        let entry_out = EntryOut {
            nodeid: file_key as u64,
            generation: 0,
            entry_valid: Duration::from_secs(5).as_secs(),
            attr_valid: Duration::from_secs(5).as_secs(),
            entry_valid_nsec: Duration::from_secs(5).subsec_nanos(),
            attr_valid_nsec: Duration::from_secs(5).subsec_nanos(),
            attr: stat.into(),
        };
        return Filesystem::reply_ok(Some(entry_out), None, in_header.unique, w);
    }

    fn rmdir(&self, in_header: InHeader, mut r: Reader, w: Writer) -> Result<usize> {
        let name_len = in_header.len as usize - size_of::<InHeader>();
        let mut buf = vec![0u8; name_len];
        r.read_exact(&mut buf).map_err(|e| {
            new_unexpected_error("failed to decode protocol messages", Some(e.into()))
        })?;
        let mut components = buf.split_inclusive(|c| *c == b'\0');
        let buf = components.next().ok_or(new_unexpected_error(
            "one or more parameters are missing",
            None,
        ))?;
        let name = match Filesystem::bytes_to_str(buf.as_ref()) {
            Ok(name) => name,
            Err(_) => return Filesystem::reply_error(in_header.unique, w),
        };

        debug!(
            "[Filesystem] rmdir: parent_key={} name={}",
            in_header.nodeid, name
        );

        let parent_file = self.get_opened_inode(in_header.nodeid as usize);
        let parent_path = match parent_file {
            Ok(parent_file) => parent_file.path,
            Err(_) => return Filesystem::reply_error(in_header.unique, w),
        };

        let path = PathBuf::from(parent_path)
            .join(name)
            .to_string_lossy()
            .to_string();

        if self.rt.block_on(self.do_delete(&path)).is_err() {
            return Filesystem::reply_error(in_header.unique, w);
        }

        Ok(0)
    }

    fn open(&self, in_header: InHeader, _r: Reader, w: Writer) -> Result<usize> {
        debug!("[Filesystem] open: key={}", in_header.nodeid);

        let file = self.get_opened_inode(in_header.nodeid as usize);
        let mut stat = match file {
            Ok(file) => file.stat,
            Err(_) => return Filesystem::reply_error(in_header.unique, w),
        };
        stat.st_ino = in_header.nodeid;

        let out = OpenOut {
            ..Default::default()
        };
        Filesystem::reply_ok(Some(out), None, in_header.unique, w)
    }

    fn opendir(&self, in_header: InHeader, _r: Reader, w: Writer) -> Result<usize> {
        debug!("[Filesystem] opendir: key={}", in_header.nodeid);

        let file = self.get_opened_inode(in_header.nodeid as usize);
        let mut stat = match file {
            Ok(file) => file.stat,
            Err(_) => return Filesystem::reply_error(in_header.unique, w),
        };
        stat.st_ino = in_header.nodeid;

        let out = OpenOut {
            ..Default::default()
        };
        Filesystem::reply_ok(Some(out), None, in_header.unique, w)
    }

    fn read(&self, in_header: InHeader, mut r: Reader, mut w: Writer) -> Result<usize> {
        debug!("[Filesystem] read: key={}", in_header.nodeid);

        let file = self.get_opened_inode(in_header.nodeid as usize);
        let path = match file {
            Ok(file) => file.path,
            Err(_) => return Filesystem::reply_error(in_header.unique, w),
        };

        let ReadIn { offset, size, .. } = r.read_obj().map_err(|e| {
            new_vhost_user_fs_error("failed to decode protocol messages", Some(e.into()))
        })?;

        debug!(
            "[Filesystem] read: key={} offset={} size={}",
            in_header.nodeid, offset, size
        );

        let data = match self.rt.block_on(self.do_read(&path)) {
            Ok(data) => data,
            Err(_) => return Filesystem::reply_error(in_header.unique, w),
        };
        let len = data.len();
        let buffer = BufferWrapper::new(data);

        debug!(
            "[Filesystem] read: key={} offset={} size={} len={} buffer={:?}",
            in_header.nodeid,
            offset,
            size,
            len,
            buffer.get_buffer(),
        );

        let mut data_writer = w.split_at(size_of::<OutHeader>()).unwrap();
        data_writer.write_from_at(&buffer, len).map_err(|e| {
            new_vhost_user_fs_error("failed to encode protocol messages", Some(e.into()))
        })?;

        let out = OutHeader {
            len: (size_of::<OutHeader>() + len) as u32,
            error: 0,
            unique: in_header.unique,
        };
        w.write_all(out.as_slice()).map_err(|e| {
            new_vhost_user_fs_error("failed to encode protocol messages", Some(e.into()))
        })?;
        Ok(out.len as usize)
    }

    fn write(&self, in_header: InHeader, mut r: Reader, w: Writer) -> Result<usize> {
        debug!("[Filesystem] write: key={}", in_header.nodeid);

        let file = self.get_opened_inode(in_header.nodeid as usize);
        let path = match file {
            Ok(file) => file.path,
            Err(_) => return Filesystem::reply_error(in_header.unique, w),
        };

        let WriteIn { size, .. } = r.read_obj().map_err(|e| {
            new_vhost_user_fs_error("failed to decode protocol messages", Some(e.into()))
        })?;

        let buffer = BufferWrapper::new(Buffer::new());
        r.read_to_at(&buffer, size as usize).map_err(|e| {
            new_vhost_user_fs_error("failed to decode protocol messages", Some(e.into()))
        })?;
        let buffer = buffer.get_buffer();

        if self.rt.block_on(self.do_write(&path, buffer)).is_err() {
            return Filesystem::reply_error(in_header.unique, w);
        }

        let out = WriteOut {
            size,
            ..Default::default()
        };
        Filesystem::reply_ok(Some(out), None, in_header.unique, w)
    }
}

impl Filesystem {
    fn reply_ok<T: ByteValued>(
        out: Option<T>,
        data: Option<&[u8]>,
        unique: u64,
        mut w: Writer,
    ) -> Result<usize> {
        let mut len = size_of::<OutHeader>();
        if out.is_some() {
            len += size_of::<T>();
        }
        if let Some(data) = data {
            len += data.len();
        }
        let header = OutHeader {
            unique,
            error: 0,
            len: len as u32,
        };
        w.write_all(header.as_slice()).map_err(|e| {
            new_vhost_user_fs_error("failed to encode protocol messages", Some(e.into()))
        })?;
        if let Some(out) = out {
            w.write_all(out.as_slice()).map_err(|e| {
                new_vhost_user_fs_error("failed to encode protocol messages", Some(e.into()))
            })?;
        }
        if let Some(data) = data {
            w.write_all(data).map_err(|e| {
                new_vhost_user_fs_error("failed to encode protocol messages", Some(e.into()))
            })?;
        }
        Ok(w.bytes_written())
    }

    fn reply_error(unique: u64, mut w: Writer) -> Result<usize> {
        let header = OutHeader {
            unique,
            error: -libc::ENOENT,
            len: size_of::<OutHeader>() as u32,
        };
        w.write_all(header.as_slice()).map_err(|e| {
            new_vhost_user_fs_error("failed to encode protocol messages", Some(e.into()))
        })?;
        Ok(w.bytes_written())
    }

    fn reply_unimplemented(unique: u64, mut w: Writer) -> Result<usize> {
        let header = OutHeader {
            unique,
            error: -libc::ENOSYS,
            len: size_of::<OutHeader>() as u32,
        };
        w.write_all(header.as_slice()).map_err(|e| {
            new_vhost_user_fs_error("failed to encode protocol messages", Some(e.into()))
        })?;
        Ok(w.bytes_written())
    }

    fn bytes_to_str(buf: &[u8]) -> Result<&str> {
        return Filesystem::bytes_to_cstr(buf).map(|cstr| cstr.to_str().unwrap());
    }

    fn bytes_to_cstr(buf: &[u8]) -> Result<&CStr> {
        CStr::from_bytes_with_nul(buf).map_err(|e| {
            new_vhost_user_fs_error("failed to decode protocol messages", Some(e.into()))
        })
    }
}

impl Filesystem {
    fn get_opened(&self, path: &str) -> Option<usize> {
        let map = self.opened_files_map.read().unwrap();
        map.get(path).copied()
    }

    fn insert_opened(&self, path: &str, key: usize) {
        let mut map = self.opened_files_map.write().unwrap();
        map.insert(path.to_string(), key);
    }

    fn get_opened_inode(&self, key: usize) -> Result<OpenedFile> {
        if let Some(opened_inode) = self.opened_files.get(key) {
            let inode_data = opened_inode.read().unwrap().clone();
            Ok(inode_data)
        } else {
            Err(new_unexpected_error("invalid file", None))
        }
    }

    fn insert_opened_inode(&self, value: OpenedFile) -> usize {
        self.opened_files.insert(RwLock::new(value)).unwrap()
    }
}

impl Filesystem {
    async fn do_get_stat(&self, path: &str) -> Result<OpenedFile> {
        let metadata = self.core.stat(path).await.map_err(opendal_error2error)?;
        let attr = opendal_metadata2opened_file(path, &metadata);

        Ok(attr)
    }

    async fn do_create_file(&self, path: &str) -> Result<()> {
        self.core
            .write(path, Buffer::new())
            .await
            .map_err(opendal_error2error)?;

        Ok(())
    }

    async fn do_create_dir(&self, path: &str) -> Result<()> {
        self.core
            .create_dir(path)
            .await
            .map_err(opendal_error2error)?;

        Ok(())
    }

    async fn do_delete(&self, path: &str) -> Result<()> {
        self.core.delete(path).await.map_err(opendal_error2error)?;

        Ok(())
    }

    async fn do_read(&self, path: &str) -> Result<Buffer> {
        let data = self.core.read(path).await.map_err(opendal_error2error)?;

        Ok(data)
    }

    async fn do_write(&self, path: &str, data: Buffer) -> Result<()> {
        self.core
            .write(path, data)
            .await
            .map_err(opendal_error2error)?;

        Ok(())
    }
}
