
use std::ptr;

pub(crate) fn alloc_code(bytes: &[u8]) -> Option<*const u8> {
    if bytes.is_empty() {
        return None;
    }
    let page = page_size();
    let size = bytes.len().div_ceil(page) * page;

    // SAFETY: plain anonymous mapping with a valid size; the returned pointer
    
    let raw = unsafe {
        libc::mmap(
            ptr::null_mut(),
            size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        )
    };
    if raw == libc::MAP_FAILED {
        return None;
    }

    // SAFETY: `raw` points to a fresh mapping of `size` bytes, which is at
    
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), raw.cast::<u8>(), bytes.len());
    }

    // SAFETY: `raw`/`size` describe the mapping created above.
    let rc = unsafe { libc::mprotect(raw, size, libc::PROT_READ | libc::PROT_EXEC) };
    if rc != 0 {
        return Some(raw.cast::<u8>());
    }

    Some(raw.cast::<u8>())
}

pub(crate) fn register_code(ptr: *const u8) {
    use std::sync::Mutex;
    use std::sync::OnceLock;

    static REGISTRY: OnceLock<Mutex<Vec<usize>>> = OnceLock::new();
    let registry = REGISTRY.get_or_init(|| Mutex::new(Vec::new()));
    if let Ok(mut guard) = registry.lock() {
        guard.push(ptr as usize);
    }
}

fn page_size() -> usize {
    // SAFETY: `sysconf` with a valid parameter is thread-safe and cannot
    
    let value = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if value > 0 {
        value as usize
    } else {
        4096
    }
}
