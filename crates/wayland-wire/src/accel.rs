pub use waybroker_common::{AccelPolicy, SimdFlavor};

pub fn global_accel_policy() -> &'static AccelPolicy {
    AccelPolicy::global()
}

pub fn selected_simd_flavor() -> SimdFlavor {
    global_accel_policy().selected_simd_flavor()
}

pub fn vulkan_enabled() -> bool {
    global_accel_policy().prefers_vulkan()
}
