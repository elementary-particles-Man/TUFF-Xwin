use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::Path;

#[cfg(has_libwayland_client)]
extern "C" {
    fn run_libwayland_probe(fd: std::os::raw::c_int) -> std::os::raw::c_int;
}

pub fn probe_libwayland_client<P: AsRef<Path>>(_socket_path: P) -> anyhow::Result<()> {
    #[cfg(not(has_libwayland_client))]
    {
        println!("cargo:warning=libwayland-client not available, skipping probe.");
        return Ok(());
    }

    #[cfg(has_libwayland_client)]
    {
        use std::os::unix::io::AsRawFd;
        use std::os::unix::net::UnixStream;

        let stream = UnixStream::connect(_socket_path)?;
        let fd = stream.as_raw_fd();

        let result = unsafe { run_libwayland_probe(fd) };
        if result == 0 {
            Ok(())
        } else {
            anyhow::bail!("libwayland probe failed with code {}", result)
        }
    }
}

pub fn has_libwayland_client() -> bool {
    cfg!(has_libwayland_client)
}
