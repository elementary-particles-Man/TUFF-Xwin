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

    // Verify state
    // 1. Surface created
    let surface_id = wayland_wire::WaylandObjectId(5);
    let surf = server.core.surfaces.surfaces.get(&surface_id).expect("surface should exist");

    // 2. SHM pool created (ID 6 in probe)
    assert!(!server.core.shm.pools.is_empty());

    // 3. Commit happened (commit_id should be > 0)
    // HeadlessWireCore doesn't track commit_count directly on surface instance yet, 
    // but current buffer_id is updated.
    assert!(surf.current.buffer_id.is_some());
    assert_eq!(surf.current.buffer_id.unwrap().0, 7); // buffer id 7 in probe
}

