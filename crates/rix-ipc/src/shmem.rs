
use memmap2::MmapMut;
use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::NamedTempFile;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ShmemError {
    #[error("I/O Error: {0}")]
    Io(#[from] io::Error),

    #[error("Invalid Shared Memory Header: magic bytes mismatch")]
    InvalidHeader,

    #[error("Unsupported shared memory version: {0}")]
    UnsupportedVersion(u32),

    #[error("Buffer index out of range: {0} (must be 0, 1, or 2)")]
    IndexOutOfRange(usize),

    #[error("Buffer too small: requested {requested}, allocated {allocated}")]
    BufferTooSmall { requested: usize, allocated: usize },

    #[error("Invalid dimensions: {0}x{1} bpp={2}")]
    InvalidDimensions(u32, u32, u32),
}

pub type Result<T> = std::result::Result<T, ShmemError>;

pub const SHMEM_MAGIC: [u8; 4] = *b"RIXF";
pub const SHMEM_VERSION: u32 = 1;
pub const HEADER_SIZE: usize = 128;
pub const NUM_BUFFERS: usize = 3;
pub const MAX_4K_WIDTH: u32 = 3840;
pub const MAX_4K_HEIGHT: u32 = 2160;
pub const RGBA_BYTES_PER_PIXEL: u32 = 4;

#[repr(C)]
pub struct RawFrameHeader {
    pub magic: [u8; 4],
    pub version: u32,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: u32,
    pub buffer_size: u64,
    pub active_front_index: AtomicU8,
    pub _reserved1: [u8; 7],
    pub frame_number: AtomicU64,
}

const _: () = assert!(std::mem::size_of::<RawFrameHeader>() <= HEADER_SIZE);
const _: () = assert!(std::mem::align_of::<RawFrameHeader>() <= 8);

pub struct SharedTripleBuffer {
    _file: File,
    mmap: MmapMut,
    buffer_size: usize,
    total_size: usize,
    backing_path: Option<PathBuf>,
}

fn validate_dims(width: u32, height: u32, bytes_per_pixel: u32) -> Result<(usize, usize)> {
    if width == 0 || height == 0 || bytes_per_pixel == 0 || bytes_per_pixel > 8 {
        return Err(ShmemError::InvalidDimensions(width, height, bytes_per_pixel));
    }
    if width > MAX_4K_WIDTH * 2 || height > MAX_4K_HEIGHT * 2 {
        return Err(ShmemError::InvalidDimensions(width, height, bytes_per_pixel));
    }
    let stride = (width as usize)
        .checked_mul(bytes_per_pixel as usize)
        .ok_or(ShmemError::InvalidDimensions(width, height, bytes_per_pixel))?;
    let buffer_size = stride
        .checked_mul(height as usize)
        .ok_or(ShmemError::InvalidDimensions(width, height, bytes_per_pixel))?;
    if buffer_size == 0 || buffer_size > 256 * 1024 * 1024 {
        return Err(ShmemError::InvalidDimensions(width, height, bytes_per_pixel));
    }
    let total = HEADER_SIZE
        .checked_add(NUM_BUFFERS.checked_mul(buffer_size).ok_or(
            ShmemError::InvalidDimensions(width, height, bytes_per_pixel),
        )?)
        .ok_or(ShmemError::InvalidDimensions(width, height, bytes_per_pixel))?;
    if total > 768 * 1024 * 1024 {
        return Err(ShmemError::InvalidDimensions(width, height, bytes_per_pixel));
    }
    Ok((stride, buffer_size))
}

fn init_header(mmap: &mut MmapMut, width: u32, height: u32, stride: usize, buffer_size: usize) {
    // SAFETY: `mmap` is freshly created with length >= HEADER_SIZE, page-aligned
    
    let header = unsafe { &mut *(mmap.as_mut_ptr() as *mut RawFrameHeader) };
    header.magic = SHMEM_MAGIC;
    header.version = SHMEM_VERSION;
    header.width = width;
    header.height = height;
    header.stride = stride as u32;
    header.format = 0;
    header.buffer_size = buffer_size as u64;
    header.active_front_index.store(0, Ordering::SeqCst);
    header.frame_number.store(0, Ordering::SeqCst);
    let hdr_len = std::mem::size_of::<RawFrameHeader>();
    if mmap.len() >= HEADER_SIZE && hdr_len < HEADER_SIZE {
        mmap[hdr_len..HEADER_SIZE].fill(0);
    }
}

impl SharedTripleBuffer {
    pub fn new(width: u32, height: u32, bytes_per_pixel: u32) -> Result<Self> {
        let (stride, buffer_size) = validate_dims(width, height, bytes_per_pixel)?;
        let total_size = HEADER_SIZE + NUM_BUFFERS * buffer_size;

        let temp_file = NamedTempFile::new()?;
        let file = temp_file.reopen()?;
        file.set_len(total_size as u64)?;
        // SAFETY: the file was freshly created, sized to `total_size`, and is not
        
        let mut mmap = unsafe { MmapMut::map_mut(&file)? };

        init_header(&mut mmap, width, height, stride, buffer_size);

        Ok(Self {
            _file: file,
            mmap,
            buffer_size,
            total_size,
            backing_path: None,
        })
    }

    pub fn new_persistent(
        width: u32,
        height: u32,
        bytes_per_pixel: u32,
        dir: Option<&Path>,
    ) -> Result<(Self, PathBuf)> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let (stride, buffer_size) = validate_dims(width, height, bytes_per_pixel)?;
        let total_size = HEADER_SIZE + NUM_BUFFERS * buffer_size;

        let base: PathBuf = dir.map_or_else(std::env::temp_dir, Path::to_path_buf);
        let _ = std::fs::create_dir_all(&base);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let uniq = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = base.join(format!(
            "rix-frame-{}-{}-{:x}.shm",
            std::process::id(),
            uniq,
            nanos & 0xffff_ffff
        ));

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)?;
        file.set_len(total_size as u64)?;
        // SAFETY: same justification as `new()`: fresh file, exact length,
        
        let mut mmap = unsafe { MmapMut::map_mut(&file)? };

        init_header(&mut mmap, width, height, stride, buffer_size);
        let _ = mmap.flush();

        Ok((
            Self {
                _file: file,
                mmap,
                buffer_size,
                total_size,
                backing_path: Some(path.clone()),
            },
            path,
        ))
    }

    pub fn open_shared(path: &Path) -> Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        Self::from_file(file)
    }

    pub fn from_file(file: File) -> Result<Self> {
        let meta = file.metadata()?;
        let total_size = meta.len() as usize;

        // SAFETY: the file handle is valid and `map_mut` spans exactly the file
        
        let mmap = unsafe { MmapMut::map_mut(&file)? };
        if mmap.len() < HEADER_SIZE {
            return Err(ShmemError::InvalidHeader);
        }

        // SAFETY: `mmap` is page-aligned (>= 8-byte alignment required) and
        
        let (magic_ok, version, buffer_size) = unsafe {
            let header = &*(mmap.as_ptr() as *const RawFrameHeader);
            (
                header.magic == SHMEM_MAGIC,
                header.version,
                header.buffer_size as usize,
            )
        };
        if !magic_ok {
            return Err(ShmemError::InvalidHeader);
        }
        if version != SHMEM_VERSION {
            return Err(ShmemError::UnsupportedVersion(version));
        }
        let expected_total = HEADER_SIZE + NUM_BUFFERS * buffer_size;
        if buffer_size == 0 || total_size != expected_total || mmap.len() != expected_total {
            return Err(ShmemError::BufferTooSmall {
                requested: expected_total,
                allocated: total_size,
            });
        }

        Ok(Self {
            _file: file,
            mmap,
            buffer_size,
            total_size,
            backing_path: None,
        })
    }

    #[inline]
    pub fn header(&self) -> &RawFrameHeader {
        debug_assert!(self.mmap.len() >= HEADER_SIZE);
        // SAFETY: mapping is page-aligned (satisfies `RawFrameHeader` alignment),
        
        unsafe { &*(self.mmap.as_ptr() as *const RawFrameHeader) }
    }

    pub fn backing_path(&self) -> Option<&Path> {
        self.backing_path.as_deref()
    }

    pub fn dimensions(&self) -> (u32, u32, u32, usize) {
        let h = self.header();
        (h.width, h.height, h.stride, self.buffer_size)
    }

    #[inline]
    pub fn active_front_index(&self) -> u8 {
        self.header().active_front_index.load(Ordering::Acquire)
    }

    #[inline]
    pub fn frame_number(&self) -> u64 {
        self.header().frame_number.load(Ordering::Acquire)
    }

    pub fn get_buffer_mut(&mut self, index: usize) -> Result<&mut [u8]> {
        if index >= NUM_BUFFERS {
            return Err(ShmemError::IndexOutOfRange(index));
        }
        let start = HEADER_SIZE + index * self.buffer_size;
        let end = start + self.buffer_size;
        if end > self.total_size || end > self.mmap.len() {
            return Err(ShmemError::BufferTooSmall {
                requested: end,
                allocated: self.total_size,
            });
        }
        Ok(&mut self.mmap[start..end])
    }

    pub fn get_buffer(&self, index: usize) -> Result<&[u8]> {
        if index >= NUM_BUFFERS {
            return Err(ShmemError::IndexOutOfRange(index));
        }
        let start = HEADER_SIZE + index * self.buffer_size;
        let end = start + self.buffer_size;
        if end > self.total_size || end > self.mmap.len() {
            return Err(ShmemError::BufferTooSmall {
                requested: end,
                allocated: self.total_size,
            });
        }
        Ok(&self.mmap[start..end])
    }

    pub fn commit_frame(&self, buffer_index: u8) -> u64 {
        debug_assert!((buffer_index as usize) < NUM_BUFFERS);
        let header = self.header();
        header.active_front_index.store(buffer_index, Ordering::Release);
        header.frame_number.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn flush(&self) -> Result<()> {
        self.mmap.flush()?;
        Ok(())
    }

    pub fn next_writable_buffer_index(&self) -> u8 {
        let front = self.active_front_index();
        (front + 1) % (NUM_BUFFERS as u8)
    }
}
