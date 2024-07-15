use std::cell::RefCell;
use std::cmp::min;
use std::io;
use std::io::Result;
use std::ptr;

use vm_memory::bitmap::BitmapSlice;
use vm_memory::VolatileSlice;

pub struct BufferWrapper {
    buffer: RefCell<opendal::Buffer>,
}

impl BufferWrapper {
    pub fn new(buffer: opendal::Buffer) -> BufferWrapper {
        BufferWrapper {
            buffer: RefCell::new(buffer),
        }
    }

    pub fn get_buffer(&self) -> opendal::Buffer {
        return self.buffer.borrow().clone();
    }
}

pub trait FileReadWriteAtVolatile<B: BitmapSlice> {
    fn read_vectored_at_volatile(&self, bufs: &[&VolatileSlice<B>], offset: u64) -> Result<usize>;
    fn write_vectored_at_volatile(&self, bufs: &[&VolatileSlice<B>], offset: u64) -> Result<usize>;
}

impl<'a, B: BitmapSlice, T: FileReadWriteAtVolatile<B> + ?Sized> FileReadWriteAtVolatile<B>
    for &'a T
{
    fn read_vectored_at_volatile(&self, bufs: &[&VolatileSlice<B>], offset: u64) -> Result<usize> {
        (**self).read_vectored_at_volatile(bufs, offset)
    }

    fn write_vectored_at_volatile(&self, bufs: &[&VolatileSlice<B>], offset: u64) -> Result<usize> {
        (**self).write_vectored_at_volatile(bufs, offset)
    }
}

impl<B: BitmapSlice> FileReadWriteAtVolatile<B> for BufferWrapper {
    fn read_vectored_at_volatile(&self, bufs: &[&VolatileSlice<B>], _offset: u64) -> Result<usize> {
        let slice_guards: Vec<_> = bufs.iter().map(|s| s.ptr_guard_mut()).collect();
        let iovecs: Vec<_> = slice_guards
            .iter()
            .map(|s| libc::iovec {
                iov_base: s.as_ptr() as *mut libc::c_void,
                iov_len: s.len() as libc::size_t,
            })
            .collect();
        if iovecs.is_empty() {
            return Ok(0);
        }
        let data = self.buffer.borrow().to_vec();
        let mut result = 0;
        for (index, iovec) in iovecs.iter().enumerate() {
            let num = min(data.len() - result, iovec.iov_len);
            if num == 0 {
                break;
            }
            unsafe {
                ptr::copy_nonoverlapping(data[result..].as_ptr(), iovec.iov_base as *mut u8, num)
            }
            bufs[index].bitmap().mark_dirty(0, num);
            result += num;
        }
        Ok(result)
    }

    fn write_vectored_at_volatile(
        &self,
        bufs: &[&VolatileSlice<B>],
        _offset: u64,
    ) -> Result<usize> {
        let slice_guards: Vec<_> = bufs.iter().map(|s| s.ptr_guard_mut()).collect();
        let iovecs: Vec<_> = slice_guards
            .iter()
            .map(|s| libc::iovec {
                iov_base: s.as_ptr() as *mut libc::c_void,
                iov_len: s.len() as libc::size_t,
            })
            .collect();
        if iovecs.is_empty() {
            return Ok(0);
        }
        let len = iovecs.iter().map(|iov| iov.iov_len).sum();
        let mut data = vec![0; len];
        let mut offset = 0;
        for iov in iovecs.iter() {
            unsafe {
                ptr::copy_nonoverlapping(
                    iov.iov_base as *const u8,
                    data.as_mut_ptr().add(offset),
                    iov.iov_len,
                );
            }
            offset += len;
        }
        *self.buffer.borrow_mut() = opendal::Buffer::from(data);
        Ok(len)
    }
}

pub trait ZeroCopyReader {
    fn read_to(&mut self, f: &BufferWrapper, count: usize, off: u64) -> io::Result<usize>;

    fn read_exact_to(
        &mut self,
        f: &mut BufferWrapper,
        mut count: usize,
        mut off: u64,
    ) -> io::Result<()> {
        let c = count
            .try_into()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        if off.checked_add(c).is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "off + count must be less than u64::MAX",
            ));
        }
        while count > 0 {
            match self.read_to(f, count, off) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "failed to fill whole buffer",
                    ))
                }
                Ok(n) => {
                    count -= n;
                    off += n as u64;
                }
                Err(ref e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    fn copy_to_end(&mut self, f: &mut BufferWrapper, mut off: u64) -> io::Result<usize> {
        let mut out = 0;
        loop {
            match self.read_to(f, usize::MAX, off) {
                Ok(0) => return Ok(out),
                Ok(n) => {
                    off = off.saturating_add(n as u64);
                    out += n;
                }
                Err(ref e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
    }
}

pub trait ZeroCopyWriter {
    fn write_from(&mut self, f: &BufferWrapper, count: usize, off: u64) -> io::Result<usize>;

    fn write_all_from(
        &mut self,
        f: &mut BufferWrapper,
        mut count: usize,
        mut off: u64,
    ) -> io::Result<()> {
        let c = count
            .try_into()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        if off.checked_add(c).is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "off + count must be less than u64::MAX",
            ));
        }
        while count > 0 {
            match self.write_from(f, count, off) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "failed to write whole buffer",
                    ))
                }
                Ok(n) => {
                    count -= n;
                    off += n as u64;
                }
                Err(ref e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    fn copy_to_end(&mut self, f: &mut BufferWrapper, mut off: u64) -> io::Result<usize> {
        let mut out = 0;
        loop {
            match self.write_from(f, usize::MAX, off) {
                Ok(0) => return Ok(out),
                Ok(n) => {
                    off = off.saturating_add(n as u64);
                    out += n;
                }
                Err(ref e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
    }
}
