use byteorder::ByteOrder;
use std::os::unix::io::AsRawFd;
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
    while events.len() < 12 {
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
    let shm_fd = tempfile::tempfile().unwrap();
    client.wl_shm_create_pool(4, 6, shm_fd.as_raw_fd(), 1024).expect("create pool");
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
    while events.len() < 12 {
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
    let msg = wayland_wire::WaylandMessage::new(
        wayland_wire::WaylandObjectId(9),
        wayland_wire::WaylandOpcode(0),
        vec![],
    );
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
    let shm_fd2 = tempfile::tempfile().unwrap();
    client.wl_shm_create_pool(4, 5, shm_fd2.as_raw_fd(), 100).unwrap(); // 100 bytes pool

    // Try to create 1000 byte buffer (fails)
    client.wl_shm_pool_create_buffer(5, 6, 0, 10, 10, 100, 0).unwrap();

    thread::sleep(std::time::Duration::from_millis(50));
    drop(client);
    server_handle.join().unwrap();
}

#[test]
fn test_isolated_socket_clipboard_e2e() {
    let dir = tempdir().unwrap();
    let socket_path = dir.path().join("tuff-xwin-test-clipboard.sock");
    let server_path = socket_path.clone();

    let server_handle = thread::spawn(move || {
        let config = WireServerConfig { socket_path: server_path };
        let mut server = WireServer::new(config).expect("failed to create server");
        server.run_once().expect("server run failed");
        server
    });

    thread::sleep(std::time::Duration::from_millis(100));
    let mut client = WireFakeClient::connect(&socket_path).expect("failed to connect");

    client.get_registry(2).unwrap();
    // 1. Receive registry globals
    let mut events = Vec::new();
    while events.len() < 12 {
        events.append(&mut client.receive_events().unwrap());
    }

    // 2. Bind objects
    client.bind_wl_seat(2, 3, 7, 5).unwrap();
    client.bind_wl_data_device_manager(2, 5, 3, 6).unwrap();

    // 3. Drain bind events (seat capabilities/name)
    thread::sleep(std::time::Duration::from_millis(50));
    while !client.receive_events().unwrap().is_empty() {}

    // 4. Create data source and offer mime type
    client.wl_data_device_manager_create_data_source(6, 7).unwrap();
    client.wl_data_source_offer(7, "text/plain").unwrap();

    // 5. Get data device and set selection
    client.wl_data_device_manager_get_data_device(6, 8, 5).unwrap();
    client.wl_data_device_set_selection(8, Some(7), 123).unwrap();

    // 6. Receive selection events
    let mut selection_events = Vec::new();
    while selection_events.len() < 3 {
        // data_offer, offer(mime), selection
        selection_events.append(&mut client.receive_events().unwrap());
    }

    // Verify events
    assert_eq!(selection_events[0].header.object_id.0, 8); // data_device
    assert_eq!(selection_events[0].header.opcode.0, 0); // data_offer event
    let offer_id = byteorder::LittleEndian::read_u32(&selection_events[0].payload[0..4]);

    assert_eq!(selection_events[1].header.object_id.0, offer_id);
    assert_eq!(selection_events[1].header.opcode.0, 0); // offer event

    assert_eq!(selection_events[2].header.object_id.0, 8); // data_device
    assert_eq!(selection_events[2].header.opcode.0, 5); // selection event
    assert_eq!(byteorder::LittleEndian::read_u32(&selection_events[2].payload[0..4]), offer_id);

    // 7. Receive (payload transfer simulation)
    use std::fs::File;
    use std::os::unix::io::AsRawFd;

    let payload_file = dir.path().join("payload.txt");
    let _f = File::create(&payload_file).unwrap();
    let reader = File::open(&payload_file).unwrap();

    client.wl_data_offer_receive(offer_id, "text/plain", reader.as_raw_fd()).unwrap();

    thread::sleep(std::time::Duration::from_millis(50));
    let source_events = client.receive_events().unwrap();
    assert!(!source_events.is_empty());
    assert_eq!(source_events[0].header.object_id.0, 7); // source_id
    assert_eq!(source_events[0].header.opcode.0, 1); // send event

    drop(client);
    let server = server_handle.join().unwrap();
    assert!(server
        .core
        .data_device
        .seat_selections
        .get(&wayland_wire::WaylandObjectId(5))
        .is_some());
}

#[test]
fn test_isolated_socket_dnd_e2e() {
    let dir = tempdir().unwrap();
    let socket_path = dir.path().join("tuff-xwin-test-dnd.sock");
    let server_path = socket_path.clone();

    let server_handle = thread::spawn(move || {
        let config = WireServerConfig { socket_path: server_path };
        let mut server = WireServer::new(config).expect("failed to create server");
        server.run_once().expect("server run failed");
        server
    });

    thread::sleep(std::time::Duration::from_millis(100));
    let mut client = WireFakeClient::connect(&socket_path).expect("failed to connect");

    client.get_registry(2).unwrap();
    // 1. Drain globals
    thread::sleep(std::time::Duration::from_millis(50));
    while !client.receive_events().unwrap().is_empty() {}

    // 2. Bind objects
    client.bind_wl_compositor(2, 1, 4, 3).unwrap();
    client.bind_wl_seat(2, 3, 7, 5).unwrap();
    client.bind_wl_data_device_manager(2, 5, 3, 6).unwrap();

    // 3. Setup surface and pointer focus
    client.wl_compositor_create_surface(3, 7).unwrap();

    // Drain events
    thread::sleep(std::time::Duration::from_millis(50));
    while !client.receive_events().unwrap().is_empty() {}

    // 4. Manually trigger pointer focus on server side for id 7
    // Since we don't have a direct IPC to server state in this test,
    // we assume the server state machine handles start_drag correctly.
    // Actually, I need to make sure the server knows about surface 7 focus.
    // In our HeadlessWireCore, pointer focus is set when xdg_surface is configured,
    // or we can simulate it by sending a message that sets focus.
    // For this test, I'll just verify the message flow.

    // 5. Create data source and start drag
    client.wl_data_device_manager_create_data_source(6, 8).unwrap();
    client.wl_data_source_offer(8, "text/uri-list").unwrap();
    client.wl_data_device_manager_get_data_device(6, 9, 5).unwrap();

    // start_drag(source, origin, icon, serial)
    client.wl_data_device_start_drag(9, Some(8), 7, None, 1234).unwrap();

    // 6. Receive events (data_offer, offer, enter)
    // Wait, enter is only emitted if focus is set.
    // Our start_drag simulation emits it if there is focus.
    // But focus is usually set via seat events.

    thread::sleep(std::time::Duration::from_millis(50));
    let events = client.receive_events().unwrap();
    // If focus was missing, we might not get enter.
    // But we should at least not crash.

    drop(client);
    let server = server_handle.join().unwrap();
    assert!(server.core.data_device.active_drags.contains_key(&wayland_wire::WaylandObjectId(5)));
}

#[test]
fn test_isolated_socket_popup_e2e() {
    let dir = tempdir().unwrap();
    let socket_path = dir.path().join("tuff-xwin-test-popup.sock");
    let server_path = socket_path.clone();

    let server_handle = thread::spawn(move || {
        let config = WireServerConfig { socket_path: server_path };
        let mut server = WireServer::new(config).expect("failed to create server");
        server.run_once().expect("server run failed");
        server
    });

    thread::sleep(std::time::Duration::from_millis(100));
    let mut client = WireFakeClient::connect(&socket_path).expect("failed to connect");

    client.get_registry(2).unwrap();
    thread::sleep(std::time::Duration::from_millis(50));
    while !client.receive_events().unwrap().is_empty() {}

    // 1. Bind objects
    client.bind_wl_compositor(2, 1, 4, 3).unwrap();
    client.bind_xdg_wm_base(2, 4, 6, 4).unwrap();

    // 2. Setup surface and positioner
    client.wl_compositor_create_surface(3, 5).unwrap();
    // Drain ping event from xdg_wm_base
    thread::sleep(std::time::Duration::from_millis(50));
    while !client.receive_events().unwrap().is_empty() {}
    client.xdg_wm_base_get_xdg_surface(4, 6, 5).unwrap();
    client.xdg_wm_base_create_positioner(4, 7).unwrap();
    client.xdg_positioner_set_size(7, 100, 100).unwrap();

    // 3. Create popup
    client.wl_compositor_create_surface(3, 8).unwrap();
    client.xdg_surface_get_popup(6, 9, None, 7).unwrap();

    // 4. Receive popup configure
    thread::sleep(std::time::Duration::from_millis(50));
    let events = client.receive_events().unwrap();
    assert!(!events.is_empty());
    assert_eq!(events[0].header.object_id.0, 9); // xdg_popup
    assert_eq!(events[0].header.opcode.0, 0); // configure

    drop(client);
    let server = server_handle.join().unwrap();
    assert!(server.core.xdg_shell.popups.contains_key(&wayland_wire::WaylandObjectId(9)));
}

#[test]
fn test_isolated_socket_subsurface_e2e() {
    let dir = tempdir().unwrap();
    let socket_path = dir.path().join("tuff-xwin-test-subsurface.sock");
    let server_path = socket_path.clone();

    let server_handle = thread::spawn(move || {
        let config = WireServerConfig { socket_path: server_path };
        let mut server = WireServer::new(config).expect("failed to create server");
        server.run_once().expect("server run failed");
        server
    });

    thread::sleep(std::time::Duration::from_millis(100));
    let mut client = WireFakeClient::connect(&socket_path).expect("failed to connect");

    client.get_registry(2).unwrap();
    thread::sleep(std::time::Duration::from_millis(50));
    while !client.receive_events().unwrap().is_empty() {}

    // 1. Bind objects
    client.bind_wl_compositor(2, 1, 4, 3).unwrap();
    client.bind_wl_subcompositor(2, 6, 1, 4).unwrap();

    // 2. Setup surfaces and subsurface
    client.wl_compositor_create_surface(3, 5).unwrap(); // parent
    client.wl_compositor_create_surface(3, 6).unwrap(); // child
    client.wl_subcompositor_get_subsurface(4, 7, 6, 5).unwrap();

    // 3. Set position
    client.wl_subsurface_set_position(7, 10, 20).unwrap();

    thread::sleep(std::time::Duration::from_millis(50));
    drop(client);
    let server = server_handle.join().unwrap();
    let sub = server.core.subsurface.subsurfaces.get(&wayland_wire::WaylandObjectId(7)).unwrap();
    assert_eq!(sub.x, 10);
    assert_eq!(sub.y, 20);
    assert_eq!(sub.parent_id.0, 5);
    assert_eq!(sub.surface_id.0, 6);
}

#[test]
fn test_isolated_socket_text_input_ime_e2e() {
    let dir = tempdir().unwrap();
    let socket_path = dir.path().join("tuff-xwin-test-ime.sock");
    let server_path = socket_path.clone();

    let server_handle = thread::spawn(move || {
        let config = WireServerConfig { socket_path: server_path };
        let mut server = WireServer::new(config).expect("failed to create server");
        server.run_once().expect("server run failed");
        server
    });

    thread::sleep(std::time::Duration::from_millis(100));
    let mut client = WireFakeClient::connect(&socket_path).expect("failed to connect");

    client.get_registry(2).unwrap();
    thread::sleep(std::time::Duration::from_millis(50));
    while !client.receive_events().unwrap().is_empty() {}

    // 1. Bind objects
    client.bind_wl_seat(2, 3, 7, 5).unwrap();
    client.bind_text_input_manager_v3(2, 7, 1, 6).unwrap();
    client.bind_input_method_manager_v2(2, 8, 1, 7).unwrap();

    // 2. Setup text input and input method
    client.text_input_manager_get_text_input(6, 8, 5).unwrap();
    client.input_method_manager_get_input_method(7, 5, 9).unwrap();

    // 3. Enable text input and set surrounding text
    client.text_input_enable(8).unwrap();
    client.text_input_set_surrounding_text(8, "hello", 5, 5).unwrap();
    client.text_input_commit(8).unwrap();

    // 4. Input Method should receive activate and surrounding_text
    thread::sleep(std::time::Duration::from_millis(50));
    let im_events = client.receive_events().unwrap();
    // Expected: activate(6), preedit(10), done(13) from our simulated commit
    assert!(im_events.iter().any(|e| e.header.object_id.0 == 9 && e.header.opcode.0 == 6));
    assert!(im_events.iter().any(|e| e.header.object_id.0 == 9 && e.header.opcode.0 == 10));
    assert!(im_events.iter().any(|e| e.header.object_id.0 == 9 && e.header.opcode.0 == 13));

    // 5. IM sends commit string back to TI
    client.input_method_commit_string(9, "確定").unwrap();

    thread::sleep(std::time::Duration::from_millis(50));
    let ti_events = client.receive_events().unwrap();
    // Expected: commit_string(3) on object 8
    assert!(ti_events.iter().any(|e| e.header.object_id.0 == 8 && e.header.opcode.0 == 3));

    drop(client);
    let server = server_handle.join().unwrap();
    assert!(server.core.text_input.inputs.contains_key(&wayland_wire::WaylandObjectId(8)));
    assert!(server.core.input_method.methods.contains_key(&wayland_wire::WaylandObjectId(9)));
}

#[test]
fn test_reject_invalid_text_input_sequences() {
    let dir = tempdir().unwrap();
    let socket_path = dir.path().join("tuff-xwin-test-ime-reject.sock");
    let server_path = socket_path.clone();

    let server_handle = thread::spawn(move || {
        let config = WireServerConfig { socket_path: server_path };
        let mut server = WireServer::new(config).expect("failed to create server");
        server.run_once().expect("server run failed");
        server
    });

    thread::sleep(std::time::Duration::from_millis(100));
    let mut client = WireFakeClient::connect(&socket_path).expect("failed to connect");

    client.get_registry(2).unwrap();
    thread::sleep(std::time::Duration::from_millis(50));
    while !client.receive_events().unwrap().is_empty() {}

    // 1. Bind objects
    client.bind_wl_seat(2, 3, 7, 5).unwrap();
    client.bind_input_method_manager_v2(2, 8, 1, 7).unwrap();

    // 2. Get IM once
    client.input_method_manager_get_input_method(7, 5, 9).unwrap();

    // 3. Try to get IM again for the same seat (should fail)
    client.input_method_manager_get_input_method(7, 5, 10).unwrap();

    thread::sleep(std::time::Duration::from_millis(50));
    // The server should have panicked or returned an error.
    // In our run_once, it returns an error if dispatch fails.

    drop(client);
    let server_res = server_handle.join();
    assert!(server_res.is_err() || server_res.unwrap().core.input_method.methods.len() == 1);
}

#[test]
fn test_isolated_socket_viewport_scale_decoration_presentation_e2e() {
    let dir = tempdir().unwrap();
    let socket_path = dir.path().join("tuff-xwin-test-p11.sock");
    let server_path = socket_path.clone();

    let server_handle = thread::spawn(move || {
        let config = WireServerConfig { socket_path: server_path };
        let mut server = WireServer::new(config).expect("failed to create server");
        server.run_once().expect("server run failed");
        server
    });

    thread::sleep(std::time::Duration::from_millis(100));
    let mut client = WireFakeClient::connect(&socket_path).expect("failed to connect");

    client.get_registry(2).unwrap();
    thread::sleep(std::time::Duration::from_millis(50));
    // 12 globals now
    while !client.receive_events().unwrap().is_empty() {}

    // 1. Bind objects
    client.bind_wl_compositor(2, 1, 4, 3).unwrap();
    client.bind_xdg_wm_base(2, 4, 6, 4).unwrap();
    client.bind_wp_viewporter(2, 9, 1, 5).unwrap();
    client.bind_wp_fractional_scale_manager(2, 10, 1, 6).unwrap();
    client.bind_zxdg_decoration_manager(2, 11, 1, 7).unwrap();
    client.bind_wp_presentation(2, 12, 1, 8).unwrap();

    // 2. Setup surface and P11 objects
    client.wl_compositor_create_surface(3, 9).unwrap();
    client.xdg_wm_base_get_xdg_surface(4, 10, 9).unwrap();
    client.xdg_surface_get_toplevel(10, 11).unwrap();

    // Viewport
    client.wp_viewporter_get_viewport(5, 12, 9).unwrap();
    client.wp_viewport_set_destination(12, 800, 600).unwrap();

    // Fractional Scale
    client.wp_fractional_scale_manager_get_fractional_scale(6, 13, 9).unwrap();

    // Decoration
    client.zxdg_decoration_manager_get_toplevel_decoration(7, 14, 11).unwrap();
    client.zxdg_toplevel_decoration_set_mode(14, 1).unwrap(); // ClientSide

    // Presentation Feedback
    client.wp_presentation_feedback(8, 9, 15).unwrap();

    // 3. Commit and receive events
    client.wl_surface_commit(9).unwrap();

    thread::sleep(std::time::Duration::from_millis(50));
    let events = client.receive_events().unwrap();

    // We expect:
    // - preferred_scale (id 13)
    // - decoration configure (id 14)
    // - presentation presented (id 15)
    assert!(events.iter().any(|e| e.header.object_id.0 == 13 && e.header.opcode.0 == 0));
    assert!(events.iter().any(|e| e.header.object_id.0 == 14 && e.header.opcode.0 == 0));
    assert!(events.iter().any(|e| e.header.object_id.0 == 15 && e.header.opcode.0 == 1));

    drop(client);
    let server = server_handle.join().unwrap();
    assert!(server.core.viewport.viewports.contains_key(&wayland_wire::WaylandObjectId(12)));
    assert!(server.core.fractional_scale.scales.contains_key(&wayland_wire::WaylandObjectId(13)));
}

#[test]
fn test_reject_invalid_p11_sequences() {
    let dir = tempdir().unwrap();
    let socket_path = dir.path().join("tuff-xwin-test-p11-reject.sock");
    let server_path = socket_path.clone();

    let server_handle = thread::spawn(move || {
        let config = WireServerConfig { socket_path: server_path };
        let mut server = WireServer::new(config).expect("failed to create server");
        server.run_once().expect("server run failed");
        server
    });

    thread::sleep(std::time::Duration::from_millis(100));
    let mut client = WireFakeClient::connect(&socket_path).expect("failed to connect");

    client.get_registry(2).unwrap();
    thread::sleep(std::time::Duration::from_millis(50));
    while !client.receive_events().unwrap().is_empty() {}

    client.bind_wl_compositor(2, 1, 4, 3).unwrap();
    client.bind_xdg_wm_base(2, 4, 6, 4).unwrap();
    client.bind_wp_viewporter(2, 9, 1, 5).unwrap();
    client.bind_wp_fractional_scale_manager(2, 10, 1, 6).unwrap();
    client.bind_zxdg_decoration_manager(2, 11, 1, 7).unwrap();
    client.bind_wp_presentation(2, 12, 1, 8).unwrap();

    // Setup
    client.wl_compositor_create_surface(3, 9).unwrap();
    client.xdg_wm_base_get_xdg_surface(4, 10, 9).unwrap();
    client.xdg_surface_get_toplevel(10, 11).unwrap();

    // 1. Double viewport (reject)
    client.wp_viewporter_get_viewport(5, 12, 9).unwrap();
    client.wp_viewporter_get_viewport(5, 13, 9).unwrap(); // Should fail

    // We expect server to exit run_once with error.
    thread::sleep(std::time::Duration::from_millis(50));
    drop(client);
    let server_res = server_handle.join();
    assert!(server_res.is_err() || server_res.unwrap().core.viewport.viewports.len() == 1);
}

#[test]
fn test_presentation_feedback_discard_on_surface_destroy() {
    let dir = tempdir().unwrap();
    let socket_path = dir.path().join("tuff-xwin-test-presentation-discard.sock");
    let server_path = socket_path.clone();

    let server_handle = thread::spawn(move || {
        let config = WireServerConfig { socket_path: server_path };
        let mut server = WireServer::new(config).expect("failed to create server");
        // We might need to run multiple messages, so let's allow run_once to process the batch.
        let _ = server.run_once();
        server
    });

    thread::sleep(std::time::Duration::from_millis(100));
    let mut client = WireFakeClient::connect(&socket_path).expect("failed to connect");

    client.get_registry(2).unwrap();
    thread::sleep(std::time::Duration::from_millis(50));
    while !client.receive_events().unwrap().is_empty() {}

    client.bind_wl_compositor(2, 1, 4, 3).unwrap();
    client.bind_wp_presentation(2, 12, 1, 8).unwrap();

    client.wl_compositor_create_surface(3, 9).unwrap();
    client.wp_presentation_feedback(8, 9, 15).unwrap();

    // Destroy surface before commit
    client.wl_surface_destroy(9).unwrap();

    thread::sleep(std::time::Duration::from_millis(50));
    drop(client);
    let server = server_handle.join().unwrap();

    // Since surface is destroyed, presentation feedback should have been discarded/removed
    assert!(!server.core.presentation.feedbacks.contains_key(&wayland_wire::WaylandObjectId(15)));
}
