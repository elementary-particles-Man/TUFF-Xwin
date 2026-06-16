use xwin_sec::*;

fn base_context() -> XwinPrivilegedAiSurfaceContext {
    XwinPrivilegedAiSurfaceContext {
        surface_markers: vec!["xwin_privileged_ai_surface_present".to_string()],
        pal_markers: vec!["pal_privileged_ai_invocation".to_string()],
        kairo_verdict: None,
    }
}

#[test]
fn no_privileged_ai_surface_allows() {
    let mut context = base_context();
    context.surface_markers = vec![];
    let action = evaluate_xwin_privileged_ai_surface_boundary(&context);
    assert_eq!(action, XwinAction::Allow);
}

#[test]
fn display_only_surface_allows() {
    let context = base_context();
    let action = evaluate_xwin_privileged_ai_surface_boundary(&context);
    assert_eq!(action, XwinAction::Observe);
}

#[test]
fn clipboard_sensitive_context_quarantines() {
    let mut context = base_context();
    context.pal_markers.push("pal_sensitive_label_present".to_string());
    context.surface_markers.push("xwin_clipboard_surface_requested".to_string());
    let action = evaluate_xwin_privileged_ai_surface_boundary(&context);
    assert_eq!(action, XwinAction::Quarantine);
}

#[test]
fn file_picker_sensitive_context_quarantines() {
    let mut context = base_context();
    context.pal_markers.push("pal_file_context".to_string());
    context.surface_markers.push("xwin_file_picker_surface_requested".to_string());
    let action = evaluate_xwin_privileged_ai_surface_boundary(&context);
    assert_eq!(action, XwinAction::Quarantine);
}

#[test]
fn native_messaging_sensitive_context_fail_closed() {
    let mut context = base_context();
    context.pal_markers.push("pal_sensitive_label_present".to_string());
    context.surface_markers.push("xwin_native_messaging_surface_requested".to_string());
    let action = evaluate_xwin_privileged_ai_surface_boundary(&context);
    assert_eq!(action, XwinAction::FailClosed);
}

#[test]
fn browser_profile_surface_fail_closed() {
    let mut context = base_context();
    context.surface_markers.push("xwin_browser_profile_surface_requested".to_string());
    context.pal_markers.push("pal_mail_context".to_string());
    let action = evaluate_xwin_privileged_ai_surface_boundary(&context);
    assert_eq!(action, XwinAction::FailClosed);
}

#[test]
fn kairo_fail_closed_propagates_fail_closed() {
    let mut context = base_context();
    context.kairo_verdict = Some("fail_closed".to_string());
    let action = evaluate_xwin_privileged_ai_surface_boundary(&context);
    assert_eq!(action, XwinAction::FailClosed);
}

#[test]
fn unknown_risky_surface_fail_closed() {
    let mut context = base_context();
    context.surface_markers.push("xwin_unknown_risky_surface_requested".to_string());
    let action = evaluate_xwin_privileged_ai_surface_boundary(&context);
    assert_eq!(action, XwinAction::FailClosed);
}
