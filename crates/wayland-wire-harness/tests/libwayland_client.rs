use std::thread;
use tempfile::tempdir;
use wayland_wire::server::{WireServer, WireServerConfig};
use wayland_wire_harness::{has_libwayland_client, probe_libwayland_client};

#[test]
fn test_libwayland_client_harness_skips_without_dependency() {
    if !has_libwayland_client() {
        println!("INFO: libwayland-client not detected. Skipping test.");
        return;
    }

    let dir = tempdir().unwrap();
    let socket_path = dir.path().join("tuff-xwin-libwayland-skip-test.sock");
    let server_path = socket_path.clone();

    let server_handle = thread::spawn(move || {
        let config = WireServerConfig { socket_path: server_path };
        let mut server = WireServer::new(config).expect("failed to create server");
        server.run_once().expect("server run failed");
    });

    thread::sleep(std::time::Duration::from_millis(100));

    probe_libwayland_client(&socket_path).expect("probe failed");
    server_handle.join().expect("server thread panicked");
}

#[test]
fn test_libwayland_client_full_sequence() {
    if !has_libwayland_client() {
        return;
    }

    let dir = tempdir().unwrap();
    let socket_path = dir.path().join("tuff-xwin-libwayland-full-test.sock");
    let server_path = socket_path.clone();

    let server_handle = thread::spawn(move || {
        let config = WireServerConfig { socket_path: server_path };
        let mut server = WireServer::new(config).expect("failed to create server");
        server.run_once().expect("server run failed");
        server
    });

    thread::sleep(std::time::Duration::from_millis(100));

    probe_libwayland_client(&socket_path).expect("probe failed");

    let server = server_handle.join().expect("server thread panicked");

    // Verify surface was created
    // The probe creates 1 surface
    assert!(!server.core.surfaces.surfaces.is_empty());
}
