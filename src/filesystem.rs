use std::collections::HashMap;
use std::ffi::CStr;
use std::io::Read;
use std::io::Write;
use std::mem::size_of;
use std::sync::Mutex;
use std::time::Duration;

use log::debug;
use opendal::Buffer;
use opendal::Operator;
use sharded_slab::Slab;
use tokio::runtime::Builder;
use tokio::runtime::Runtime;
use vm_memory::ByteValued;

use crate::buffer::BufferWrapper;
use crate::error::*;
use crate::filesystem_message::*;
use crate::util::Reader;
use crate::util::Writer;

const KERNEL_VERSION: u32 = 7;
const KERNEL_MINOR_VERSION: u32 = 38;
const MIN_KERNEL_MINOR_VERSION: u32 = 27;
const BUFFER_HEADER_SIZE: u32 = 4096;
const MAX_BUFFER_SIZE: u32 = 1 << 20;
const DEFAULT_TTL: Duration = Duration::from_secs(1);
const DEFAULT_GID: u32 = 1000;
const DEFAULT_UID: u32 = 1000;
const DEFAULT_DIR_NLINK: u32 = 2;
const DEFAULT_FILE_NLINK: u32 = 1;
const DEFAULT_MODE: u32 = 0o755;
const DEFAULT_ROOT_DIR_INODE: u64 = 1;
const DEAFULT_DIR_TYPE_IN_DIR_ENTRY: u32 = 4;
const DEAFULT_FILE_TYPE_IN_DIR_ENTRY: u32 = 8;
const DIRENT_PADDING: [u8; 8] = [0; 8];

enum FileType {
    Dir,
    File,
}

struct InnerWriter {
    writer: opendal::Writer,
    written: u64,
}

#[derive(Clone)]
struct OpenedFile {
    path: String,
    metadata: Attr,
}

impl OpenedFile {
    fn new(file_type: FileType, path: &str) -> OpenedFile {
        let mut attr: Attr = unsafe { std::mem::zeroed() };
        attr.uid = DEFAULT_UID;
        attr.gid = DEFAULT_GID;
        match file_type {
            FileType::Dir => {
                attr.nlink = DEFAULT_DIR_NLINK;
                attr.mode = libc::S_IFDIR | DEFAULT_MODE;
            }
            FileType::File => {
                attr.nlink = DEFAULT_FILE_NLINK;
                attr.mode = libc::S_IFREG | DEFAULT_MODE;
            }
        }
        OpenedFile {
            path: path.to_string(),
            metadata: attr,
        }
    }
}

struct DirEntry {
    ino: u64,
    off: u64,
    type_: u32,
    name: String,
}

pub struct Filesystem {
    rt: Runtime,
    core: Operator,
    opened_files: Slab<OpenedFile>,
    opened_files_map: Mutex<HashMap<String, u64>>,
    opened_files_writer: Mutex<HashMap<String, InnerWriter>>,
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
            opened_files: Slab::new(),
            opened_files_map: Mutex::new(HashMap::new()),
            opened_files_writer: Mutex::new(HashMap::new()),
        }
    }

    pub fn handle_message(&self, mut r: Reader, w: Writer) -> Result<usize> {
        let in_header: InHeader = r.read_obj().map_err(|_| Error::from(libc::EIO))?;
        if in_header.len > (MAX_BUFFER_SIZE + BUFFER_HEADER_SIZE) {
            return Filesystem::reply_error(in_header.unique, w, libc::EIO);
        }
        if let Ok(opcode) = Opcode::try_from(in_header.opcode) {
            debug!(
                "received request: opcode={}, inode={}",
                in_header.opcode, in_header.nodeid
            );
            match opcode {
                Opcode::Init => self.init(in_header, r, w),
                Opcode::Destroy => self.destory(),
                Opcode::Forget => self.forget(in_header),
                Opcode::Lookup => self.lookup(in_header, r, w),
                Opcode::Getattr => self.getattr(in_header, r, w),
                Opcode::Setattr => self.setattr(in_header, r, w),
                Opcode::Create => self.create(in_header, r, w),
                Opcode::Unlink => self.unlink(in_header, r, w),
                Opcode::Release => self.release(in_header, r, w),
                Opcode::Flush => self.flush(in_header, r, w),
                Opcode::Open => self.open(in_header, r, w),
                Opcode::Read => self.read(in_header, r, w),
                Opcode::Write => self.write(in_header, r, w),
                Opcode::Mkdir => self.mkdir(in_header, r, w),
                Opcode::Rmdir => self.rmdir(in_header, r, w),
                Opcode::Releasedir => self.releasedir(in_header, r, w),
                Opcode::Fsyncdir => self.fsyncdir(in_header, r, w),
                Opcode::Opendir => self.opendir(in_header, r, w),
                Opcode::Readdir => self.readdir(in_header, r, w),
            }
        } else {
            debug!(
                "received unknown request: opcode={}, inode={}",
                in_header.opcode, in_header.nodeid
            );
            Filesystem::reply_error(in_header.unique, w, libc::ENOSYS)
        }
    }
}

impl Filesystem {
    fn init(&self, in_header: InHeader, mut r: Reader, w: Writer) -> Result<usize> {
        let InitIn { major, minor, .. } = r.read_obj().map_err(|_| Error::from(libc::EIO))?;

        if major != KERNEL_VERSION || minor < MIN_KERNEL_MINOR_VERSION {
            return Filesystem::reply_error(in_header.unique, w, libc::EIO);
        }

        let mut attr = OpenedFile::new(FileType::Dir, "/");
        attr.metadata.ino = DEFAULT_ROOT_DIR_INODE;
        self.opened_files
            .insert(attr.clone())
            .expect("failed to allocate inode");
        self.opened_files
            .insert(attr.clone())
            .expect("failed to allocate inode");
        let mut opened_files_map = self.opened_files_map.lock().unwrap();
        opened_files_map.insert("/".to_string(), DEFAULT_ROOT_DIR_INODE);

        let out = InitOut {
            major: KERNEL_VERSION,
            minor: KERNEL_MINOR_VERSION,
            max_write: MAX_BUFFER_SIZE,
            ..Default::default()
        };
        Filesystem::reply_ok(Some(out), None, in_header.unique, w)
    }

    fn destory(&self) -> Result<usize> {
        // do nothing for destroy.
        Ok(0)
    }

    fn forget(&self, _in_header: InHeader) -> Result<usize> {
        // do nothing for forget.
        Ok(0)
    }

    fn lookup(&self, in_header: InHeader, mut r: Reader, w: Writer) -> Result<usize> {
        let name_len = in_header.len as usize - size_of::<InHeader>();
        let mut buf = vec![0; name_len];
        r.read_exact(&mut buf).map_err(|_| Error::from(libc::EIO))?;
        let name = match Filesystem::bytes_to_str(buf.as_ref()) {
            Ok(name) => name,
            Err(_) => return Filesystem::reply_error(in_header.unique, w, libc::EIO),
        };

        debug!("lookup: parent inode={} name={}", in_header.nodeid, name);

        let parent_path = match self
            .opened_files
            .get(in_header.nodeid as usize)
            .map(|f| f.path.clone())
        {
            Some(path) => path,
            None => return Filesystem::reply_error(in_header.unique, w, libc::ENOENT),
        };

        let path = format!("{}/{}", parent_path, name);
        let metadata = match self.rt.block_on(self.do_get_metadata(&path)) {
            Ok(metadata) => metadata,
            Err(_) => return Filesystem::reply_error(in_header.unique, w, libc::ENOENT),
        };

        let out = EntryOut {
            nodeid: metadata.metadata.ino,
            entry_valid: DEFAULT_TTL.as_secs(),
            attr_valid: DEFAULT_TTL.as_secs(),
            entry_valid_nsec: DEFAULT_TTL.subsec_nanos(),
            attr_valid_nsec: DEFAULT_TTL.subsec_nanos(),
            attr: metadata.metadata,
            ..Default::default()
        };
        Filesystem::reply_ok(Some(out), None, in_header.unique, w)
    }

    fn getattr(&self, in_header: InHeader, _r: Reader, w: Writer) -> Result<usize> {
        debug!("getattr: inode={}", in_header.nodeid);

        let path = match self
            .opened_files
            .get(in_header.nodeid as usize)
            .map(|f| f.path.clone())
        {
            Some(path) => path,
            None => return Filesystem::reply_error(in_header.unique, w, libc::ENOENT),
        };

        let metadata = match self.rt.block_on(self.do_get_metadata(&path)) {
            Ok(metadata) => metadata,
            Err(_) => return Filesystem::reply_error(in_header.unique, w, libc::ENOENT),
        };

        let out = AttrOut {
            attr_valid: DEFAULT_TTL.as_secs(),
            attr_valid_nsec: DEFAULT_TTL.subsec_nanos(),
            attr: metadata.metadata,
            ..Default::default()
        };
        Filesystem::reply_ok(Some(out), None, in_header.unique, w)
    }

    fn setattr(&self, in_header: InHeader, _r: Reader, w: Writer) -> Result<usize> {
        debug!("setattr: inode={}", in_header.nodeid);

        // do nothing for setattr.
        self.getattr(in_header, _r, w)
    }

    fn create(&self, in_header: InHeader, mut r: Reader, w: Writer) -> Result<usize> {
        let CreateIn { flags, .. } = r.read_obj().map_err(|_| Error::from(libc::EIO))?;

        let name_len = in_header.len as usize - size_of::<InHeader>() - size_of::<CreateIn>();
        let mut buf = vec![0; name_len];
        r.read_exact(&mut buf).map_err(|_| Error::from(libc::EIO))?;
        let name = match Filesystem::bytes_to_str(buf.as_ref()) {
            Ok(name) => name,
            Err(_) => return Filesystem::reply_error(in_header.unique, w, libc::EIO),
        };

        debug!(
            "create: parent inode={} name={} flags={}",
            in_header.nodeid, name, flags
        );

        let parent_path = match self
            .opened_files
            .get(in_header.nodeid as usize)
            .map(|f| f.path.clone())
        {
            Some(path) => path,
            None => return Filesystem::reply_error(in_header.unique, w, libc::ENOENT),
        };

        let path = format!("{}/{}", parent_path, name);
        let mut attr = OpenedFile::new(FileType::File, &path);
        let inode = self
            .opened_files
            .insert(attr.clone())
            .expect("failed to allocate inode");
        attr.metadata.ino = inode as u64;
        let mut opened_files_map = self.opened_files_map.lock().unwrap();
        opened_files_map.insert(path.to_string(), inode as u64);

        match self.rt.block_on(self.do_set_writer(&path, flags)) {
            Ok(writer) => writer,
            Err(_) => return Filesystem::reply_error(in_header.unique, w, libc::ENOENT),
        };

        let entry_out = EntryOut {
            nodeid: attr.metadata.ino,
            entry_valid: DEFAULT_TTL.as_secs(),
            attr_valid: DEFAULT_TTL.as_secs(),
            entry_valid_nsec: DEFAULT_TTL.subsec_nanos(),
            attr_valid_nsec: DEFAULT_TTL.subsec_nanos(),
            attr: attr.metadata,
            ..Default::default()
        };
        let open_out = OpenOut {
            ..Default::default()
        };
        Filesystem::reply_ok(
            Some(entry_out),
            Some(open_out.as_slice()),
            in_header.unique,
            w,
        )
    }

    fn unlink(&self, in_header: InHeader, mut r: Reader, w: Writer) -> Result<usize> {
        let name_len = in_header.len as usize - size_of::<InHeader>();
        let mut buf = vec![0; name_len];
        r.read_exact(&mut buf).map_err(|_| Error::from(libc::EIO))?;
        let name = match Filesystem::bytes_to_str(buf.as_ref()) {
            Ok(name) => name,
            Err(_) => return Filesystem::reply_error(in_header.unique, w, libc::EIO),
        };

        debug!("unlink: parent inode={} name={}", in_header.nodeid, name);

        let parent_path = match self
            .opened_files
            .get(in_header.nodeid as usize)
            .map(|f| f.path.clone())
        {
            Some(path) => path,
            None => return Filesystem::reply_error(in_header.unique, w, libc::ENOENT),
        };

        let path = format!("{}/{}", parent_path, name);
        if self.rt.block_on(self.do_delete(&path)).is_err() {
            return Filesystem::reply_error(in_header.unique, w, libc::ENOENT);
        }

        let mut opened_files_map = self.opened_files_map.lock().unwrap();
        opened_files_map.remove(&path);

        Filesystem::reply_ok(None::<u8>, None, in_header.unique, w)
    }

    fn release(&self, in_header: InHeader, _r: Reader, w: Writer) -> Result<usize> {
        debug!("release: inode={}", in_header.nodeid);

        let path = match self
            .opened_files
            .get(in_header.nodeid as usize)
            .map(|f| f.path.clone())
        {
            Some(path) => path,
            None => return Filesystem::reply_error(in_header.unique, w, libc::ENOENT),
        };

        let mut opened_file_writer = self.opened_files_writer.lock().unwrap();
        opened_file_writer.remove(&path);

        Filesystem::reply_ok(None::<u8>, None, in_header.unique, w)
    }

    fn flush(&self, in_header: InHeader, _r: Reader, w: Writer) -> Result<usize> {
        debug!("flush: inode={}", in_header.nodeid);

        if self.opened_files.get(in_header.nodeid as usize).is_none() {
            return Filesystem::reply_error(in_header.unique, w, libc::ENOENT);
        }

        Filesystem::reply_ok(None::<u8>, None, in_header.unique, w)
    }

    fn open(&self, in_header: InHeader, mut r: Reader, w: Writer) -> Result<usize> {
        debug!("open: inode={}", in_header.nodeid);

        let OpenIn { flags, .. } = r.read_obj().map_err(|_| Error::from(libc::EIO))?;

        let path = match self
            .opened_files
            .get(in_header.nodeid as usize)
            .map(|f| f.path.clone())
        {
            Some(path) => path,
            None => return Filesystem::reply_error(in_header.unique, w, libc::ENOENT),
        };

        match self.rt.block_on(self.do_set_writer(&path, flags)) {
            Ok(writer) => writer,
            Err(_) => return Filesystem::reply_error(in_header.unique, w, libc::ENOENT),
        };

        let out = OpenOut {
            ..Default::default()
        };
        Filesystem::reply_ok(Some(out), None, in_header.unique, w)
    }

    fn read(&self, in_header: InHeader, mut r: Reader, mut w: Writer) -> Result<usize> {
        let ReadIn { offset, size, .. } = r.read_obj().map_err(|_| Error::from(libc::EIO))?;

        let path = match self
            .opened_files
            .get(in_header.nodeid as usize)
            .map(|f| f.path.clone())
        {
            Some(path) => path,
            None => return Filesystem::reply_error(in_header.unique, w, libc::ENOENT),
        };

        let data = match self.rt.block_on(self.do_read(&path, offset)) {
            Ok(data) => data,
            Err(_) => return Filesystem::reply_error(in_header.unique, w, libc::ENOENT),
        };
        let len = data.len();
        let buffer = BufferWrapper::new(data);

        let mut data_writer = w.split_at(size_of::<OutHeader>()).unwrap();
        data_writer
            .write_from_at(&buffer, len)
            .map_err(|_| Error::from(libc::EIO))?;

        debug!(
            "read: inode={} offset={} size={} len={}",
            in_header.nodeid, offset, size, len
        );

        let out = OutHeader {
            len: (size_of::<OutHeader>() + len) as u32,
            error: 0,
            unique: in_header.unique,
        };
        w.write_all(out.as_slice())
            .map_err(|_| Error::from(libc::EIO))?;
        Ok(out.len as usize)
    }

    fn write(&self, in_header: InHeader, mut r: Reader, w: Writer) -> Result<usize> {
        let WriteIn { offset, size, .. } = r.read_obj().map_err(|_| Error::from(libc::EIO))?;

        debug!(
            "write: inode={} offset={} size={}",
            in_header.nodeid, offset, size
        );

        let path = match self
            .opened_files
            .get(in_header.nodeid as usize)
            .map(|f| f.path.clone())
        {
            Some(path) => path,
            None => return Filesystem::reply_error(in_header.unique, w, libc::ENOENT),
        };

        let buffer = BufferWrapper::new(Buffer::new());
        r.read_to_at(&buffer, size as usize)
            .map_err(|_| Error::from(libc::EIO))?;
        let buffer = buffer.get_buffer();

        match self.rt.block_on(self.do_write(&path, offset, buffer)) {
            Ok(writer) => writer,
            Err(_) => return Filesystem::reply_error(in_header.unique, w, libc::EIO),
        };

        let out = WriteOut {
            size,
            ..Default::default()
        };
        Filesystem::reply_ok(Some(out), None, in_header.unique, w)
    }

    fn mkdir(&self, in_header: InHeader, mut r: Reader, w: Writer) -> Result<usize> {
        let MkdirIn { .. } = r.read_obj().map_err(|_| Error::from(libc::EIO))?;

        let name_len = in_header.len as usize - size_of::<InHeader>() - size_of::<MkdirIn>();
        let mut buf = vec![0; name_len];
        r.read_exact(&mut buf).map_err(|_| Error::from(libc::EIO))?;
        let name = match Filesystem::bytes_to_str(buf.as_ref()) {
            Ok(name) => name,
            Err(_) => return Filesystem::reply_error(in_header.unique, w, libc::EIO),
        };

        debug!("mkdir: parent inode={} name={}", in_header.nodeid, name);

        let parent_path = match self
            .opened_files
            .get(in_header.nodeid as usize)
            .map(|f| f.path.clone())
        {
            Some(path) => path,
            None => return Filesystem::reply_error(in_header.unique, w, libc::ENOENT),
        };

        let path = format!("{}/{}", parent_path, name);
        let mut attr = OpenedFile::new(FileType::Dir, &path);
        let inode = self
            .opened_files
            .insert(attr.clone())
            .expect("failed to allocate inode");
        attr.metadata.ino = inode as u64;
        let mut opened_files_map = self.opened_files_map.lock().unwrap();
        opened_files_map.insert(path.to_string(), inode as u64);

        if self.rt.block_on(self.do_create_dir(&path)).is_err() {
            return Filesystem::reply_error(in_header.unique, w, libc::ENOENT);
        }

        let out = EntryOut {
            nodeid: attr.metadata.ino,
            entry_valid: DEFAULT_TTL.as_secs(),
            attr_valid: DEFAULT_TTL.as_secs(),
            entry_valid_nsec: DEFAULT_TTL.subsec_nanos(),
            attr_valid_nsec: DEFAULT_TTL.subsec_nanos(),
            attr: attr.metadata,
            ..Default::default()
        };
        Filesystem::reply_ok(Some(out), None, in_header.unique, w)
    }

    fn rmdir(&self, in_header: InHeader, mut r: Reader, w: Writer) -> Result<usize> {
        let name_len = in_header.len as usize - size_of::<InHeader>();
        let mut buf = vec![0; name_len];
        r.read_exact(&mut buf).map_err(|_| Error::from(libc::EIO))?;
        let name = match Filesystem::bytes_to_str(buf.as_ref()) {
            Ok(name) => name,
            Err(_) => return Filesystem::reply_error(in_header.unique, w, libc::EIO),
        };

        debug!("rmdir: parent inode={} name={}", in_header.nodeid, name);

        let parent_path = match self
            .opened_files
            .get(in_header.nodeid as usize)
            .map(|f| f.path.clone())
        {
            Some(path) => path,
            None => return Filesystem::reply_error(in_header.unique, w, libc::ENOENT),
        };

        let path = format!("{}/{}", parent_path, name);
        if self.rt.block_on(self.do_delete(&path)).is_err() {
            return Filesystem::reply_error(in_header.unique, w, libc::ENOENT);
        }

        let mut opened_files_map = self.opened_files_map.lock().unwrap();
        opened_files_map.remove(&path);

        Filesystem::reply_ok(None::<u8>, None, in_header.unique, w)
    }

    fn releasedir(&self, in_header: InHeader, _r: Reader, w: Writer) -> Result<usize> {
        debug!("releasedir: inode={}", in_header.nodeid);

        if self.opened_files.get(in_header.nodeid as usize).is_none() {
            return Filesystem::reply_error(in_header.unique, w, libc::ENOENT);
        }

        Filesystem::reply_ok(None::<u8>, None, in_header.unique, w)
    }

    fn fsyncdir(&self, in_header: InHeader, _r: Reader, w: Writer) -> Result<usize> {
        debug!("fsyncdir: inode={}", in_header.nodeid);

        if self.opened_files.get(in_header.nodeid as usize).is_none() {
            return Filesystem::reply_error(in_header.unique, w, libc::ENOENT);
        }

        Filesystem::reply_ok(None::<u8>, None, in_header.unique, w)
    }

    fn opendir(&self, in_header: InHeader, _r: Reader, w: Writer) -> Result<usize> {
        debug!("opendir: inode={}", in_header.nodeid);

        if self.opened_files.get(in_header.nodeid as usize).is_none() {
            return Filesystem::reply_error(in_header.unique, w, libc::ENOENT);
        }

        let out = OpenOut {
            ..Default::default()
        };
        Filesystem::reply_ok(Some(out), None, in_header.unique, w)
    }

    fn readdir(&self, in_header: InHeader, mut r: Reader, mut w: Writer) -> Result<usize> {
        let path = match self
            .opened_files
            .get(in_header.nodeid as usize)
            .map(|f| f.path.clone())
        {
            Some(path) => path,
            None => return Filesystem::reply_error(in_header.unique, w, libc::ENOENT),
        };

        let ReadIn { offset, size, .. } = r.read_obj().map_err(|_| Error::from(libc::EIO))?;

        debug!(
            "readdir: inode={} offset={} size={}",
            in_header.nodeid, offset, size
        );

        let mut data_writer = w.split_at(size_of::<OutHeader>()).unwrap();

        let entries = match self.rt.block_on(self.do_readdir(&path)) {
            Ok(entries) => entries,
            Err(_) => return Filesystem::reply_error(in_header.unique, w, libc::ENOENT),
        };

        if offset as usize >= entries.len() {
            let out = OutHeader {
                len: size_of::<OutHeader>() as u32,
                error: 0,
                unique: in_header.unique,
            };
            w.write_all(out.as_slice())
                .map_err(|_| Error::from(libc::EIO))?;
            return Ok(out.len as usize);
        }

        let mut total_written = 0;
        for entry in entries {
            match Filesystem::reply_add_dir_entry(&mut data_writer, entry) {
                Ok(len) => {
                    total_written += len;
                }
                Err(_) => return Filesystem::reply_error(in_header.unique, w, libc::EIO),
            };
        }

        let out = OutHeader {
            len: (size_of::<OutHeader>() + total_written) as u32,
            error: 0,
            unique: in_header.unique,
        };

        w.write_all(out.as_slice())
            .map_err(|_| Error::from(libc::EIO))?;
        Ok(out.len as usize)
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
        w.write_all(header.as_slice())
            .map_err(|_| Error::from(libc::EIO))?;
        if let Some(out) = out {
            w.write_all(out.as_slice())
                .map_err(|_| Error::from(libc::EIO))?;
        }
        if let Some(data) = data {
            w.write_all(data).map_err(|_| Error::from(libc::EIO))?;
        }
        Ok(w.bytes_written())
    }

    fn reply_add_dir_entry(cursor: &mut Writer, entry: DirEntry) -> Result<usize> {
        let entry_len = size_of::<DirEntryOut>() + entry.name.len();
        let total_len = (entry_len + 7) & !7;

        let out = DirEntryOut {
            ino: entry.ino,
            off: entry.off,
            namelen: entry.name.len() as u32,
            type_: entry.type_,
        };

        cursor
            .write_all(out.as_slice())
            .map_err(|_| Error::from(libc::EIO))?;
        cursor
            .write_all(entry.name.as_bytes())
            .map_err(|_| Error::from(libc::EIO))?;

        let padding = total_len - entry_len;
        if padding > 0 {
            cursor
                .write_all(&DIRENT_PADDING[..padding])
                .map_err(|_| Error::from(libc::EIO))?;
        }

        Ok(total_len)
    }

    fn reply_error(unique: u64, mut w: Writer, error: libc::c_int) -> Result<usize> {
        let header = OutHeader {
            unique,
            error: -error,
            len: size_of::<OutHeader>() as u32,
        };
        w.write_all(header.as_slice())
            .map_err(|_| Error::from(libc::EIO))?;
        Ok(w.bytes_written())
    }

    fn bytes_to_str(buf: &[u8]) -> Result<&str> {
        Filesystem::bytes_to_cstr(buf)?
            .to_str()
            .map_err(|_| Error::from(libc::EINVAL))
    }

    fn bytes_to_cstr(buf: &[u8]) -> Result<&CStr> {
        CStr::from_bytes_with_nul(buf).map_err(|_| Error::from(libc::EINVAL))
    }

    fn check_flags(&self, flags: u32) -> Result<(bool, bool)> {
        let is_trunc = flags & libc::O_TRUNC as u32 != 0 || flags & libc::O_CREAT as u32 != 0;
        let is_append = flags & libc::O_APPEND as u32 != 0;
        let mode = flags & libc::O_ACCMODE as u32;
        let is_write = mode == libc::O_WRONLY as u32 || mode == libc::O_RDWR as u32 || is_append;

        let capability = self.core.info().full_capability();
        if is_trunc && !capability.write {
            Err(Error::from(libc::EACCES))?;
        }
        if is_append && !capability.write_can_append {
            Err(Error::from(libc::EACCES))?;
        }
        Ok((is_write, is_append))
    }
}

impl Filesystem {
    async fn do_get_metadata(&self, path: &str) -> Result<OpenedFile> {
        let metadata = self.core.stat(path).await.map_err(|err| Error::from(err))?;
        let file_type = match metadata.mode() {
            opendal::EntryMode::DIR => FileType::Dir,
            _ => FileType::File,
        };
        let mut attr = OpenedFile::new(file_type, path);
        attr.metadata.size = metadata.content_length();
        let mut opened_files_map = self.opened_files_map.lock().unwrap();
        if let Some(inode) = opened_files_map.get(path) {
            attr.metadata.ino = *inode;
        } else {
            let inode = self
                .opened_files
                .insert(attr.clone())
                .expect("failed to allocate inode");
            attr.metadata.ino = inode as u64;
            opened_files_map.insert(path.to_string(), inode as u64);
        }

        Ok(attr)
    }

    async fn do_set_writer(&self, path: &str, flags: u32) -> Result<()> {
        let (is_write, is_append) = self.check_flags(flags)?;
        if !is_write {
            return Ok(());
        }

        let writer = self
            .core
            .writer_with(path)
            .append(is_append)
            .await
            .map_err(|err| Error::from(err))?;
        let written = if is_append {
            self.core
                .stat(path)
                .await
                .map_err(|err| Error::from(err))?
                .content_length()
        } else {
            0
        };

        let inner_writer = InnerWriter { writer, written };
        let mut opened_file_writer = self.opened_files_writer.lock().unwrap();
        opened_file_writer.insert(path.to_string(), inner_writer);

        Ok(())
    }

    async fn do_delete(&self, path: &str) -> Result<()> {
        self.core
            .delete(path)
            .await
            .map_err(|err| Error::from(err))?;

        Ok(())
    }

    async fn do_read(&self, path: &str, offset: u64) -> Result<Buffer> {
        let data = self
            .core
            .read_with(path)
            .range(offset..)
            .await
            .map_err(|err| Error::from(err))?;

        Ok(data)
    }

    async fn do_write(&self, path: &str, offset: u64, data: Buffer) -> Result<usize> {
        let len = data.len();
        let mut opened_file_writer = self.opened_files_writer.lock().unwrap();
        let inner_writer = opened_file_writer
            .get_mut(path)
            .ok_or(Error::from(libc::EIO))?;
        if offset != inner_writer.written {
            return Err(Error::from(libc::EIO));
        }
        inner_writer
            .writer
            .write_from(data)
            .await
            .map_err(|err| Error::from(err))?;
        inner_writer.written += len as u64;

        Ok(len)
    }

    async fn do_create_dir(&self, path: &str) -> Result<()> {
        let path = if !path.ends_with('/') {
            format!("{}/", path)
        } else {
            path.to_string()
        };
        self.core
            .create_dir(&path)
            .await
            .map_err(|err| Error::from(err))?;

        Ok(())
    }

    async fn do_readdir(&self, path: &str) -> Result<Vec<DirEntry>> {
        let path = if !path.ends_with('/') {
            format!("{}/", path)
        } else {
            path.to_string()
        };

        let entries = self
            .core
            .list(&path)
            .await
            .map_err(|err| Error::from(err))?
            .into_iter()
            .enumerate()
            .map(|(i, entry)| {
                let metadata = entry.metadata();
                let file_type = match metadata.mode() {
                    opendal::EntryMode::DIR => FileType::Dir,
                    _ => FileType::File,
                };

                let path = format!("{}/{}", path, entry.name());
                let mut attr = OpenedFile::new(file_type, &path);
                attr.metadata.size = metadata.content_length();

                let mut opened_files_map = self.opened_files_map.lock().unwrap();
                let inode = if let Some(inode) = opened_files_map.get(&path) {
                    *inode
                } else {
                    let inode = self
                        .opened_files
                        .insert(attr)
                        .expect("failed to allocate inode");
                    opened_files_map.insert(path.to_string(), inode as u64);
                    inode as u64
                };

                let type_ = match metadata.mode() {
                    opendal::EntryMode::DIR => DEAFULT_DIR_TYPE_IN_DIR_ENTRY,
                    _ => DEAFULT_FILE_TYPE_IN_DIR_ENTRY,
                };

                let mut name = entry.name().to_string();
                if name.ends_with('/') {
                    name.truncate(name.len() - 1);
                }

                let entry = DirEntry {
                    ino: inode,
                    off: i as u64 + 1,
                    name,
                    type_,
                };
                entry
            })
            .collect();

        Ok(entries)
    }
}
