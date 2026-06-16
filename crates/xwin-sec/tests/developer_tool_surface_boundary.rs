use xwin_sec::*;

fn base_context() -> XwinDeveloperToolSurfaceContext {
    XwinDeveloperToolSurfaceContext {
        surface_markers: vec!["xwin_developer_tool_surface_present".to_string()],
        pal_markers: vec![],
        kairo_verdict: None,
    }
}

#[test]
fn no_developer_surface_allows() {
    let mut context = base_context();
    context.surface_markers = vec![];
    let action = evaluate_xwin_developer_tool_surface_boundary(&context);
    assert_eq!(action, XwinDeveloperToolSurfaceAction::Allow);
}

#[test]
fn trusted_developer_surface_allows() {
    let context = base_context();
    let action = evaluate_xwin_developer_tool_surface_boundary(&context);
    assert_eq!(action, XwinDeveloperToolSurfaceAction::Observe);
}

#[test]
fn untrusted_context_without_surface_observes() {
    let mut context = base_context();
    context.surface_markers.push("xwin_untrusted_developer_source_context".to_string());
    let action = evaluate_xwin_developer_tool_surface_boundary(&context);
    assert_eq!(action, XwinDeveloperToolSurfaceAction::Observe);
}

#[test]
fn clipboard_untrusted_quarantines() {
    let mut context = base_context();
    context.surface_markers.push("xwin_untrusted_developer_source_context".to_string());
    context.surface_markers.push("xwin_clipboard_surface_requested".to_string());
    let action = evaluate_xwin_developer_tool_surface_boundary(&context);
    assert_eq!(action, XwinDeveloperToolSurfaceAction::Quarantine);
}

#[test]
fn file_picker_untrusted_quarantines() {
    let mut context = base_context();
    context.surface_markers.push("xwin_interview_task_context".to_string());
    context.surface_markers.push("xwin_file_picker_surface_requested".to_string());
    let action = evaluate_xwin_developer_tool_surface_boundary(&context);
    assert_eq!(action, XwinDeveloperToolSurfaceAction::Quarantine);
}

#[test]
fn drag_drop_untrusted_quarantines() {
    let mut context = base_context();
    context.pal_markers.push("pal_repo_origin_untrusted".to_string());
    context.surface_markers.push("xwin_drag_drop_surface_requested".to_string());
    let action = evaluate_xwin_developer_tool_surface_boundary(&context);
    assert_eq!(action, XwinDeveloperToolSurfaceAction::Quarantine);
}

#[test]
fn terminal_launch_untrusted_quarantines() {
    let mut context = base_context();
    context.surface_markers.push("xwin_untrusted_developer_source_context".to_string());
    context.surface_markers.push("xwin_terminal_launch_surface_requested".to_string());
    let action = evaluate_xwin_developer_tool_surface_boundary(&context);
    assert_eq!(action, XwinDeveloperToolSurfaceAction::Quarantine);
}

#[test]
fn download_upload_untrusted_quarantines() {
    let mut context = base_context();
    context.surface_markers.push("xwin_untrusted_developer_source_context".to_string());
    context.surface_markers.push("xwin_browser_download_upload_surface_requested".to_string());
    let action = evaluate_xwin_developer_tool_surface_boundary(&context);
    assert_eq!(action, XwinDeveloperToolSurfaceAction::Quarantine);
}

#[test]
fn ide_extension_install_untrusted_quarantines() {
    let mut context = base_context();
    context.surface_markers.push("xwin_untrusted_developer_source_context".to_string());
    context.surface_markers.push("xwin_ide_extension_install_surface_requested".to_string());
    let action = evaluate_xwin_developer_tool_surface_boundary(&context);
    assert_eq!(action, XwinDeveloperToolSurfaceAction::Quarantine);
}

#[test]
fn browser_extension_install_untrusted_quarantines() {
    let mut context = base_context();
    context.surface_markers.push("xwin_untrusted_developer_source_context".to_string());
    context.surface_markers.push("xwin_browser_extension_install_surface_requested".to_string());
    let action = evaluate_xwin_developer_tool_surface_boundary(&context);
    assert_eq!(action, XwinDeveloperToolSurfaceAction::Quarantine);
}

#[test]
fn screen_capture_untrusted_quarantines() {
    let mut context = base_context();
    context.surface_markers.push("xwin_untrusted_developer_source_context".to_string());
    context.surface_markers.push("xwin_screen_capture_surface_requested".to_string());
    let action = evaluate_xwin_developer_tool_surface_boundary(&context);
    assert_eq!(action, XwinDeveloperToolSurfaceAction::Quarantine);
}

#[test]
fn external_protocol_untrusted_fail_closed() {
    let mut context = base_context();
    context.surface_markers.push("xwin_untrusted_developer_source_context".to_string());
    context.surface_markers.push("xwin_external_protocol_surface_requested".to_string());
    let action = evaluate_xwin_developer_tool_surface_boundary(&context);
    assert_eq!(action, XwinDeveloperToolSurfaceAction::FailClosed);
}

#[test]
fn secret_surface_overlap_fail_closed() {
    let mut context = base_context();
    context.surface_markers.push("xwin_untrusted_developer_source_context".to_string());
    context.pal_markers.push("pal_ssh_key_access".to_string());
    context.surface_markers.push("xwin_terminal_launch_surface_requested".to_string());
    let action = evaluate_xwin_developer_tool_surface_boundary(&context);
    assert_eq!(action, XwinDeveloperToolSurfaceAction::FailClosed);
}

#[test]
fn persistence_surface_overlap_fail_closed() {
    let mut context = base_context();
    context.surface_markers.push("xwin_untrusted_developer_source_context".to_string());
    context.pal_markers.push("pal_shell_profile_write".to_string());
    context.surface_markers.push("xwin_terminal_launch_surface_requested".to_string());
    let action = evaluate_xwin_developer_tool_surface_boundary(&context);
    assert_eq!(action, XwinDeveloperToolSurfaceAction::FailClosed);
}

#[test]
fn kairo_quarantine_propagates() {
    let mut context = base_context();
    context.kairo_verdict = Some("quarantine".to_string());
    let action = evaluate_xwin_developer_tool_surface_boundary(&context);
    assert_eq!(action, XwinDeveloperToolSurfaceAction::Quarantine);
}

#[test]
fn kairo_fail_closed_propagates() {
    let mut context = base_context();
    context.kairo_verdict = Some("fail_closed".to_string());
    let action = evaluate_xwin_developer_tool_surface_boundary(&context);
    assert_eq!(action, XwinDeveloperToolSurfaceAction::FailClosed);
}
