// TUFF-Xwin waylandd completion/regression tests.
// This file is included from new_main.rs under #[cfg(test)].

fn test_surface_snapshot(id: &str, generation: u64) -> SurfaceSnapshot {
    SurfaceSnapshot {
        id: id.to_string(),
        app_id: "completion-test".into(),
        placement: SurfacePlacement {
            x: 0,
            y: 0,
            width: 64,
            height: 64,
            z: 0,
            visible: true,
        },
        buffer_handle: Some(format!("buffer-{generation}")),
        buffer_generation: generation,
        damage_rects: vec![waybroker_common::Rect {
            x: 0,
            y: 0,
            width: 64,
            height: 64,
        }],
        pixel_transport: Some(PixelTransportHandle {
            client_id: 1,
            surface_id: id.to_string(),
            buffer_generation: generation,
            scene_generation: generation,
        }),
        layer_class: 0,
        creation_sequence: generation,
    }
}

fn test_pixel_payload(id: &str, generation: u64, bytes: usize) -> PixelTransportPayload {
    PixelTransportPayload {
        handle: PixelTransportHandle {
            client_id: 1,
            surface_id: id.to_string(),
            buffer_generation: generation,
            scene_generation: generation,
        },
        pixels: vec![0x5a; bytes],
        width: 64,
        height: 64,
        stride: 256,
        format: 0,
    }
}

#[test]
fn completion_copy_bytes_accel_matches_portable_semantics() {
    for len in [0usize, 1, 31, 32, 33, 511, 512, 513, 4097, 65537] {
        let input = (0..len).map(|i| (i as u8).wrapping_mul(37)).collect::<Vec<_>>();
        let output = copy_bytes_accel(&input);
        assert_eq!(output, input, "accelerated copy changed data at len={len}");
    }
}

#[test]
fn completion_scene_budget_accepts_normal_scene() {
    let surfaces = vec![test_surface_snapshot("s1", 1), test_surface_snapshot("s2", 2)];
    let payloads = vec![
        test_pixel_payload("s1", 1, 16 * 1024),
        test_pixel_payload("s2", 2, 16 * 1024),
    ];
    let usage = validate_scene_budget(&surfaces, &payloads).unwrap();
    assert_eq!(usage.surfaces, 2);
    assert_eq!(usage.pixel_payloads, 2);
    assert_eq!(usage.pixel_bytes, 32 * 1024);
}

#[test]
fn completion_scene_budget_rejects_surface_exhaustion() {
    let surface = test_surface_snapshot("s", 1);
    let surfaces = vec![surface; MAX_SCENE_SURFACES + 1];
    let err = validate_scene_budget(&surfaces, &[]).unwrap_err();
    assert!(err.to_string().contains("surface budget exceeded"));
}

#[test]
fn completion_pending_commit_is_latest_state_only() {
    let mut scene = CanonicalSceneState { generation: 10, ..Default::default() };
    

    coalesce_pending_commit(
        &mut scene,
        PendingCanonicalCommit {
            generation: 10,
            surfaces: vec![test_surface_snapshot("old", 10)],
            pixel_payloads: vec![test_pixel_payload("old", 10, 128)],
            reason: "old".into(),
        },
    );
    coalesce_pending_commit(
        &mut scene,
        PendingCanonicalCommit {
            generation: 12,
            surfaces: vec![test_surface_snapshot("new", 12)],
            pixel_payloads: vec![test_pixel_payload("new", 12, 256)],
            reason: "new".into(),
        },
    );
    coalesce_pending_commit(
        &mut scene,
        PendingCanonicalCommit {
            generation: 11,
            surfaces: vec![test_surface_snapshot("stale", 11)],
            pixel_payloads: vec![test_pixel_payload("stale", 11, 64)],
            reason: "stale".into(),
        },
    );

    let pending = scene.pending.as_ref().unwrap();
    assert_eq!(pending.generation, 12);
    assert_eq!(pending.surfaces[0].id, "new");
    assert_eq!(pending.pixel_payloads[0].pixels.len(), 256);
}

#[test]
fn completion_pending_replay_never_replays_older_generation() {
    let mut scene = CanonicalSceneState { generation: 20, ..Default::default() };
    
    scene.pending = Some(PendingCanonicalCommit {
        generation: 19,
        surfaces: vec![test_surface_snapshot("stale", 19)],
        pixel_payloads: vec![test_pixel_payload("stale", 19, 64)],
        reason: "stale".into(),
    });
    assert!(pending_replay_commit(&scene).is_none());

    scene.pending = Some(PendingCanonicalCommit {
        generation: 20,
        surfaces: vec![test_surface_snapshot("current", 20)],
        pixel_payloads: vec![test_pixel_payload("current", 20, 64)],
        reason: "current".into(),
    });
    let replay = pending_replay_commit(&scene).unwrap();
    assert_eq!(replay.0, 20);
    assert_eq!(replay.1[0].id, "current");
}

#[test]
fn completion_frame_callbacks_survive_visual_coalescing_until_presented() {
    let mut callbacks = PendingFrameCallbacks::default();
    callbacks
        .register(
            7,
            wayland_wire::WaylandObjectId(100),
            9,
            40,
            vec!["OUT-1".into()],
        )
        .unwrap();
    callbacks
        .register(
            7,
            wayland_wire::WaylandObjectId(101),
            9,
            41,
            vec!["OUT-1".into()],
        )
        .unwrap();

    assert_eq!(callbacks.len(), 2);
    assert!(callbacks.release_for_presented(7, 39, "OUT-1").is_empty());
    assert_eq!(callbacks.len(), 2);

    let released = callbacks.release_for_presented(7, 41, "OUT-1");
    assert_eq!(released, vec![100, 101]);
    assert_eq!(callbacks.len(), 0);
}

#[test]
fn completion_frame_callbacks_are_output_scoped_and_exactly_once() {
    let mut callbacks = PendingFrameCallbacks::default();
    callbacks
        .register(
            8,
            wayland_wire::WaylandObjectId(200),
            10,
            50,
            vec!["OUT-A".into(), "OUT-B".into()],
        )
        .unwrap();

    assert!(callbacks.release_for_presented(8, 50, "OUT-X").is_empty());
    let once = callbacks.release_for_presented(8, 50, "OUT-B");
    assert_eq!(once, vec![200]);
    assert!(callbacks.release_for_presented(8, 50, "OUT-A").is_empty());
}

#[test]
fn r2_surface_membership_churn_is_stable_for_unchanged_geometry() {
    let placement = SurfacePlacement {
        x: 100,
        y: 100,
        width: 800,
        height: 600,
        z: 0,
        visible: true,
    };
    let outputs = vec![
        (
            "OUT-1".into(),
            waybroker_common::Rect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
        ),
        (
            "OUT-2".into(),
            waybroker_common::Rect {
                x: 1920,
                y: 0,
                width: 1920,
                height: 1080,
            },
        ),
    ];

    let first = intersecting_output_ids(&placement, &outputs);
    let second = intersecting_output_ids(&placement, &outputs);
    assert_eq!(first, vec!["OUT-1".to_string()]);
    assert_eq!(first, second);
}

#[test]
fn completion_scene_order_is_deterministic() {
    let mut a = test_surface_snapshot("a", 1);
    let mut b = test_surface_snapshot("b", 2);
    let mut c = test_surface_snapshot("c", 3);
    a.layer_class = 1;
    a.placement.z = 5;
    b.layer_class = 0;
    b.placement.z = 99;
    c.layer_class = 1;
    c.placement.z = 5;
    a.creation_sequence = 2;
    c.creation_sequence = 1;

    let ordered = order_scene_surfaces(vec![a, b, c]);
    let ids = ordered.iter().map(|s| s.id.as_str()).collect::<Vec<_>>();
    assert_eq!(ids, vec!["b", "c", "a"]);
    for (index, surface) in ordered.iter().enumerate() {
        assert_eq!(surface.placement.z, index as i32);
    }
}

#[test]
fn completion_vulkan_backend_cpu_fallback_preserves_refine_semantics() {
    let backend = VulkanBackend::new(VulkanBackendConfig {
        enable_vulkan: false,
        ..Default::default()
    });
    let mut pixels = vec![0xFF332211u32; 257];
    backend.refine_screenshot_pixels(&mut pixels);
    assert!(pixels.iter().all(|pixel| *pixel == 0xFF112233u32));
}

#[test]
fn completion_scene_budget_rejects_orphan_payload() {
    let surfaces = vec![test_surface_snapshot("s1", 1)];
    let payloads = vec![test_pixel_payload("s2", 1, 16 * 1024)];
    let err = validate_scene_budget(&surfaces, &payloads).unwrap_err();
    assert!(err.to_string().contains("orphan PixelTransport payload"));
}

#[test]
fn completion_scene_budget_rejects_duplicate_payload_handle() {
    let surfaces = vec![test_surface_snapshot("s1", 1)];
    let payload = test_pixel_payload("s1", 1, 16 * 1024);
    let err = validate_scene_budget(&surfaces, &[payload.clone(), payload]).unwrap_err();
    assert!(
        err.to_string()
            .contains("duplicate PixelTransport payload handle")
    );
}

#[test]
fn completion_scene_budget_rejects_payload_size_mismatch() {
    let surfaces = vec![test_surface_snapshot("s1", 1)];
    let payloads = vec![test_pixel_payload("s1", 1, 1024)];
    let err = validate_scene_budget(&surfaces, &payloads).unwrap_err();
    assert!(err.to_string().contains("payload size mismatch"));
}

#[test]
fn completion_scene_generation_stamp_is_atomic_across_metadata_and_payload() {
    let mut surfaces = vec![test_surface_snapshot("s1", 1), test_surface_snapshot("s2", 2)];
    let mut payloads = vec![
        test_pixel_payload("s1", 1, 16 * 1024),
        test_pixel_payload("s2", 2, 16 * 1024),
    ];

    stamp_scene_generation(&mut surfaces, &mut payloads, 77);

    assert!(surfaces.iter().all(|surface| {
        surface
            .pixel_transport
            .as_ref()
            .map(|handle| handle.scene_generation == 77)
            .unwrap_or(true)
    }));
    assert!(
        payloads
            .iter()
            .all(|payload| payload.handle.scene_generation == 77)
    );
    validate_scene_budget(&surfaces, &payloads).unwrap();
}

#[test]
fn completion_scene_budget_accepts_metadata_without_pixel_transport() {
    let mut surface = test_surface_snapshot("metadata-only", 1);
    surface.pixel_transport = None;
    let usage = validate_scene_budget(&[surface], &[]).unwrap();
    assert_eq!(usage.surfaces, 1);
    assert_eq!(usage.pixel_payloads, 0);
    assert_eq!(usage.pixel_bytes, 0);
}
