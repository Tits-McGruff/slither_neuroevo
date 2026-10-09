//! Return unused allocator pages after cold generation allocations are released.
//!
//! The Linux/glibc profile showed stable live allocations with retained free
//! pages. This maintenance belongs after the generation swap and worker rebind,
//! when the prior population has been dropped. Fixed steps do not call it.

#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[link(name = "c")]
extern "C" {
    /// GNU allocator maintenance primitive with a size_t padding argument.
    fn malloc_trim(pad: usize) -> std::ffi::c_int;
}

/// Ask the supported GNU allocator to return whole unused pages to the OS.
/// Live buffers, game state and RNG are unaffected; a zero return simply means
/// there were no releasable pages. Other targets retain their allocator policy.
pub(crate) fn release_unused_pages() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: The GNU ABI declares malloc_trim(size_t). Zero is valid padding.
    // malloc_trim is MT-safe and operates only on freed allocator-owned pages;
    // it takes no application pointer and cannot invalidate live allocations.
    // See https://man7.org/linux/man-pages/man3/malloc_trim.3.html.
    unsafe {
        malloc_trim(0);
    }
}
