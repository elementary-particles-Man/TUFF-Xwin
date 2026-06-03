use wayland_wire::generated::p13_protocol_spec;

fn expect_request(
    spec: &wayland_wire::protocol::ProtocolSpec,
    iface: &str,
    request: &str,
    opcode: u16,
) {
    let iface_spec =
        spec.interfaces.get(iface).unwrap_or_else(|| panic!("missing interface {iface}"));
    let msg = iface_spec
        .requests
        .iter()
        .find(|msg| msg.name == request)
        .unwrap_or_else(|| panic!("missing request {iface}.{request}"));
    assert_eq!(msg.opcode, opcode, "unexpected opcode for {iface}.{request}");
}

fn expect_event(
    spec: &wayland_wire::protocol::ProtocolSpec,
    iface: &str,
    event: &str,
    opcode: u16,
) {
    let iface_spec =
        spec.interfaces.get(iface).unwrap_or_else(|| panic!("missing interface {iface}"));
    let msg = iface_spec
        .events
        .iter()
        .find(|msg| msg.name == event)
        .unwrap_or_else(|| panic!("missing event {iface}.{event}"));
    assert_eq!(msg.opcode, opcode, "unexpected opcode for {iface}.{event}");
}

#[test]
fn test_protocol_matrix_snapshot() {
    let spec = p13_protocol_spec();

    // Core / SHM / surface baseline
    expect_request(spec, "wl_display", "get_registry", 1);
    expect_request(spec, "wl_display", "sync", 0);
    expect_request(spec, "wl_compositor", "create_surface", 0);
    expect_request(spec, "wl_compositor", "create_region", 1);
    expect_request(spec, "wl_surface", "attach", 1);
    expect_request(spec, "wl_surface", "damage", 2);
    expect_request(spec, "wl_surface", "frame", 3);
    expect_request(spec, "wl_surface", "commit", 6);
    expect_request(spec, "wl_shm", "create_pool", 0);
    expect_request(spec, "wl_seat", "get_pointer", 0);
    expect_request(spec, "wl_seat", "get_keyboard", 1);

    // XDG shell / popup / subsurface
    expect_request(spec, "xdg_wm_base", "create_positioner", 1);
    expect_request(spec, "xdg_wm_base", "get_xdg_surface", 2);
    expect_request(spec, "xdg_wm_base", "pong", 3);
    expect_request(spec, "xdg_surface", "get_toplevel", 1);
    expect_request(spec, "xdg_surface", "get_popup", 2);
    expect_request(spec, "xdg_surface", "ack_configure", 4);
    expect_request(spec, "xdg_toplevel", "set_title", 2);
    expect_request(spec, "xdg_toplevel", "set_app_id", 3);
    expect_request(spec, "wl_subcompositor", "get_subsurface", 1);
    expect_request(spec, "wl_subsurface", "set_position", 1);
    expect_request(spec, "wl_subsurface", "set_sync", 4);
    expect_request(spec, "wl_subsurface", "set_desync", 5);

    // Clipboard / DnD / text input / IME
    expect_request(spec, "wl_data_device_manager", "create_data_source", 0);
    expect_request(spec, "wl_data_device_manager", "get_data_device", 1);
    expect_request(spec, "wl_data_source", "offer", 0);
    expect_request(spec, "wl_data_source", "destroy", 1);
    expect_request(spec, "zwp_text_input_manager_v3", "get_text_input", 1);
    expect_request(spec, "zwp_text_input_v3", "enable", 1);
    expect_request(spec, "zwp_text_input_v3", "commit", 7);
    expect_request(spec, "zwp_input_method_manager_v2", "get_input_method", 1);
    expect_request(spec, "zwp_input_method_v2", "commit_string", 1);
    expect_request(spec, "zwp_input_method_v2", "get_input_popup_surface", 5);

    // Output / presentation / viewport / decoration
    expect_request(spec, "zxdg_output_manager_v1", "get_xdg_output", 1);
    expect_request(spec, "zwlr_output_manager_v1", "create_configuration", 0);
    expect_request(spec, "zwlr_screencopy_manager_v1", "capture_output", 0);
    expect_request(spec, "ext_image_copy_capture_manager_v1", "create_session", 1);
    expect_request(spec, "wp_viewporter", "get_viewport", 1);
    expect_request(spec, "wp_viewport", "set_destination", 2);
    expect_request(spec, "wp_presentation", "feedback", 1);
    assert!(spec.interfaces.contains_key("wp_presentation_feedback"));
    expect_request(spec, "wp_fractional_scale_manager_v1", "get_fractional_scale", 1);
    expect_request(spec, "zxdg_decoration_manager_v1", "get_toplevel_decoration", 1);

    // P13 layer/input control
    expect_request(spec, "zwlr_layer_shell_v1", "get_layer_surface", 0);
    expect_request(spec, "zwlr_layer_surface_v1", "set_size", 0);
    expect_request(spec, "zwlr_layer_surface_v1", "ack_configure", 6);
    expect_event(spec, "zwlr_layer_surface_v1", "configure", 0);
    expect_event(spec, "zwlr_layer_surface_v1", "closed", 1);
    expect_request(spec, "zwp_idle_inhibit_manager_v1", "create_inhibitor", 0);
    expect_request(spec, "zwp_relative_pointer_manager_v1", "get_relative_pointer", 0);
    expect_event(spec, "zwp_relative_pointer_v1", "relative_motion", 0);
    expect_request(spec, "zwp_pointer_constraints_v1", "lock_pointer", 0);
    expect_request(spec, "zwp_pointer_constraints_v1", "confine_pointer", 1);
    expect_event(spec, "zwp_locked_pointer_v1", "locked", 0);
    expect_event(spec, "zwp_confined_pointer_v1", "confined", 0);
}
