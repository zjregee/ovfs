use std::io;
use std::ops::Deref;
use std::collections::VecDeque;

use virtio_queue::DescriptorChain;
use vm_memory::bitmap::{Bitmap, BitmapSlice};
use vm_memory::{Address, GuestMemory, GuestMemoryMmap, GuestMemoryRegion, VolatileMemory, VolatileMemoryError, VolatileSlice};

use crate::file_traits::FileReadWriteAtVolatile;

#[derive(Debug)]
pub enum Error {
    DescriptorChainOverflow,
    FindMemoryRegion,
    VolatileMemoryError(VolatileMemoryError),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use self::Error::*;

        match self {
            DescriptorChainOverflow => write!(f, "the combined length of all the buffers in a `DescriptorChain` would overflow"),
            FindMemoryRegion => write!(f, "no memory region for this address range"),
            VolatileMemoryError(e) => write!(f, "volatile memory error: {e}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

pub struct Reader<'a, B = ()> {
    buffer: DescriptorChainConsumer<'a, B>
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
        let buffers = desc_chain.readable()
            .map(|desc| {
                len = len.checked_add(desc.len() as usize)
                    .ok_or(Error::DescriptorChainOverflow)?;
                let region = mem.find_region(desc.addr())
                    .ok_or(Error::FindMemoryRegion)?;
                let offset = desc.addr()
                    .checked_sub(region.start_addr().raw_value())
                    .unwrap();
                region.deref()
                    .get_slice(offset.raw_value() as usize, desc.len() as usize)
                    .map_err(Error::VolatileMemoryError)
            })
            .collect::<Result<VecDeque<VolatileSlice<'a, B>>>>()?;
        Ok(Reader {
            buffer: DescriptorChainConsumer {
                buffers,
                bytes_consumed: 0,
            }
        })
    }

    pub fn read_to_at<F: FileReadWriteAtVolatile<B>>(
        &mut self,
        dst: F,
        count: usize,
        off: u64,
    ) -> io::Result<usize> {
        self.buffer.consume(count, |bufs| dst.write_vectored_at_volatile(bufs, off))
    }
}

pub struct Writer<'a, B = ()> {
    buffer: DescriptorChainConsumer<'a, B>
}

impl <'a, B: Bitmap + BitmapSlice + 'static> Writer<'a, B> {
    pub fn new<M>(
        mem: &'a GuestMemoryMmap<B>,
        desc_chain: DescriptorChain<M>,
    ) -> Result<Writer<'a, B>>
    where
        M: Deref,
        M::Target: GuestMemory + Sized,
    {
        let mut len: usize = 0;
        let buffers = desc_chain.writable()
            .map(|desc| {
                len = len.checked_add(desc.len() as usize)
                    .ok_or(Error::DescriptorChainOverflow)?;
                let region = mem.find_region(desc.addr())
                    .ok_or(Error::FindMemoryRegion)?;
                let offset = desc.addr().checked_sub(region.start_addr().raw_value()).unwrap();
                region
                    .deref()
                    .get_slice(offset.raw_value() as usize, desc.len() as usize)
                    .map_err(Error::VolatileMemoryError)
            })
            .collect::<Result<VecDeque<VolatileSlice<'a, B>>>>()?;
        Ok(Writer {
            buffer: DescriptorChainConsumer {
                buffers,
                bytes_consumed: 0,
            }
        })
    }

    pub fn write_from_at<F: FileReadWriteAtVolatile<B>>(
        &mut self,
        src: F,
        count: usize,
        off: u64,
    ) -> io::Result<usize> {
        self.buffer.consume(count, |bufs| src.read_vectored_at_volatile(bufs, off))
    }
}

struct DescriptorChainConsumer<'a, B> {
    buffers: VecDeque<VolatileSlice<'a, B>>,
    bytes_consumed: usize,
}

impl<'a, B: BitmapSlice> DescriptorChainConsumer<'a, B> {
    fn consume<F>(&mut self, count: usize, f: F) -> io::Result<usize>
    where
        F: FnOnce(&[&VolatileSlice<B>]) -> io::Result<usize>,
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
        let total_bytes_consumed = self.bytes_consumed
            .checked_add(bytes_consumed)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, Error::DescriptorChainOverflow))?;
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
}
