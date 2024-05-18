use std::io;
use std::ffi::CStr;
use std::ops::Deref;
use std::sync::RwLock;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use opendal::Buffer;
use opendal::Operator;
use sharded_slab::Slab;
use tokio::runtime::Runtime;

use super::*;
use super::types::*;
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
    opened_inodes: Slab<RwLock<InodeData>>,
}

impl OVFSFileSystem {
    pub fn new(core: Operator) -> OVFSFileSystem {
        OVFSFileSystem {
            core,
            rt: Runtime::new().unwrap(),
            config: Config::default(),
            opened_inodes: Slab::default(),
        }
    }

    fn build_path(&self, path: &str) -> String {
        let path = PathBuf::from(&self.config.root_dir).join(path);
        path.to_string_lossy().to_string()
    }

    fn get_opened_file(&self, key: Inode) -> Result<InodeData> {
        let file = match self
            .opened_inodes
            .get(key as usize)
            .as_ref()
            .ok_or(Error::from(libc::ENOENT))?
            .deref()
            .read()
        {
            Ok(file) => file.clone(),
            Err(_) => Err(Error::from(libc::EBADF))?,
        };

        Ok(file)
    }

    fn set_opened_file(&self, value: InodeData) -> Result<Inode> {
        if let Some(key) = self.opened_inodes.insert(RwLock::new(value)) {
            Ok(key as Inode)
        } else {
            Err(Error::from(libc::EBADF))
        }
    }

    fn delete_opened_file(&self, key: Inode) -> Result<()> {
        unimplemented!()
    }
}

impl OVFSFileSystem {
    async fn do_get_stat(&self, path: &str) -> Result<InodeData> {
        let metadata = self
            .core
            .stat(path)
            .await
            .map_err(opendal_error2error)?;

        let now = SystemTime::now();
        let attr = opendal_metadata2stat64(&metadata, now);

        Ok(attr)
    }

    async fn do_create_file(&self, path: &str) -> Result<()> {
        self.core
            .write(&path, Buffer::new())
            .await
            .map_err(opendal_error2error)?;

        Ok(())
    }

    async fn do_delete_file(&self, path: &str) -> Result<()> {
        self.core
            .delete(&path)
            .await
            .map_err(opendal_error2error)?;

        Ok(())
    }

    async fn do_read(&self, path: &str, offset: u64, size: u64) -> Result<Buffer> {
        let data = self
            .core
            .read_with(&path)
            .range(offset..offset + size)
            .await
            .map_err(opendal_error2error)?;

        Ok(data)
    }

    async fn do_write(&self, path: &str, data: Buffer) -> Result<()> {
        self
            .core
            .write_with(path, data)
            .await
            .map_err(opendal_error2error)?;

        Ok(())
    }

    async fn do_create_dir(&self, path: &str) -> Result<()> {
        self.core
            .create_dir(&path)
            .await
            .map_err(opendal_error2error)?;

        Ok(())
    }

    async fn do_delete_dir(&self, path: &str) -> Result<()> {
        self.core
            .delete(&path)
            .await
            .map_err(opendal_error2error)?;

        Ok(())
    }

    async fn do_readdir(&self, path: &str) -> Result<Vec<InodeData>> {
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
    type Inode = Inode;
    type Handle = Handle;
    type DirIter = ReadDir;

    fn init(&self, _capable: FsOptions) -> io::Result<FsOptions> {
        // Set the root dir's InodeData
        let data = InodeData::new(InodeType::FILE, "/".to_string());
        let _ = self.set_opened_file(data.clone())?;
        Ok(FsOptions::empty())
    }

    fn destroy(&self) {}

    fn lookup(&self, _ctx: Context, parent: Self::Inode, name: &CStr) -> io::Result<Entry> {
        let name = match name.to_str() {
            Ok(name) => name,
            Err(_) => Err(Error::from(libc::EBADF))?,
        };
        let file = self.get_opened_file(parent)?;
        let path = PathBuf::from(file.path).join(name);
        let metadata = self.rt.block_on(self.do_get_stat(&path.to_string_lossy()))?;
        let ino = self.set_opened_file(metadata.clone())?;
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
        let file = self.get_opened_file(inode)?;
        Ok((file.stat, self.config.attr_timeout))
    }

    fn setattr(
        &self,
        _ctx: Context,
        inode: Self::Inode,
        _attr: libc::stat64,
        _handle: Option<Self::Handle>,
    ) -> io::Result<(libc::stat64, Duration)> {
        let file = self.get_opened_file(inode)?;
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
            Err(_) => Err(Error::from(libc::EBADF))?,
        };
        let path = self.build_path(name);
        self.rt.block_on(self.do_create_file(&path))?;
        let data = InodeData::new(InodeType::FILE, path);
        let ino = self.set_opened_file(data.clone())?;
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
            Err(_) => Err(Error::from(libc::EBADF))?,
        };
        let file = self.get_opened_file(parent)?;
        let path = PathBuf::from(file.path).join(name).to_string_lossy().to_string();
        self.rt.block_on(self.do_create_file(&path))?;
        let data = InodeData::new(InodeType::FILE, path);
        let ino = self.set_opened_file(data.clone())?;
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
            Err(_) => Err(Error::from(libc::EBADF))?,
        };
        let file = self.get_opened_file(parent)?;
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
        let _ = self.get_opened_file(inode)?;
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
        let inode_data = self.get_opened_file(inode)?;
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
        let inode_data = self.get_opened_file(inode)?;
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
            Err(_) => Err(Error::from(libc::EBADF))?,
        };
        let file = self.get_opened_file(parent)?;
        let path = PathBuf::from(file.path).join(name).to_string_lossy().to_string();
        self.rt.block_on(self.do_create_dir(&path))?;
        let data = InodeData::new(InodeType::DIR, path);
        let ino = self.set_opened_file(data.clone())?;
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
            Err(_) => Err(Error::from(libc::EBADF))?,
        };
        let file = self.get_opened_file(parent)?;
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
        let _ = self.get_opened_file(inode)?;
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
        let inode_data = self.get_opened_file(inode)?;
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
