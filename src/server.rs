use std::sync::atomic::AtomicU64;

use crate::filesystem::*;
use crate::placeholder::*;
use crate::file_traits::*;
use crate::descriptor_utils::*;

struct ZcReader<'a>(Reader<'a>);

impl<'a> ZeroCopyReader for ZcReader<'a> {
    fn read_to(&mut self, f: &BufferWrapper, count: usize, off: u64) -> std::io::Result<usize> {
        self.0.read_to_at(f, count, off)
    }
}

struct ZcWriter<'a>(Writer<'a>);

impl<'a> ZeroCopyWriter for ZcWriter<'a> {
    fn write_from(&mut self, f: &BufferWrapper, count: usize, off: u64) -> std::io::Result<usize> {
        self.0.write_from_at(f, count, off)
    }
}

pub struct Server<F: FileSystem + Sync> {
    fs: F,
    options: AtomicU64,
}

impl<F: FileSystem + Sync> Server<F> {
    pub fn new(fs: F) -> Server<F> {
        Server {
            fs,
            options: AtomicU64::new(FsOptions::empty().bits()),
        }
    }
}
