use std::cmp::min;
use std::collections::VecDeque;
use std::io::Read;
use std::io::Write;
use std::io::{self};
use std::mem::size_of;
use std::mem::MaybeUninit;
use std::ops::Deref;
use std::ptr::copy_nonoverlapping;

use virtio_queue::DescriptorChain;
use vm_memory::bitmap::Bitmap;
use vm_memory::bitmap::BitmapSlice;
use vm_memory::Address;
use vm_memory::ByteValued;
use vm_memory::GuestMemory;
use vm_memory::GuestMemoryMmap;
use vm_memory::GuestMemoryRegion;
use vm_memory::VolatileMemory;
use vm_memory::VolatileSlice;

use crate::buffer::ReadWriteAtVolatile;
use crate::error::*;

struct DescriptorChainConsumer<'a, B> {
    buffers: VecDeque<VolatileSlice<'a, B>>,
    bytes_consumed: usize,
}

impl<'a, B: BitmapSlice> DescriptorChainConsumer<'a, B> {
    fn bytes_consumed(&self) -> usize {
        self.bytes_consumed
    }

    fn consume<F>(&mut self, count: usize, f: F) -> Result<usize>
    where
        F: FnOnce(&[&VolatileSlice<B>]) -> Result<usize>,
    {
        let mut len = 0;
        let mut bufs = Vec::with_capacity(self.buffers.len());
        for vs in &self.buffers {
            if len >= count {
                break;
            }
            bufs.push(vs);
            let remain = count - len;
            if remain < vs.len() {
                len += remain;
            } else {
                len += vs.len();
            }
        }
        if bufs.is_empty() {
            return Ok(0);
        }
        let bytes_consumed = f(&bufs)?;
        let total_bytes_consumed =
            self.bytes_consumed
                .checked_add(bytes_consumed)
                .ok_or(new_vhost_user_fs_error(
                    "the combined length of all the buffers in DescriptorChain would overflow",
                    None,
                ))?;
        let mut remain = bytes_consumed;
        while let Some(vs) = self.buffers.pop_front() {
            if remain < vs.len() {
                self.buffers.push_front(vs.offset(remain).unwrap());
                break;
            }
            remain -= vs.len();
        }
        self.bytes_consumed = total_bytes_consumed;
        Ok(bytes_consumed)
    }

    fn split_at(&mut self, offset: usize) -> Result<DescriptorChainConsumer<'a, B>> {
        let mut remain = offset;
        let pos = self.buffers.iter().position(|vs| {
            if remain < vs.len() {
                true
            } else {
                remain -= vs.len();
                false
            }
        });
        if let Some(at) = pos {
            let mut other = self.buffers.split_off(at);
            if remain > 0 {
                let front = other.pop_front().expect("empty VecDeque after split");
                self.buffers.push_back(
                    front
                        .subslice(0, remain)
                        .map_err(|_| new_vhost_user_fs_error("volatile memory error", None))?,
                );
                other.push_front(
                    front
                        .offset(remain)
                        .map_err(|_| new_vhost_user_fs_error("volatile memory error", None))?,
                );
            }
            Ok(DescriptorChainConsumer {
                buffers: other,
                bytes_consumed: 0,
            })
        } else if remain == 0 {
            Ok(DescriptorChainConsumer {
                buffers: VecDeque::new(),
                bytes_consumed: 0,
            })
        } else {
            Err(new_vhost_user_fs_error(
                "DescriptorChain split is out of bounds",
                None,
            ))
        }
    }
}

pub struct Reader<'a, B = ()> {
    buffer: DescriptorChainConsumer<'a, B>,
}

impl<'a, B: Bitmap + BitmapSlice + 'static> Reader<'a, B> {
    pub fn new<M>(
        mem: &'a GuestMemoryMmap<B>,
        desc_chain: DescriptorChain<M>,
    ) -> Result<Reader<'a, B>>
    where
        M: Deref,
        M::Target: GuestMemory + Sized,
    {
        let mut len: usize = 0;
        let buffers = desc_chain
            .readable()
            .map(|desc| {
                len = len
                    .checked_add(desc.len() as usize)
                    .ok_or(new_vhost_user_fs_error(
                        "the combined length of all the buffers in DescriptorChain would overflow",
                        None,
                    ))?;
                let region = mem.find_region(desc.addr()).ok_or(new_vhost_user_fs_error(
                    "no memory region for this address range",
                    None,
                ))?;
                let offset = desc
                    .addr()
                    .checked_sub(region.start_addr().raw_value())
                    .unwrap();
                region
                    .deref()
                    .get_slice(offset.raw_value() as usize, desc.len() as usize)
                    .map_err(|err| {
                        new_vhost_user_fs_error("volatile memory error", Some(err.into()))
                    })
            })
            .collect::<Result<VecDeque<VolatileSlice<'a, B>>>>()?;
        Ok(Reader {
            buffer: DescriptorChainConsumer {
                buffers,
                bytes_consumed: 0,
            },
        })
    }

    pub fn read_obj<T: ByteValued>(&mut self) -> io::Result<T> {
        let mut obj = MaybeUninit::<T>::uninit();
        let buf =
            unsafe { std::slice::from_raw_parts_mut(obj.as_mut_ptr() as *mut u8, size_of::<T>()) };
        self.read_exact(buf)?;
        Ok(unsafe { obj.assume_init() })
    }

    pub fn read_to_at<F: ReadWriteAtVolatile<B>>(
        &mut self,
        dst: F,
        count: usize,
    ) -> io::Result<usize> {
        self.buffer
            .consume(count, |bufs| dst.write_vectored_at_volatile(bufs))
            .map_err(|err| err.into())
    }
}

impl<'a, B: BitmapSlice> io::Read for Reader<'a, B> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.buffer
            .consume(buf.len(), |bufs| {
                let mut rem = buf;
                let mut total = 0;
                for vs in bufs {
                    let copy_len = min(rem.len(), vs.len());
                    unsafe {
                        copy_nonoverlapping(vs.ptr_guard().as_ptr(), rem.as_mut_ptr(), copy_len);
                    }
                    rem = &mut rem[copy_len..];
                    total += copy_len;
                }
                Ok(total)
            })
            .map_err(|err| err.into())
    }
}

pub struct Writer<'a, B = ()> {
    buffer: DescriptorChainConsumer<'a, B>,
}

impl<'a, B: Bitmap + BitmapSlice + 'static> Writer<'a, B> {
    pub fn new<M>(
        mem: &'a GuestMemoryMmap<B>,
        desc_chain: DescriptorChain<M>,
    ) -> Result<Writer<'a, B>>
    where
        M: Deref,
        M::Target: GuestMemory + Sized,
    {
        let mut len: usize = 0;
        let buffers = desc_chain
            .writable()
            .map(|desc| {
                len = len
                    .checked_add(desc.len() as usize)
                    .ok_or(new_vhost_user_fs_error(
                        "the combined length of all the buffers in DescriptorChain would overflow",
                        None,
                    ))?;
                let region = mem.find_region(desc.addr()).ok_or(new_vhost_user_fs_error(
                    "no memory region for this address range",
                    None,
                ))?;
                let offset = desc
                    .addr()
                    .checked_sub(region.start_addr().raw_value())
                    .unwrap();
                region
                    .deref()
                    .get_slice(offset.raw_value() as usize, desc.len() as usize)
                    .map_err(|err| {
                        new_vhost_user_fs_error("volatile memory error", Some(err.into()))
                    })
            })
            .collect::<Result<VecDeque<VolatileSlice<'a, B>>>>()?;
        Ok(Writer {
            buffer: DescriptorChainConsumer {
                buffers,
                bytes_consumed: 0,
            },
        })
    }

    pub fn bytes_written(&self) -> usize {
        self.buffer.bytes_consumed()
    }

    pub fn split_at(&mut self, offset: usize) -> Result<Writer<'a, B>> {
        self.buffer.split_at(offset).map(|buffer| Writer { buffer })
    }

    pub fn write_from_at<F: ReadWriteAtVolatile<B>>(
        &mut self,
        src: F,
        count: usize,
    ) -> io::Result<usize> {
        self.buffer
            .consume(count, |bufs| src.read_vectored_at_volatile(bufs))
            .map_err(|err| err.into())
    }
}

impl<'a, B: BitmapSlice> Write for Writer<'a, B> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buffer
            .consume(buf.len(), |bufs| {
                let mut rem = buf;
                let mut total = 0;
                for vs in bufs {
                    let copy_len = min(rem.len(), vs.len());
                    unsafe {
                        copy_nonoverlapping(rem.as_ptr(), vs.ptr_guard_mut().as_ptr(), copy_len);
                    }
                    vs.bitmap().mark_dirty(0, copy_len);
                    rem = &rem[copy_len..];
                    total += copy_len;
                }
                Ok(total)
            })
            .map_err(|err| err.into())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
