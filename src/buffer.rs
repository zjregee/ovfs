use std::cell::RefCell;
use std::cmp::min;
use std::ptr;

use vm_memory::bitmap::BitmapSlice;
use vm_memory::VolatileSlice;

use crate::error::*;

pub trait ReadWriteAtVolatile<B: BitmapSlice> {
    fn read_vectored_at_volatile(&self, bufs: &[&VolatileSlice<B>]) -> Result<usize>;
    fn write_vectored_at_volatile(&self, bufs: &[&VolatileSlice<B>]) -> Result<usize>;
}

impl<'a, B: BitmapSlice, T: ReadWriteAtVolatile<B> + ?Sized> ReadWriteAtVolatile<B> for &'a T {
    fn read_vectored_at_volatile(&self, bufs: &[&VolatileSlice<B>]) -> Result<usize> {
        (**self).read_vectored_at_volatile(bufs)
    }

    fn write_vectored_at_volatile(&self, bufs: &[&VolatileSlice<B>]) -> Result<usize> {
        (**self).write_vectored_at_volatile(bufs)
    }
}

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

impl<B: BitmapSlice> ReadWriteAtVolatile<B> for BufferWrapper {
    fn read_vectored_at_volatile(&self, bufs: &[&VolatileSlice<B>]) -> Result<usize> {
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

    fn write_vectored_at_volatile(&self, bufs: &[&VolatileSlice<B>]) -> Result<usize> {
        let slice_guards: Vec<_> = bufs.iter().map(|s| s.ptr_guard()).collect();
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
            offset += iov.iov_len;
        }
        *self.buffer.borrow_mut() = opendal::Buffer::from(data);
        Ok(len)
    }
}
