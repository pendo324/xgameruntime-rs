use windows_core::HRESULT;

pub(crate) const S_OK: HRESULT = HRESULT(0);
pub(crate) const E_PENDING: HRESULT = HRESULT(0x8000000Au32 as i32);
pub(crate) const E_ABORT: HRESULT = HRESULT(0x80004004u32 as i32);
pub(crate) const E_NOINTERFACE: HRESULT = HRESULT(0x80004002u32 as i32);
pub(crate) const E_POINTER: HRESULT = HRESULT(0x80004003u32 as i32);
pub(crate) const E_INVALIDARG: HRESULT = HRESULT(0x80070057u32 as i32);
pub(crate) const E_NOT_SUFFICIENT_BUFFER: HRESULT = HRESULT(0x8007007Au32 as i32);
/// What a store service's 401 becomes at the GDK boundary. The XStore API has no way to
/// say "succeeded, but there is no value", so an absent key has to surface as a failure.
pub(crate) const E_ACCESSDENIED: HRESULT = HRESULT(0x80070005u32 as i32);
/// Returned when an XAsync entry point is called out of order, e.g. asking for a result
/// before the call has completed.
pub(crate) const E_ILLEGAL_METHOD_CALL: HRESULT = HRESULT(0x8000000Eu32 as i32);
