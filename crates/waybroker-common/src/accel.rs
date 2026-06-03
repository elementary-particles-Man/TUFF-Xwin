use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimdFlavor {
    Portable,
    Sse42,
    Avx2,
    Avx512f,
    Neon,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccelPolicy {
    pub accel_enabled: bool,
    pub simd_enabled: bool,
    pub vulkan_enabled: bool,
    pub avx2_available: bool,
    pub avx512f_available: bool,
    pub sse42_available: bool,
    pub neon_available: bool,
}

impl AccelPolicy {
    pub fn detect() -> Self {
        Self::detect_with(|key| std::env::var(key).ok())
    }

    pub(crate) fn detect_with(env_lookup: impl Fn(&str) -> Option<String>) -> Self {
        let disable_accel = parse_env_bool(env_lookup("TUFF_XWIN_DISABLE_ACCEL"));
        let disable_simd = disable_accel || parse_env_bool(env_lookup("TUFF_XWIN_DISABLE_SIMD"));
        let disable_vulkan =
            disable_accel || parse_env_bool(env_lookup("TUFF_XWIN_DISABLE_VULKAN"));

        let avx2_available = !disable_simd && detect_avx2();
        let avx512f_available = !disable_simd && detect_avx512f();
        let sse42_available = !disable_simd && detect_sse42();
        let neon_available = !disable_simd && detect_neon();

        Self {
            accel_enabled: !disable_accel,
            simd_enabled: !disable_simd,
            vulkan_enabled: !disable_vulkan,
            avx2_available,
            avx512f_available,
            sse42_available,
            neon_available,
        }
    }

    pub fn global() -> &'static Self {
        static POLICY: OnceLock<AccelPolicy> = OnceLock::new();
        POLICY.get_or_init(Self::detect)
    }

    pub fn selected_simd_flavor(&self) -> SimdFlavor {
        if !self.simd_enabled {
            return SimdFlavor::Portable;
        }

        if self.avx512f_available {
            SimdFlavor::Avx512f
        } else if self.avx2_available {
            SimdFlavor::Avx2
        } else if self.sse42_available {
            SimdFlavor::Sse42
        } else if self.neon_available {
            SimdFlavor::Neon
        } else {
            SimdFlavor::Portable
        }
    }

    pub fn prefers_vulkan(&self) -> bool {
        self.accel_enabled && self.vulkan_enabled
    }
}

pub fn global_accel_policy() -> &'static AccelPolicy {
    AccelPolicy::global()
}

fn parse_env_bool(value: Option<String>) -> bool {
    match value.as_deref().map(str::trim).map(str::to_ascii_lowercase) {
        Some(ref v) if matches!(v.as_str(), "1" | "true" | "yes" | "on") => true,
        Some(ref v) if matches!(v.as_str(), "0" | "false" | "no" | "off") => false,
        _ => false,
    }
}

#[cfg(target_arch = "x86_64")]
fn detect_avx2() -> bool {
    std::arch::is_x86_feature_detected!("avx2")
}

#[cfg(not(target_arch = "x86_64"))]
fn detect_avx2() -> bool {
    false
}

#[cfg(target_arch = "x86_64")]
fn detect_avx512f() -> bool {
    std::arch::is_x86_feature_detected!("avx512f")
}

#[cfg(not(target_arch = "x86_64"))]
fn detect_avx512f() -> bool {
    false
}

#[cfg(target_arch = "x86_64")]
fn detect_sse42() -> bool {
    std::arch::is_x86_feature_detected!("sse4.2")
}

#[cfg(not(target_arch = "x86_64"))]
fn detect_sse42() -> bool {
    false
}

#[cfg(target_arch = "aarch64")]
fn detect_neon() -> bool {
    std::arch::is_aarch64_feature_detected!("neon")
}

#[cfg(not(target_arch = "aarch64"))]
fn detect_neon() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::{AccelPolicy, SimdFlavor};

    #[test]
    fn parses_disable_flags() {
        let policy = AccelPolicy::detect_with(|key| match key {
            "TUFF_XWIN_DISABLE_ACCEL" => Some("1".to_string()),
            "TUFF_XWIN_DISABLE_SIMD" => Some("0".to_string()),
            "TUFF_XWIN_DISABLE_VULKAN" => Some("0".to_string()),
            _ => None,
        });

        assert!(!policy.accel_enabled);
        assert!(!policy.simd_enabled);
        assert!(!policy.vulkan_enabled);
        assert_eq!(policy.selected_simd_flavor(), SimdFlavor::Portable);
    }

    #[test]
    fn honors_simd_override_without_disabling_vulkan() {
        let policy = AccelPolicy::detect_with(|key| match key {
            "TUFF_XWIN_DISABLE_SIMD" => Some("true".to_string()),
            _ => None,
        });

        assert!(!policy.simd_enabled);
        assert!(policy.vulkan_enabled);
        assert_eq!(policy.selected_simd_flavor(), SimdFlavor::Portable);
    }
}
