fn main() {
    println!("cargo::rustc-check-cfg=cfg(has_libwayland_client)");

    println!("cargo:rerun-if-changed=c/libwayland_probe.c");
    println!("cargo:rerun-if-changed=build.rs");

    match pkg_config::Config::new().atleast_version("1.0").probe("wayland-client") {
        Ok(lib) => {
            println!("cargo:rustc-cfg=has_libwayland_client");
            let mut build = cc::Build::new();
            build.file("c/libwayland_probe.c");
            for path in lib.include_paths {
                build.include(path);
            }
            build.compile("wayland_probe");
            println!("cargo:rustc-link-lib=wayland-client");
        }
        Err(e) => {
            println!("cargo:warning=libwayland-client not found: {}. Tests requiring it will be skipped.", e);
        }
    }
}
