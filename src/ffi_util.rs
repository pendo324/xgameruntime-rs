//! Small FFI-adjacent helpers shared across modules that don't otherwise share a parent.

/// Reinterpret a pointer (opaque or function) as a function pointer of type `F`.
///
/// # Safety
/// `ptr` must be non-null and actually point to a function whose ABI and signature match
/// `F` exactly.
pub(crate) unsafe fn fn_ptr_cast<S: Copy, F: Copy>(ptr: S) -> F {
    // `transmute` can't be used here: it requires statically-known, equal-sized types at
    // the definition site, which a generic `F` doesn't satisfy. `transmute_copy` is the
    // standard way to transmute into a generic destination type.
    // SAFETY: this fn's own `# Safety` doc is the precondition callers must uphold.
    unsafe { std::mem::transmute_copy::<S, F>(&ptr) }
}

/// Copy `src` into a caller-supplied output buffer.
///
/// # Safety
/// `dst` must be valid for writes of `src.len()` bytes and must not overlap `src`.
pub(crate) unsafe fn write_out_bytes(src: &[u8], dst: *mut u8) {
    // SAFETY: this fn's own `# Safety` doc is the precondition callers must uphold.
    unsafe { std::ptr::copy_nonoverlapping(src.as_ptr(), dst, src.len()) };
}
