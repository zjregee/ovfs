use std::ops::Deref;
use std::sync::RwLock;
use std::time::SystemTime;

use opendal::Buffer;

use super::*;
use super::utils::*;

pub struct OVFSFilesystem {
    core: opendal::Operator,
    opened_inodes: sharded_slab::Slab<RwLock<InodeData>>,
}

impl OVFSFilesystem {
    pub fn new(core: opendal::Operator) -> OVFSFilesystem {
        OVFSFilesystem {
            core,
            opened_inodes: sharded_slab::Slab::new()
        }
    }

    fn get_opened_file(&self, key: InodeKey) -> Result<InodeData> {
        let file = match self
            .opened_inodes
            .get(key.0)
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
}

impl OVFSFilesystem {
    async fn do_get_stat(&self, path: &str) -> Result<InodeData> {
        let metadata = self
            .core
            .stat(path)
            .await
            .map_err(opendal_error2error)?;

        let now = SystemTime::now();
        let attr = opendal_metadata2stat64(metadata, now);

        Ok(attr)
    }

    async fn do_create_file(&self, path: &str) -> Result<()> {
        self.core
            .write(&path, Buffer::new())
            .await
            .map_err(opendal_error2error)?;

        Ok(())
    }

    async fn delete_file(&self, path: &str) -> Result<()> {
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
        unimplemented!()
    }
}
