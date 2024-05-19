use std::io;
use std::ffi::CStr;
use std::sync::RwLock;
use std::path::PathBuf;
use std::collections::HashMap;
use std::time::{Duration, SystemTime};

use opendal::Buffer;
use opendal::Operator;
use sharded_slab::Slab;
use tokio::runtime::Runtime;

use super::*;
use super::utils::*;
use super::config::*;
use super::consts::*;
use crate::filesystem::*;
use crate::placeholder::*;
use crate::file_traits::*;

pub struct OVFSFileSystem {
    rt: Runtime,
    config: Config,
    core: Operator,
    opened_inodes: Slab<RwLock<InodeData>>,            // inode -> inode_data
    opened_inodes_map: RwLock<HashMap<String, Inode>>, // path in backend -> inode
}

impl OVFSFileSystem {
    pub fn new(core: Operator) -> OVFSFileSystem {
        OVFSFileSystem {
            core,
            rt: Runtime::new().unwrap(),
            config: Config::default(),
            opened_inodes: Slab::default(),
            opened_inodes_map: RwLock::new(HashMap::default()),
        }
    }

    fn build_path(&self, path: &str) -> String {
        let path = PathBuf::from(&self.config.root_dir).join(path);
        path.to_string_lossy().to_string()
    }

    fn get_opened(&self, path: &str) -> Option<Inode> {
        let map = self.opened_inodes_map.read().unwrap();
        map.get(path).copied()
    }

    fn set_opened(&self, path: &str, inode: Inode) {
        let mut map = self.opened_inodes_map.write().unwrap();
        map.insert(path.to_string(), inode);
    }

    fn delete_opened(&self, path: &str) {
        let mut map = self.opened_inodes_map.write().unwrap();
        map.remove(path);
    }

    fn get_opened_inode(&self, key: Inode) -> io::Result<InodeData> {
        if let Some(opened_inode) = self.opened_inodes.get(key as usize) {
            let inode_data = opened_inode.read()
                .unwrap()
                .clone();
            Ok(inode_data)
        } else {
            Err(io::Error::from_raw_os_error(libc::EBADF)) // Invalid inode
        }
    }

    fn set_opened_inode(&self, key: Inode, value: InodeData) -> io::Result<()> {
        if let Some(opened_inode) = self.opened_inodes.get(key as usize) {
            let mut inode_data = opened_inode.write().unwrap();
            *inode_data = value;
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(libc::EBADF)) // Invalid inode
        }
    }

    fn insert_opened_inode(&self, value: InodeData) -> io::Result<Inode> {
        if let Some(key) = self.opened_inodes.insert(RwLock::new(value)) {
            Ok(key as Inode)
        } else {
            Err(io::Error::from_raw_os_error(libc::ENFILE)) // Too many opened fiels
        }
    }

    fn delete_opened_inode(&self, key: Inode) -> io::Result<()> {
        if self.opened_inodes.remove(key as usize) {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(libc::EBADF)) // Invalid inode
        }
    }
}

impl OVFSFileSystem {
    async fn do_get_stat(&self, path: &str) -> io::Result<InodeData> {
        let metadata = self
            .core
            .stat(path)
            .await
            .map_err(opendal_error2error)?;

        let now = SystemTime::now();
        let attr = opendal_metadata2stat64(&metadata, now);

        Ok(attr)
    }

    async fn do_create_file(&self, path: &str) -> io::Result<()> {
        self.core
            .write(&path, Buffer::new())
            .await
            .map_err(opendal_error2error)?;

        Ok(())
    }

    async fn do_delete_file(&self, path: &str) -> io::Result<()> {
        self.core
            .delete(&path)
            .await
            .map_err(opendal_error2error)?;

        Ok(())
    }

    async fn do_read(&self, path: &str, offset: u64, size: u64) -> io::Result<Buffer> {
        let data = self
            .core
            .read_with(&path)
            .range(offset..offset + size)
            .await
            .map_err(opendal_error2error)?;

        Ok(data)
    }

    async fn do_write(&self, path: &str, data: Buffer) -> io::Result<()> {
        self
            .core
            .write_with(path, data)
            .await
            .map_err(opendal_error2error)?;

        Ok(())
    }

    async fn do_create_dir(&self, path: &str) -> io::Result<()> {
        self.core
            .create_dir(&path)
            .await
            .map_err(opendal_error2error)?;

        Ok(())
    }

    async fn do_delete_dir(&self, path: &str) -> io::Result<()> {
        self.core
            .delete(&path)
            .await
            .map_err(opendal_error2error)?;

        Ok(())
    }

    async fn do_readdir(&self, path: &str) -> io::Result<Vec<InodeData>> {
        let now = SystemTime::now();

        let children = self
            .core
            .list(&path)
            .await
            .map_err(opendal_error2error)?
            .into_iter()
            .map(|e| opendal_metadata2stat64(e.metadata(), now))
            .collect();

        Ok(children)
    }
}

impl FileSystem for OVFSFileSystem {
    // Inode number which is lazily allocate when accessed
    type Inode = Inode;
    // There is no need to use for now
    type Handle = Handle;
    // Directory traversal iterator
    type DirIter = ReadDir;

    fn init(&self, _capable: FsOptions) -> io::Result<FsOptions> {
        // Set the root dir's InodeData
        let data = InodeData::new(InodeType::FILE, "/");
        let _ = self.insert_opened_inode(data.clone())?;
        Ok(FsOptions::empty())
    }

    fn destroy(&self) {}

    fn lookup(&self, _ctx: Context, parent: Self::Inode, name: &CStr) -> io::Result<Entry> {
        let name = match name.to_str() {
            Ok(name) => name,
            Err(_) => Err(io::Error::from_raw_os_error(libc::EBADF))?,
        };
        let file = self.get_opened_inode(parent)?;
        let path = PathBuf::from(file.path).join(name);
        let metadata = self.rt.block_on(self.do_get_stat(&path.to_string_lossy()))?;
        let ino = self.insert_opened_inode(metadata.clone())?;
        Ok(Entry {
            ino,
            attr: metadata.stat,
            attr_flags: DEFAULT_ATTR_FLAGS,
            attr_timeout: self.config.attr_timeout,
            entry_timeout: self.config.entry_timeout,
        })
    }

    fn getattr(
        &self,
        _ctx: Context,
        inode: Self::Inode,
        _handle: Option<Self::Handle>,
    ) -> io::Result<(libc::stat64, Duration)> {
        let file = self.get_opened_inode(inode)?;
        Ok((file.stat, self.config.attr_timeout))
    }

    fn setattr(
        &self,
        _ctx: Context,
        inode: Self::Inode,
        _attr: libc::stat64,
        _handle: Option<Self::Handle>,
    ) -> io::Result<(libc::stat64, Duration)> {
        let file = self.get_opened_inode(inode)?;
        Ok((file.stat, self.config.attr_timeout))
    }

    fn mknod(
        &self,
        name: &CStr,
        _mode: u32,
        _rdev: u32,
        _umask: u32,
        _extension: Extensions,
    ) -> io::Result<Entry> {
        let name = match name.to_str() {
            Ok(name) => name,
            Err(_) => Err(io::Error::from_raw_os_error(libc::EBADF))?,
        };
        let path = self.build_path(name);
        self.rt.block_on(self.do_create_file(&path))?;
        let data = InodeData::new(InodeType::FILE, &path);
        let ino = self.insert_opened_inode(data.clone())?;
        Ok(Entry{
            ino,
            attr: data.stat,
            attr_flags: DEFAULT_ATTR_FLAGS,
            attr_timeout: self.config.attr_timeout,
            entry_timeout: self.config.entry_timeout,
        })
    }

    fn create(
        &self,
        _ctx: Context,
        parent: Self::Inode,
        name: &CStr,
        _mode: u32,
        _kill_priv: bool,
        _flags: u32,
        _umask: u32,
        _extension: Extensions,
    )-> io::Result<(Entry, Option<Self::Handle>, OpenOptions)> {
        let name = match name.to_str() {
            Ok(name) => name,
            Err(_) => Err(io::Error::from_raw_os_error(libc::EINVAL))?, // Invalid argument
        };
        let file = self.get_opened_inode(parent)?;
        let path = PathBuf::from(file.path).join(name).to_string_lossy().to_string();
        self.rt.block_on(self.do_create_file(&path))?;
        let data = InodeData::new(InodeType::FILE, &path);
        let ino = self.insert_opened_inode(data.clone())?;
        Ok((Entry{
            ino,
            attr: data.stat,
            attr_flags: DEFAULT_ATTR_FLAGS,
            attr_timeout: self.config.attr_timeout,
            entry_timeout: self.config.entry_timeout,
        }, Some(0), OpenOptions::empty()))
    }

    fn unlink(
        &self,
        _ctx: Context,
        parent: Self::Inode,
        name: &CStr,
    ) -> io::Result<()> {
        let name = match name.to_str() {
            Ok(name) => name,
            Err(_) => Err(io::Error::from_raw_os_error(libc::EBADF))?,
        };
        let file = self.get_opened_inode(parent)?;
        let path = PathBuf::from(file.path).join(name).to_string_lossy().to_string();
        self.rt.block_on(self.do_delete_file(&path))?;
        Ok(())
    }

    fn open(
        &self,
        _ctx: Context,
        inode: Self::Inode,
        _kill_priv: bool,
        _flags: u32,
    ) -> io::Result<(Option<Self::Handle>, OpenOptions)> {
        // inode should have been loaded by lookup
        let _ = self.get_opened_inode(inode)?;
        Ok((Some(inode), OpenOptions::empty()))
    }

    fn read<W: io::Write + ZeroCopyWriter>(
        &self,
        _ctx: Context,
        inode: Self::Inode,
        _handle: Self::Handle,
        mut w: W,
        size: u32,
        offset: u64,
        _lock_owner: Option<u64>,
        _flags: u32,
    ) -> io::Result<usize> {
        let inode_data = self.get_opened_inode(inode)?;
        let data = self.rt.block_on( self.do_read(&inode_data.path, offset, size as u64))?;
        let mut data_wrapper  = BufferWrapper::new(data);
        let size = w.copy_to_end(&mut data_wrapper, 0)?;
        Ok(size)
    }

    fn write<R: io::Read + ZeroCopyReader>(
        &self,
        _ctx: Context,
        inode: Self::Inode,
        _handle: Self::Handle,
        mut r: R,
        _size: u32,
        _offset: u64,
        _lock_owner: Option<u64>,
        _delayed_write: bool,
        _kill_priv: bool,
        _flags: u32,
    ) -> io::Result<usize> {
        let inode_data = self.get_opened_inode(inode)?;
        let mut data_wrapper = BufferWrapper::new(opendal::Buffer::new());
        let size = r.copy_to_end(&mut data_wrapper, 0)?;
        let data = data_wrapper.get_buffer();
        self.rt.block_on( self.do_write(&inode_data.path, data))?;
        Ok(size)
    }

    fn mkdir(
        &self,
        _ctx: Context,
        parent: Self::Inode,
        name: &CStr,
        _mode: u32,
        _umask: u32,
        _extensions: Extensions,
    ) -> io::Result<Entry> {
        let name = match name.to_str() {
            Ok(name) => name,
            Err(_) => Err(io::Error::from_raw_os_error(libc::EBADF))?,
        };
        let file = self.get_opened_inode(parent)?;
        let path = PathBuf::from(file.path).join(name).to_string_lossy().to_string();
        self.rt.block_on(self.do_create_dir(&path))?;
        let data = InodeData::new(InodeType::DIR, &path);
        let ino = self.insert_opened_inode(data.clone())?;
        Ok(Entry{
            ino,
            attr: data.stat,
            attr_flags: DEFAULT_ATTR_FLAGS,
            attr_timeout: self.config.attr_timeout,
            entry_timeout: self.config.entry_timeout,
        })
    }

    fn rmdir(
        &self,
        _ctx: Context,
        parent: Self::Inode,
        name: &CStr,
    ) -> io::Result<()> {
        let name = match name.to_str() {
            Ok(name) => name,
            Err(_) => Err(io::Error::from_raw_os_error(libc::EBADF))?,
        };
        let file = self.get_opened_inode(parent)?;
        let path = PathBuf::from(file.path).join(name).to_string_lossy().to_string();
        self.rt.block_on(self.do_delete_dir(&path))?;
        Ok(())
    }

    fn opendir(
        &self,
        _ctx: Context,
        inode: Self::Inode,
        _flags: u32,
    ) -> io::Result<(Option<Self::Handle>, OpenOptions)> {
        // inode should have been loaded by lookup
        let _ = self.get_opened_inode(inode)?;
        Ok((Some(inode), OpenOptions::empty()))
    }

    fn readdir(
        &self,
        _ctx: Context,
        inode: Self::Inode,
        _handle: Self::Handle,
        _size: u32,
        _offset: u64,
    ) -> io::Result<Self::DirIter> {
        let inode_data = self.get_opened_inode(inode)?;
        let data = self.rt.block_on( self.do_readdir(&inode_data.path))?;
        let iter = ReadDir::new(data);
        Ok(iter)
    }

    fn release(
        &self,
        _ctx: Context,
        _indoe: Self::Inode,
        _flags: u32,
        _handle: Self::Handle,
        _flush: bool,
        _flock_release: bool,
        _lock_owner: Option<u64>,
    ) -> io::Result<()> {
        Ok(())
    }

    fn releasedir(
        &self,
        _ctx: Context,
        _inode: Self::Inode,
        _flags: u32,
        _handle: Self::Handle,
    ) -> io::Result<()> {
        Ok(())
    }

    fn access(&self, _ctx: Context, _inode: Self::Inode, _mask: u32) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::env;
    use super::*;
    use opendal::services::{Memory, Fs};

    #[test]
    fn test_ovfs_opened_inodes() {
        let builder = Memory::default();
        let operator = Operator::new(builder)
            .expect("failed to build operator")
            .finish();
        let ovfs = OVFSFileSystem::new(operator);

        let path = "/test_file";
        ovfs.set_opened(path, 0);
        assert_eq!(ovfs.get_opened(path), Some(0));
        ovfs.delete_opened(path);
        assert_eq!(ovfs.get_opened(path), None);

        let dir_inode_path = "/test_dir";
        let file_inode_path = "/test_dir/test_file";
        let mut dir_inode_data = InodeData::new(InodeType::DIR, dir_inode_path);
        let mut file_inode_data = InodeData::new(InodeType::FILE, file_inode_path);
        assert_eq!(ovfs.insert_opened_inode(dir_inode_data.clone()).ok(), Some(0));
        assert_eq!(ovfs.insert_opened_inode(file_inode_data.clone()).ok(), Some(1));
        assert!(matches!(ovfs.get_opened_inode(0), Ok(ref inode_data) if inode_data.path == dir_inode_path));
        assert!(matches!(ovfs.get_opened_inode(1), Ok(ref inode_data) if inode_data.path == file_inode_path));
        dir_inode_data.inode_type = InodeType::FILE;
        file_inode_data.inode_type = InodeType::DIR;
        assert!(ovfs.set_opened_inode(0, dir_inode_data.clone()).is_ok());
        assert!(ovfs.set_opened_inode(1, file_inode_data.clone()).is_ok());
        assert!(matches!(ovfs.get_opened_inode(0), Ok(ref inode_data) if inode_data.inode_type == InodeType::FILE));
        assert!(matches!(ovfs.get_opened_inode(1), Ok(ref inode_data) if inode_data.inode_type == InodeType::DIR));
        assert!(ovfs.delete_opened_inode(0).is_ok());
        assert!(ovfs.delete_opened_inode(1).is_ok());
        assert!(ovfs.get_opened_inode(0).is_err());
        assert!(ovfs.get_opened_inode(1).is_err());
    }

    #[test]
    fn test_ovfs_core_based_on_fs() {
        let test_dir = env::current_dir()
            .expect("failed to get current dir")
            .join("tmp");
        fs::create_dir(&test_dir).expect("failed to create test dir");

        let mut builder = Fs::default();
        builder.root(test_dir.to_str().unwrap());
        let operator = Operator::new(builder)
            .expect("failed to build operator")
            .finish();
        let ovfs = OVFSFileSystem::new(operator);

        fs::remove_dir_all(&test_dir).expect("failed to remove test dir");
    }
}
