use std::thread;
use tempfile::tempdir;
use wayland_wire::{
    client::WireFakeClient,
    server::{WireServer, WireServerConfig},
    WaylandObjectId, WaylandOpcode,
};

#[test]
fn test_isolated_socket_e2e() {
    let dir = tempdir().unwrap();
    let socket_path = dir.path().join("tuff-xwin-test-wayland.sock");
    let server_path = socket_path.clone();

    let server_thread = thread::spawn(move || {
        let config = WireServerConfig { socket_path: server_path };
        let mut server = WireServer::new(config).expect("failed to create server");
        server.run_once().expect("server run failed");
    });

    // Give server a moment to start
    thread::sleep(std::time::Duration::from_millis(50));

    let mut client = WireFakeClient::connect(&socket_path).expect("failed to connect");

    // 1. Get Registry
    client.get_registry(2).expect("get_registry");

    // 2. Receive globals
    let mut events = Vec::new();
    while events.len() < 3 {
        let mut evs = client.receive_events().expect("receive events");
        events.append(&mut evs);
        if events.len() < 3 {
            thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].header.object_id, WaylandObjectId(2));

    // 3. Sync
    client.sync(3).expect("sync");
    let mut sync_events = Vec::new();
    while sync_events.is_empty() {
        sync_events = client.receive_events().expect("receive sync");
    }
    assert_eq!(sync_events[0].header.object_id, WaylandObjectId(3));
    assert_eq!(sync_events[0].header.opcode, WaylandOpcode(0)); // wl_callback.done

    // Client closes connection, server loop in run_once should exit
    drop(client);
    server_thread.join().expect("server thread panicked");
}
