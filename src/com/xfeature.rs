use windows_core::{IUnknown, implement, interface};
#[interface("8836fe87-edb9-4fe3-8dad-05f0d2cd5b40")]
pub unsafe trait IXFeature: IUnknown {
    pub unsafe fn XGameRuntimeIsFeatureAvailable(&self, feature: u32) -> bool;
}

#[implement(IXFeature)]
pub struct XFeature;

impl IXFeature_Impl for XFeature_Impl {
    /// Every feature reports available.
    ///
    /// A title that is told a feature is missing takes a fallback path we have not
    /// implemented either, so claiming absence buys nothing. The honest "not implemented"
    /// lives at the individual API, which returns `E_NOTIMPL`.
    unsafe fn XGameRuntimeIsFeatureAvailable(&self, _feature: u32) -> bool {
        true
    }
}
