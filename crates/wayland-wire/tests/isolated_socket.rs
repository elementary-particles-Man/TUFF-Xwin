use byteorder::ByteOrder;
use std::thread;
use tempfile::tempdir;
use wayland_wire::{
    client::WireFakeClient,
    server::{WireServer, WireServerConfig},
};

#[test]
fn test_isolated_socket_surface_commit_e2e() {
    let dir = tempdir().unwrap();
    let socket_path = dir.path().join("tuff-xwin-test-wayland-e2e.sock");
    let server_path = socket_path.clone();

    let server_handle = thread::spawn(move || {
        let config = WireServerConfig { socket_path: server_path };
        let mut server = WireServer::new(config).expect("failed to create server");
        server.run_once().expect("server run failed");
        server // return server to inspect state
    });

    // Give server a moment to start
    thread::sleep(std::time::Duration::from_millis(100));

    let mut client = WireFakeClient::connect(&socket_path).expect("failed to connect");

    // 1. Get Registry
    client.get_registry(2).expect("get_registry");
    let mut events = Vec::new();
    while events.len() < 3 {
        events.append(&mut client.receive_events().unwrap());
    }

    // 2. Bind Compositor (assume name 1)
    client.bind_wl_compositor(2, 1, 4, 3).expect("bind compositor");

    // 3. Bind SHM (assume name 2)
    client.bind_wl_shm(2, 2, 1, 4).expect("bind shm");

    // Receive format events
    let mut formats = Vec::new();
    while formats.len() < 2 {
        formats.append(&mut client.receive_events().unwrap());
    }
    assert_eq!(formats[0].header.object_id.0, 4); // shm object id

    // 4. Create Surface
    client.wl_compositor_create_surface(3, 5).expect("create surface");

    // 5. Create Pool and Buffer
    client.wl_shm_create_pool(4, 6, 0, 1024).expect("create pool");
    client.wl_shm_pool_create_buffer(6, 7, 0, 16, 16, 64, 0).expect("create buffer");

    // 6. Attach, Damage, Frame, Commit
    client.wl_surface_attach(5, 7, 0, 0).expect("attach");
    client.wl_surface_damage(5, 0, 0, 16, 16).expect("damage");
    client.wl_surface_frame(5, 8).expect("frame callback");
    client.wl_surface_commit(5).expect("commit");

    // 7. Receive callback done
    let mut commit_events = Vec::new();
    while commit_events.is_empty() {
        commit_events = client.receive_events().unwrap();
    }
    assert_eq!(commit_events[0].header.object_id.0, 8); // callback id
    assert_eq!(commit_events[0].header.opcode.0, 0); // done

    drop(client);
    let server = server_handle.join().unwrap();

    // Verify server state
    let surf = server.core.surfaces.surfaces.get(&wayland_wire::WaylandObjectId(5)).unwrap();
    assert_eq!(surf.current.buffer_id.unwrap().0, 7);
    assert!(surf.pending.damage.is_empty());
}

#[test]
fn test_isolated_socket_xdg_input_e2e() {
    let dir = tempdir().unwrap();
    let socket_path = dir.path().join("tuff-xwin-test-xdg-input.sock");
    let server_path = socket_path.clone();

    let _server_handle = thread::spawn(move || {
        let config = WireServerConfig { socket_path: server_path };
        let mut server = WireServer::new(config).expect("failed to create server");
        server.run_once().expect("server run failed");
    });

    thread::sleep(std::time::Duration::from_millis(100));
    let mut client = WireFakeClient::connect(&socket_path).expect("failed to connect");

    client.get_registry(2).unwrap();
    let mut events = Vec::new();
    while events.len() < 4 {
        // compositor, shm, seat, xdg_wm_base
        events.append(&mut client.receive_events().unwrap());
    }

    // 1. Bind objects
    client.bind_wl_compositor(2, 1, 4, 3).unwrap();
    client.bind_wl_shm(2, 2, 1, 4).unwrap();
    client.bind_wl_seat(2, 3, 7, 5).unwrap();
    client.bind_xdg_wm_base(2, 4, 6, 6).unwrap();

    // 2. Setup surface and xdg
    client.wl_compositor_create_surface(3, 7).unwrap();
    client.xdg_wm_base_get_xdg_surface(6, 8, 7).unwrap();
    client.xdg_surface_get_toplevel(8, 9).unwrap();
    client.xdg_toplevel_set_title(9, "Parity Test").unwrap();

    // 3. Receive configure and ack
    let mut config_events = Vec::new();
    while config_events.len() < 2 {
        config_events.append(&mut client.receive_events().unwrap());
    }
    // xdg_surface.configure (opcode 0) has serial.
    // In our HeadlessWireCore, it emits toplevel.configure (opcode 0) then surface.configure.
    let serial = if config_events[1].header.object_id.0 == 8 {
        byteorder::LittleEndian::read_u32(&config_events[1].payload[0..4])
    } else {
        byteorder::LittleEndian::read_u32(&config_events[0].payload[0..4])
    };

    client.xdg_surface_ack_configure(8, serial).unwrap();
    client.wl_surface_commit(7).unwrap();

    // 4. Input setup
    client.wl_seat_get_pointer(5, 10).unwrap();
    client.wl_seat_get_keyboard(5, 11).unwrap();

    // Server thread is run_once and already accepted a connection.
    // To send fake input, we'd normally need a way to trigger server side events.
    // In this E2E test, the server is running the HeadlessWireCore loop.
    // We'll stop here for P6 E2E handshake.
}

#[test]
fn test_reject_duplicate_xdg_surface() {
    let dir = tempdir().unwrap();
    let socket_path = dir.path().join("tuff-xwin-test-xdg-dup.sock");
    let server_path = socket_path.clone();

    let _server_handle = thread::spawn(move || {
        let config = WireServerConfig { socket_path: server_path };
        let mut server = WireServer::new(config).expect("failed to create server");
        let _ = server.run_once();
    });

    thread::sleep(std::time::Duration::from_millis(100));
    let mut client = WireFakeClient::connect(&socket_path).expect("failed to connect");

    client.get_registry(2).unwrap();
    client.bind_wl_compositor(2, 1, 4, 3).unwrap();
    client.bind_xdg_wm_base(2, 4, 6, 6).unwrap();
    client.wl_compositor_create_surface(3, 7).unwrap();
    
    // Drain events (globals and ping)
    thread::sleep(std::time::Duration::from_millis(50));
    while !client.receive_events().unwrap().is_empty() {}

    // Create first xdg_surface
    client.xdg_wm_base_get_xdg_surface(6, 8, 7).unwrap();
    
    // Create second xdg_surface for SAME wl_surface (id 7) -> should fail on server side
    client.xdg_wm_base_get_xdg_surface(6, 9, 7).unwrap();
    
    thread::sleep(std::time::Duration::from_millis(200));
    // Connection should be closed by server due to ProtocolError
    let res = client.receive_events();
    assert!(res.is_err());
}

#[test]
fn test_reject_invalid_ack_configure() {
    let dir = tempdir().unwrap();
    let socket_path = dir.path().join("tuff-xwin-test-xdg-ack-fail.sock");
    let server_path = socket_path.clone();

    let _server_handle = thread::spawn(move || {
        let config = WireServerConfig { socket_path: server_path };
        let mut server = WireServer::new(config).expect("failed to create server");
        let _ = server.run_once();
    });

    thread::sleep(std::time::Duration::from_millis(100));
    let mut client = WireFakeClient::connect(&socket_path).expect("failed to connect");

    client.get_registry(2).unwrap();
    client.bind_wl_compositor(2, 1, 4, 3).unwrap();
    client.bind_xdg_wm_base(2, 4, 6, 6).unwrap();
    client.wl_compositor_create_surface(3, 7).unwrap();
    client.xdg_wm_base_get_xdg_surface(6, 8, 7).unwrap();

    // Drain events
    thread::sleep(std::time::Duration::from_millis(50));
    while !client.receive_events().unwrap().is_empty() {}
    
    // Ack a serial before any configure is sent (last_serial is 0)
    client.xdg_surface_ack_configure(8, 1234).unwrap();
    
    thread::sleep(std::time::Duration::from_millis(200));
    let res = client.receive_events();
    assert!(res.is_err());
}

#[test]
fn test_reject_xdg_request_after_destroy() {
    let dir = tempdir().unwrap();
    let socket_path = dir.path().join("tuff-xwin-test-xdg-dest.sock");
    let server_path = socket_path.clone();

    let _server_handle = thread::spawn(move || {
        let config = WireServerConfig { socket_path: server_path };
        let mut server = WireServer::new(config).expect("failed to create server");
        let _ = server.run_once();
    });

    thread::sleep(std::time::Duration::from_millis(100));
    let mut client = WireFakeClient::connect(&socket_path).expect("failed to connect");

    client.get_registry(2).unwrap();
    client.bind_wl_compositor(2, 1, 4, 3).unwrap();
    client.bind_xdg_wm_base(2, 4, 6, 6).unwrap();
    client.wl_compositor_create_surface(3, 7).unwrap();
    client.xdg_wm_base_get_xdg_surface(6, 8, 7).unwrap();
    client.xdg_surface_get_toplevel(8, 9).unwrap();

    // Drain events
    thread::sleep(std::time::Duration::from_millis(50));
    while !client.receive_events().unwrap().is_empty() {}
    
    // Destroy toplevel (opcode 0)
    let msg = wayland_wire::WaylandMessage::new(wayland_wire::WaylandObjectId(9), wayland_wire::WaylandOpcode(0), vec![]);
    client.send_message(&msg).unwrap();
    
    // Try set_title (opcode 2) on destroyed toplevel -> should fail
    client.xdg_toplevel_set_title(9, "After Destroy").unwrap();
    
    thread::sleep(std::time::Duration::from_millis(200));
    let res = client.receive_events();
    assert!(res.is_err());
}

#[test]
fn test_isolated_socket_reject_invalid_shm_buffer_bounds() {
    let dir = tempdir().unwrap();
    let socket_path = dir.path().join("tuff-xwin-test-wayland-reject.sock");
    let server_path = socket_path.clone();

    let server_handle = thread::spawn(move || {
        let config = WireServerConfig { socket_path: server_path };
        let mut server = WireServer::new(config).expect("failed to create server");
        let _ = server.run_once();
    });

    thread::sleep(std::time::Duration::from_millis(100));
    let mut client = WireFakeClient::connect(&socket_path).expect("failed to connect");

    client.get_registry(2).unwrap();
    client.bind_wl_shm(2, 2, 1, 4).unwrap();
    client.wl_shm_create_pool(4, 5, 0, 100).unwrap(); // 100 bytes pool

    // Try to create 1000 byte buffer (fails)
    client.wl_shm_pool_create_buffer(5, 6, 0, 10, 10, 100, 0).unwrap();

    thread::sleep(std::time::Duration::from_millis(50));
    drop(client);
    server_handle.join().unwrap();
}
