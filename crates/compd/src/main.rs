use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    io::BufReader,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use vulkan_backend::{
    VulkanBackend, VulkanBackendConfig, VulkanBatchSubmission, VulkanWorkloadClass,
};
use waybroker_common::{
    CommitTarget, CommittedSceneState, DisplayCommand, DisplayEvent, FocusTarget, IpcEnvelope,
    MessageKind, PixelTransportPayload, ServiceBanner, ServiceEndpoint, ServiceRole, ServiceStream,
    SurfacePlacement, SurfaceRegistrySnapshot, SurfaceSnapshot, WaylandCommand, WaylandEvent,
    WaylandSelectionHandoff, WaylandSelectionState, WaylandSurfaceRole, WaylandSurfaceState,
    accel::global_accel_policy, bind_service_socket, connect_service_socket,
    is_recoverable_accept_error, read_ipc_envelope, send_ipc_display_command, send_ipc_envelope,
};

const MAX_RELAY_SURFACES: usize = 4096;
const MAX_RELAY_PIXEL_PAYLOADS: usize = 4096;
const MAX_RELAY_PIXEL_BYTES: usize = 256 * 1024 * 1024;
const MAX_RELAY_SINGLE_PAYLOAD_BYTES: usize = 64 * 1024 * 1024;
const DISPLAY_RECONNECT_RETRY_LIMIT: u8 = 3;
const IPC_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RelayResourceUsage {
    surfaces: usize,
    pixel_payloads: usize,
    pixel_bytes: usize,
}

#[derive(Debug, Clone)]
struct RelayScene {
    scene: CompdScene,
    pixel_payloads: Vec<PixelTransportPayload>,
}

impl RelayScene {
    fn version(&self) -> (u64, u64) {
        (self.scene.scene_epoch, self.scene.scene_generation)
    }

    #[cfg(test)]
    fn to_commit_command(&self) -> DisplayCommand {
        DisplayCommand::CommitScene {
            target: CommitTarget::Output { name: self.scene.target_output.clone() },
            focus: self.scene.focus.clone(),
            selection: self.scene.selection.clone(),
            surfaces: self.scene.surfaces.clone(),
            pixel_payloads: self.pixel_payloads.clone(),
            scene_epoch: self.scene.scene_epoch,
            scene_generation: self.scene.scene_generation,
        }
    }

    fn take_commit_command(&mut self) -> DisplayCommand {
        DisplayCommand::CommitScene {
            target: CommitTarget::Output { name: self.scene.target_output.clone() },
            focus: self.scene.focus.clone(),
            selection: self.scene.selection.clone(),
            surfaces: self.scene.surfaces.clone(),
            pixel_payloads: std::mem::take(&mut self.pixel_payloads),
            scene_epoch: self.scene.scene_epoch,
            scene_generation: self.scene.scene_generation,
        }
    }

    fn take_reconcile_command(&mut self, display_epoch: u64) -> DisplayCommand {
        DisplayCommand::ReconcileScene {
            epoch: display_epoch,
            scene_epoch: self.scene.scene_epoch,
            scene_generation: self.scene.scene_generation,
            target: CommitTarget::Output { name: self.scene.target_output.clone() },
            focus: self.scene.focus.clone(),
            selection: self.scene.selection.clone(),
            surfaces: self.scene.surfaces.clone(),
            pixel_payloads: std::mem::take(&mut self.pixel_payloads),
        }
    }

    fn restore_payloads_from_command(&mut self, command: DisplayCommand) {
        self.pixel_payloads = match command {
            DisplayCommand::CommitScene { pixel_payloads, .. }
            | DisplayCommand::ReconcileScene { pixel_payloads, .. } => pixel_payloads,
            _ => Vec::new(),
        };
    }
}

#[derive(Debug, Clone)]
struct PendingRelayScene {
    version: (u64, u64),
    usage: RelayResourceUsage,
    reason: String,
}

#[derive(Debug, Default)]
struct CompdRelayState {
    last_forwarded_epoch: u64,
    last_forwarded_generation: u64,
    pending: Option<PendingRelayScene>,
    accepted_count: u64,
    deferred_count: u64,
    superseded_count: u64,
}

impl CompdRelayState {
    fn version_floor(&self) -> (u64, u64) {
        let forwarded = (self.last_forwarded_epoch, self.last_forwarded_generation);
        self.pending
            .as_ref()
            .map(|pending| pending.version)
            .filter(|pending| *pending > forwarded)
            .unwrap_or(forwarded)
    }

    fn should_reject_version(&self, incoming: (u64, u64)) -> bool {
        let floor = self.version_floor();
        if floor == (0, 0) {
            return false;
        }
        if incoming == (0, 0) {
            return true;
        }
        if incoming < floor {
            return true;
        }
        if incoming == floor {
            return self
                .pending
                .as_ref()
                .map(|pending| pending.version != incoming)
                .unwrap_or(true);
        }
        false
    }

    fn note_forwarded(&mut self, relay: &RelayScene) {
        let version = relay.version();
        if version != (0, 0)
            && version >= (self.last_forwarded_epoch, self.last_forwarded_generation)
        {
            self.last_forwarded_epoch = version.0;
            self.last_forwarded_generation = version.1;
        }
        if self.pending.as_ref().map(|pending| pending.version <= version).unwrap_or(false) {
            self.pending = None;
        }
        self.accepted_count = self.accepted_count.saturating_add(1);
    }

    fn defer_latest(&mut self, relay: &RelayScene, reason: String) {
        let version = relay.version();
        let replace =
            self.pending.as_ref().map(|pending| version >= pending.version).unwrap_or(true);
        if replace {
            if self.pending.is_some() {
                self.superseded_count = self.superseded_count.saturating_add(1);
            }
            self.pending =
                Some(PendingRelayScene { version, usage: relay_resource_usage(relay), reason });
        }
        self.deferred_count = self.deferred_count.saturating_add(1);
    }
}

fn relay_resource_usage(relay: &RelayScene) -> RelayResourceUsage {
    RelayResourceUsage {
        surfaces: relay.scene.surfaces.len(),
        pixel_payloads: relay.pixel_payloads.len(),
        pixel_bytes: relay
            .pixel_payloads
            .iter()
            .fold(0usize, |total, payload| total.saturating_add(payload.pixels.len())),
    }
}

fn validate_relay_scene(
    relay: &RelayScene,
    require_transport_payloads: bool,
) -> Result<RelayResourceUsage> {
    let usage = relay_resource_usage(relay);
    if relay.scene.target_output.is_empty() {
        bail!("scene target output must not be empty");
    }
    if (relay.scene.scene_epoch == 0) != (relay.scene.scene_generation == 0) {
        bail!("scene epoch and generation must both be zero or both be non-zero");
    }
    if usage.surfaces > MAX_RELAY_SURFACES {
        bail!("compd surface budget exceeded: {} > {}", usage.surfaces, MAX_RELAY_SURFACES);
    }
    if usage.pixel_payloads > MAX_RELAY_PIXEL_PAYLOADS {
        bail!(
            "compd PixelTransport payload budget exceeded: {} > {}",
            usage.pixel_payloads,
            MAX_RELAY_PIXEL_PAYLOADS
        );
    }
    if usage.pixel_bytes > MAX_RELAY_PIXEL_BYTES {
        bail!(
            "compd PixelTransport byte budget exceeded: {} > {}",
            usage.pixel_bytes,
            MAX_RELAY_PIXEL_BYTES
        );
    }

    let mut surface_ids = BTreeSet::new();
    let mut expected_handles = BTreeSet::new();
    for surface in &relay.scene.surfaces {
        if surface.id.is_empty() || !surface_ids.insert(surface.id.as_str()) {
            bail!("scene contains empty or duplicate surface identity");
        }
        if surface.placement.visible
            && (surface.placement.width == 0 || surface.placement.height == 0)
        {
            bail!("visible surface {} has zero dimensions", surface.id);
        }
        let right =
            i64::from(surface.placement.x).saturating_add(i64::from(surface.placement.width));
        let bottom =
            i64::from(surface.placement.y).saturating_add(i64::from(surface.placement.height));
        if right > i64::from(i32::MAX) || bottom > i64::from(i32::MAX) {
            bail!("surface {} placement overflows compositor coordinates", surface.id);
        }
        if let Some(handle) = surface.pixel_transport.as_ref() {
            if handle.surface_id != surface.id {
                bail!("PixelTransport surface identity mismatch for {}", surface.id);
            }
            if relay.scene.scene_generation != 0
                && handle.scene_generation != relay.scene.scene_generation
            {
                bail!("PixelTransport scene generation mismatch for {}", surface.id);
            }
            if surface.buffer_generation != 0
                && handle.buffer_generation != surface.buffer_generation
            {
                bail!("PixelTransport buffer generation mismatch for {}", surface.id);
            }
            expected_handles.insert(handle.clone());
        }
    }

    if let FocusTarget::Surface { id } = &relay.scene.focus {
        if !surface_ids.contains(id.as_str()) {
            bail!("focused surface {id} is not present in the scene");
        }
    }

    for owner in [
        relay.scene.selection.clipboard_owner.as_deref(),
        relay.scene.selection.primary_selection_owner.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if !surface_ids.contains(owner) {
            bail!("selection owner {owner} is not present in the scene");
        }
    }

    let mut payload_handles = BTreeSet::new();
    for payload in &relay.pixel_payloads {
        let handle = &payload.handle;
        if !payload_handles.insert(handle.clone()) {
            bail!("duplicate PixelTransport payload handle for {}", handle.surface_id);
        }
        if !surface_ids.contains(handle.surface_id.as_str()) {
            bail!("orphan PixelTransport payload for {}", handle.surface_id);
        }
        if !expected_handles.contains(handle) {
            bail!("PixelTransport payload does not match canonical surface handle");
        }
        if relay.scene.scene_generation != 0
            && handle.scene_generation != relay.scene.scene_generation
        {
            bail!("PixelTransport payload generation does not match scene generation");
        }
        if payload.width == 0 || payload.height == 0 {
            bail!("PixelTransport payload dimensions must be non-zero");
        }
        let minimum_stride = payload
            .width
            .checked_mul(4)
            .ok_or_else(|| anyhow::anyhow!("PixelTransport stride overflow"))?;
        if payload.stride < minimum_stride {
            bail!("PixelTransport payload stride is smaller than 32-bit pixel width");
        }
        let required = (payload.stride as usize)
            .checked_mul(payload.height as usize)
            .ok_or_else(|| anyhow::anyhow!("PixelTransport byte length overflow"))?;
        if required != payload.pixels.len() {
            bail!(
                "PixelTransport payload byte length mismatch for {}: {} != {}",
                handle.surface_id,
                payload.pixels.len(),
                required
            );
        }
        if required > MAX_RELAY_SINGLE_PAYLOAD_BYTES {
            bail!(
                "single PixelTransport payload budget exceeded: {} > {}",
                required,
                MAX_RELAY_SINGLE_PAYLOAD_BYTES
            );
        }
    }

    if require_transport_payloads && !expected_handles.is_subset(&payload_handles) {
        bail!("canonical surface references PixelTransport payload that is not attached");
    }

    Ok(usage)
}

fn canonicalize_relay_scene(relay: &mut RelayScene) {
    relay.scene.surfaces.sort_by(|left, right| {
        (left.layer_class, left.placement.z, left.creation_sequence, left.id.as_str()).cmp(&(
            right.layer_class,
            right.placement.z,
            right.creation_sequence,
            right.id.as_str(),
        ))
    });
    relay.pixel_payloads.sort_by(|left, right| left.handle.cmp(&right.handle));
}

fn scene_surface_words(surfaces: &[SurfaceSnapshot]) -> Vec<u32> {
    let mut words = Vec::with_capacity(surfaces.len().saturating_mul(8));
    for surface in surfaces {
        words.extend_from_slice(&[
            surface.placement.x as u32,
            surface.placement.y as u32,
            surface.placement.width,
            surface.placement.height,
            surface.placement.z as u32,
            u32::from(surface.placement.visible),
            surface.buffer_generation as u32,
            surface.creation_sequence as u32,
        ]);
    }
    words
}

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::from_args(env::args().skip(1))?;
    let banner = ServiceBanner::new(ServiceRole::Compd, "scene, focus, composition policy");
    println!("{}", banner.render());

    let accel_policy = global_accel_policy();
    println!(
        "service=compd op=accel_policy event=selected simd={:?} vulkan_enabled={}",
        accel_policy.selected_simd_flavor(),
        accel_policy.prefers_vulkan()
    );
    let vulkan = if config.use_vulkan && accel_policy.prefers_vulkan() {
        let backend = VulkanBackend::new(VulkanBackendConfig::default());
        let caps = backend.initialize();
        println!(
            "service=compd op=vulkan_init event={} compute_available={} driver={} device={}",
            if caps.compute_available { "success" } else { "fallback" },
            caps.compute_available,
            caps.driver_name,
            caps.device_name
        );
        Some(backend)
    } else {
        None
    };

    if config.restore_from_displayd && config.scene_path.is_some() {
        bail!("--scene cannot be combined with --restore-from-displayd");
    }
    if config.handoff_selection && !config.reconcile_waylandd {
        bail!("--handoff-selection requires --reconcile-waylandd");
    }

    let scene = prepare_scene(&config, vulkan.as_ref()).await?;

    if config.handoff_selection {
        apply_selection_handoff(&config, scene.as_ref())?;
    }

    if config.serve_ipc {
        if config.restore_from_displayd || config.reconcile_waylandd {
            match scene.as_ref() {
                Some(scene) => {
                    println!(
                        "service=compd op=startup_rebuild event=scene_ready target={} focus={:?} surfaces={}",
                        scene.target_output,
                        scene.focus,
                        scene.surfaces.len()
                    );
                    match commit_scene_to_displayd(scene) {
                        Ok(committed) => {
                            println!(
                                "service=compd op=startup_rebuild event=scene_committed surface_count={} commit_id={}",
                                committed.surface_count, committed.commit_id
                            );
                        }
                        Err(err) => {
                            if config.require_displayd {
                                return Err(err).context(
                                    "failed to commit rebuilt scene to displayd during startup",
                                );
                            }
                            println!(
                                "service=compd op=startup_rebuild event=failed reason=\"{}\"",
                                err
                            );
                        }
                    }
                }
                None => {
                    println!("service=compd op=startup_rebuild event=skipped reason=no-scene");
                }
            }
        }
        serve_ipc(&config)?;
        return Ok(());
    }

    if config.commit_demo {
        match scene.as_ref() {
            Some(scene) => {
                println!(
                    "service=compd op=scene_build event=success target={} focus={:?} surfaces={}",
                    scene.target_output,
                    scene.focus,
                    scene.surfaces.len()
                );

                match commit_scene_to_displayd(scene) {
                    Ok(committed) => {
                        println!(
                            "service=compd op=displayd_response event=scene_committed surface_count={} commit_id={}",
                            committed.surface_count, committed.commit_id
                        );
                    }
                    Err(err) => {
                        if config.require_displayd {
                            return Err(err)
                                .context("failed to commit scene to displayd (required)");
                        } else {
                            println!(
                                "service=compd op=scene_commit event=failed reason=\"{}\"",
                                err
                            );
                        }
                    }
                }
            }
            None => {
                println!("service=compd op=scene_build event=skipped reason=no-scene");
            }
        }
    }

    if !config.commit_demo && !config.print_scene && scene.is_none() {
        println!("service=compd state=idle (use --commit-demo or --serve-ipc)");
    }

    Ok(())
}

#[derive(Debug, Default)]
struct Config {
    scene_path: Option<PathBuf>,
    commit_demo: bool,
    print_scene: bool,
    require_displayd: bool,
    serve_ipc: bool,
    serve_once: bool,
    fail_resume: bool,
    restore_from_displayd: bool,
    reconcile_waylandd: bool,
    require_waylandd: bool,
    handoff_selection: bool,
    use_vulkan: bool,
    session_instance_id: String,
}

impl Config {
    fn from_args(mut args: impl Iterator<Item = String>) -> Result<Self> {
        let mut config = Self {
            use_vulkan: true,
            session_instance_id: "default-single-session".to_string(),
            ..Self::default()
        };
        // Prefer GPU acceleration; Vulkan initialization remains fail-soft.

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--scene" => {
                    let path = args.next().context("--scene requires a path")?;
                    config.scene_path = Some(PathBuf::from(path));
                }
                "--commit-demo" => config.commit_demo = true,
                "--print-scene" => config.print_scene = true,
                "--require-displayd" => config.require_displayd = true,
                "--serve-ipc" => config.serve_ipc = true,
                "--once" => config.serve_once = true,
                "--fail-resume" => config.fail_resume = true,
                "--restore-from-displayd" => config.restore_from_displayd = true,
                "--reconcile-waylandd" => config.reconcile_waylandd = true,
                "--require-waylandd" => config.require_waylandd = true,
                "--handoff-selection" => config.handoff_selection = true,
                "--vulkan" => config.use_vulkan = true,
                "--no-vulkan" => config.use_vulkan = false,
                "--session-instance-id" => {
                    config.session_instance_id =
                        args.next().context("--session-instance-id requires an id")?;
                }
                "--help" | "-h" => {
                    println!(
                        "usage: compd [--scene PATH] [--print-scene] [--commit-demo] [--require-displayd] [--require-waylandd] [--restore-from-displayd] [--reconcile-waylandd] [--handoff-selection] [--serve-ipc] [--once] [--fail-resume] [--vulkan|--no-vulkan] [--session-instance-id ID]"
                    );
                    std::process::exit(0);
                }
                _ => bail!("unknown argument: {arg}"),
            }
        }

        Ok(config)
    }
}

async fn prepare_scene(
    config: &Config,
    vulkan: Option<&VulkanBackend>,
) -> Result<Option<CompdScene>> {
    if !config.restore_from_displayd
        && config.scene_path.is_none()
        && !config.reconcile_waylandd
        && !config.commit_demo
        && !config.print_scene
    {
        return Ok(None);
    }

    let recovered_scene = if config.restore_from_displayd {
        match query_scene_snapshot_from_displayd(None) {
            Ok(Some(snapshot)) => {
                println!(
                    "service=compd op=scene_recover event=success source={} commit_id={} surfaces={} timestamp={}",
                    snapshot.source.as_str(),
                    snapshot.commit_id,
                    snapshot.surfaces.len(),
                    snapshot.unix_timestamp
                );
                Some(snapshot)
            }
            Ok(None) => {
                if config.require_displayd {
                    bail!("displayd has no committed scene snapshot");
                }
                println!("service=compd op=scene_recover event=empty");
                None
            }
            Err(err) => {
                if config.require_displayd {
                    return Err(err).context("failed to recover scene from displayd");
                }
                println!("service=compd op=scene_recover event=failed reason=\"{}\"", err);
                None
            }
        }
    } else {
        None
    };

    let mut scene = match recovered_scene.as_ref() {
        Some(snapshot) => Some(scene_from_snapshot(snapshot)),
        None if config.restore_from_displayd => None,
        None => Some(load_scene(config.scene_path.as_ref())?),
    };

    if config.reconcile_waylandd {
        scene = reconcile_scene(config, scene, vulkan).await?;
    }

    if config.print_scene {
        if let Some(scene) = scene.as_ref() {
            println!(
                "service=compd op=scene_print event=success target={} surfaces={}",
                scene.target_output,
                scene.surfaces.len()
            );
            println!("{}", serde_json::to_string_pretty(scene)?);
        } else {
            println!("service=compd op=scene_print event=skipped reason=no-scene");
        }
    }

    Ok(scene)
}

fn query_output_inventory_from_displayd() -> Result<Vec<waybroker_common::OutputMode>> {
    match send_display_command(&DisplayCommand::EnumerateOutputs)? {
        DisplayEvent::OutputInventory { outputs } => Ok(outputs),
        DisplayEvent::Rejected { reason } => {
            bail!("displayd rejected inventory query: {reason}")
        }
        other => bail!("unexpected displayd response: {other:?}"),
    }
}

async fn reconcile_scene(
    config: &Config,
    scene: Option<CompdScene>,
    vulkan: Option<&VulkanBackend>,
) -> Result<Option<CompdScene>> {
    let Some(scene) = scene else {
        println!("service=compd op=scene_reconcile event=skipped reason=no-scene");
        return Ok(None);
    };

    let target_output_name = scene.target_output.clone();
    let outputs = query_output_inventory_from_displayd().unwrap_or_default();
    let default_output = waybroker_common::OutputMode {
        name: target_output_name.clone(),
        width: STUB_SCREEN_WIDTH,
        height: STUB_SCREEN_HEIGHT,
        refresh_hz: 60,
    };
    let output_mode =
        outputs.into_iter().find(|o| o.name == target_output_name).unwrap_or(default_output);

    match query_surface_registry_from_waylandd() {
        Ok(snapshot) => {
            println!(
                "service=compd op=surface_registry event=success generation={} surfaces={} timestamp={}",
                snapshot.generation,
                snapshot.surfaces.len(),
                snapshot.unix_timestamp
            );

            if let Some(vulkan) = vulkan {
                let surface_words = scene_surface_words(&scene.surfaces);
                let handle = vulkan.submit_batch(VulkanBatchSubmission {
                    workload: VulkanWorkloadClass::BulkPrefilter,
                    payload_len: surface_words.len().saturating_mul(std::mem::size_of::<u32>()),
                    surface_words: Some(surface_words),
                    timeout: Duration::from_millis(100),
                    requires_zeroize: false,
                    allows_gpu: true,
                });
                let result = vulkan.wait_for_completion(handle).await;
                println!(
                    "service=compd op=vulkan_prefilter event=completed workload={:?} path={:?} fallback_reason={:?}",
                    result.workload, result.path, result.fallback_reason
                );
            }

            let reconciled = reconcile_scene_with_registry(scene, &snapshot, &output_mode);
            println!(
                "service=compd op=scene_reconcile event=success kept={} dropped={} app_id_updates={} selection_handoffs={} focus={:?} clipboard_owner={} primary_selection_owner={}",
                reconciled.scene.surfaces.len(),
                reconciled.dropped_surface_ids.len(),
                reconciled.updated_app_ids,
                reconciled.selection_handoffs,
                reconciled.scene.focus,
                format_owner(reconciled.scene.selection.clipboard_owner.as_deref()),
                format_owner(reconciled.scene.selection.primary_selection_owner.as_deref())
            );
            if !reconciled.dropped_surface_ids.is_empty() {
                println!(
                    "service=compd op=scene_reconcile dropped_ids={}",
                    reconciled.dropped_surface_ids.join(",")
                );
            }
            Ok(Some(reconciled.scene))
        }
        Err(err) => {
            if config.require_waylandd {
                return Err(err).context("failed to reconcile scene with waylandd");
            }
            println!("service=compd op=surface_registry event=failed reason=\"{}\"", err);
            Ok(Some(scene))
        }
    }
}

fn serve_ipc(config: &Config) -> Result<()> {
    let listener = bind_service_socket(ServiceRole::Compd)?;
    let _socket_guard = SocketGuard::new(listener.endpoint().clone());
    println!("service=compd op=listen event=socket_bound path={}", listener.endpoint());

    let mut relay_state = CompdRelayState::default();
    let mut served = 0usize;
    for stream in listener.incoming() {
        let stream = match stream {
            Ok(stream) => stream,
            Err(err) if is_recoverable_accept_error(&err) => {
                println!("service=compd op=accept event=recoverable_error reason=\"{}\"", err);
                continue;
            }
            Err(err) => {
                println!("service=compd op=accept event=fatal_error reason=\"{}\"", err);
                return Err(err).context("compd IPC accept failed");
            }
        };
        handle_client(stream, config, &mut relay_state)?;
        served += 1;

        if config.serve_once {
            break;
        }
    }

    let pending_reason =
        relay_state.pending.as_ref().map(|pending| pending.reason.as_str()).unwrap_or("none");
    let pending_usage = relay_state
        .pending
        .as_ref()
        .map(|pending| pending.usage)
        .unwrap_or(RelayResourceUsage { surfaces: 0, pixel_payloads: 0, pixel_bytes: 0 });
    println!(
        "service=compd op=terminate event=finished served_requests={} relayed={} deferred={} superseded={} pending_surfaces={} pending_payloads={} pending_bytes={} pending_reason={:?}",
        served,
        relay_state.accepted_count,
        relay_state.deferred_count,
        relay_state.superseded_count,
        pending_usage.surfaces,
        pending_usage.pixel_payloads,
        pending_usage.pixel_bytes,
        pending_reason
    );
    Ok(())
}

fn handle_client(
    mut stream: ServiceStream,
    config: &Config,
    relay_state: &mut CompdRelayState,
) -> Result<()> {
    stream.set_read_timeout(Some(IPC_REQUEST_TIMEOUT))?;
    stream.set_write_timeout(Some(IPC_REQUEST_TIMEOUT))?;
    let request: IpcEnvelope = {
        let mut reader = BufReader::new(stream.try_clone()?);
        read_ipc_envelope(&mut reader)?
    };

    let response = build_response(request, config, relay_state);
    send_ipc_envelope(&mut stream, &response)?;
    Ok(())
}

fn build_response(
    request: IpcEnvelope,
    config: &Config,
    relay_state: &mut CompdRelayState,
) -> IpcEnvelope {
    let source = request.source;
    let response_kind = match request.kind {
        MessageKind::DisplayCommand(command) if request.destination == ServiceRole::Compd => {
            match forward_display_command(command, relay_state) {
                Ok(event) => MessageKind::DisplayEvent(event),
                Err(err) => {
                    MessageKind::DisplayEvent(DisplayEvent::Rejected { reason: err.to_string() })
                }
            }
        }
        MessageKind::SessionCommand(waybroker_common::SessionCommand::ResumeHint {
            stage,
            output,
        }) if request.destination == ServiceRole::Compd => {
            if config.fail_resume {
                println!(
                    "service=compd op=resume_hint event=failed reason=\"fault injection\" stage={:?}",
                    stage
                );
                MessageKind::SessionCommand(waybroker_common::SessionCommand::DegradedMode {
                    reason: "compd fault injection".into(),
                })
            } else {
                println!(
                    "service=compd op=resume_hint event=success stage={:?} output={:?}",
                    stage, output
                );
                MessageKind::SessionCommand(waybroker_common::SessionCommand::ResumeHint {
                    stage,
                    output,
                })
            }
        }
        MessageKind::LockCommand(waybroker_common::LockCommand::SetLockState { state }) => {
            println!("service=compd op=lock_state_hint event=success state={:?}", state);
            MessageKind::LockCommand(waybroker_common::LockCommand::SetLockState { state })
        }
        other => MessageKind::SessionCommand(waybroker_common::SessionCommand::DegradedMode {
            reason: format!("compd does not handle {other:?}"),
        }),
    };

    IpcEnvelope::new(ServiceRole::Compd, source, response_kind)
}

fn send_display_command(command: &DisplayCommand) -> Result<DisplayEvent> {
    let mut stream = connect_service_socket(ServiceRole::Displayd)
        .context("compd could not connect to displayd")?;
    stream.set_read_timeout(Some(IPC_REQUEST_TIMEOUT))?;
    stream.set_write_timeout(Some(IPC_REQUEST_TIMEOUT))?;
    send_ipc_display_command(&mut stream, ServiceRole::Compd, ServiceRole::Displayd, command)?;
    let mut reader = BufReader::new(stream);
    let response: IpcEnvelope = read_ipc_envelope(&mut reader)?;
    if response.source != ServiceRole::Displayd {
        bail!("displayd returned response from {}", response.source.as_str());
    }
    if response.destination != ServiceRole::Compd {
        bail!("displayd returned response addressed to {}", response.destination.as_str());
    }
    match response.kind {
        MessageKind::DisplayEvent(event) => Ok(event),
        other => bail!("displayd returned unexpected response: {other:?}"),
    }
}

fn scene_committed_event(relay: &RelayScene, receipt: SceneCommitReceipt) -> DisplayEvent {
    DisplayEvent::SceneCommitted {
        target: CommitTarget::Output { name: relay.scene.target_output.clone() },
        focus: relay.scene.focus.clone(),
        selection: relay.scene.selection.clone(),
        surface_count: receipt.surface_count,
        commit_id: receipt.commit_id,
        publication: None,
    }
}

fn matching_snapshot_receipt(relay: &RelayScene) -> Result<Option<SceneCommitReceipt>> {
    let snapshot = query_scene_snapshot_from_displayd(Some(relay.scene.target_output.as_str()))?;
    let Some(snapshot) = snapshot else {
        return Ok(None);
    };
    let snapshot_output = match &snapshot.target {
        CommitTarget::Output { name } => name,
    };
    if snapshot_output != &relay.scene.target_output
        || snapshot.scene_epoch != relay.scene.scene_epoch
        || snapshot.scene_generation != relay.scene.scene_generation
    {
        return Ok(None);
    }
    if snapshot.commit_id == 0 {
        bail!("matching displayd scene snapshot has invalid commit id");
    }
    Ok(Some(SceneCommitReceipt {
        surface_count: snapshot.surfaces.len(),
        commit_id: snapshot.commit_id,
    }))
}

fn forward_display_command(
    command: DisplayCommand,
    relay_state: &mut CompdRelayState,
) -> Result<DisplayEvent> {
    match command {
        DisplayCommand::CommitScene {
            target,
            focus,
            selection,
            surfaces,
            pixel_payloads,
            scene_epoch,
            scene_generation,
        } => {
            let target_output = match target {
                CommitTarget::Output { name } => name,
            };
            let mut relay = RelayScene {
                scene: CompdScene {
                    target_output,
                    focus,
                    selection,
                    surfaces,
                    scene_epoch,
                    scene_generation,
                },
                pixel_payloads,
            };
            canonicalize_relay_scene(&mut relay);
            let usage = match validate_relay_scene(&relay, true) {
                Ok(usage) => usage,
                Err(err) => {
                    return Ok(DisplayEvent::Rejected {
                        reason: format!("compd rejected malformed scene: {err}"),
                    });
                }
            };

            if relay_state.should_reject_version(relay.version()) {
                return Ok(DisplayEvent::Rejected {
                    reason: format!(
                        "compd rejected stale scene epoch={} generation={} floor={:?}",
                        relay.scene.scene_epoch,
                        relay.scene.scene_generation,
                        relay_state.version_floor()
                    ),
                });
            }

            let retrying_deferred_version = relay_state
                .pending
                .as_ref()
                .map(|pending| pending.version == relay.version())
                .unwrap_or(false);
            if retrying_deferred_version {
                if let Ok(Some(receipt)) = matching_snapshot_receipt(&relay) {
                    relay_state.note_forwarded(&relay);
                    return Ok(scene_committed_event(&relay, receipt));
                }
            }

            println!(
                "service=compd op=scene_admission event=accepted epoch={} generation={} surfaces={} payloads={} bytes={}",
                relay.scene.scene_epoch,
                relay.scene.scene_generation,
                usage.surfaces,
                usage.pixel_payloads,
                usage.pixel_bytes
            );

            let direct_command = relay.take_commit_command();
            let direct_result = send_display_command(&direct_command);
            relay.restore_payloads_from_command(direct_command);
            match direct_result {
                Ok(event @ DisplayEvent::SceneCommitted { .. }) => {
                    relay_state.note_forwarded(&relay);
                    mark_display_connected(0);
                    Ok(event)
                }
                Ok(event @ DisplayEvent::Rejected { .. }) => {
                    if let Ok(Some(receipt)) = matching_snapshot_receipt(&relay) {
                        relay_state.note_forwarded(&relay);
                        Ok(scene_committed_event(&relay, receipt))
                    } else {
                        Ok(event)
                    }
                }
                Ok(other) => bail!("displayd returned unexpected scene response: {other:?}"),
                Err(transport_error) => {
                    mark_display_disconnected();
                    match reconcile_relay_scene_to_displayd(&mut relay) {
                        Ok(receipt) => {
                            relay_state.note_forwarded(&relay);
                            Ok(scene_committed_event(&relay, receipt))
                        }
                        Err(reconcile_error) => {
                            let reason = format!(
                                "compd deferred latest scene after displayd transport loss: {transport_error}; reconciliation failed: {reconcile_error}"
                            );
                            relay_state.defer_latest(&relay, reason.clone());
                            Ok(DisplayEvent::Rejected { reason })
                        }
                    }
                }
            }
        }
        DisplayCommand::ReconcileScene {
            epoch,
            scene_epoch,
            scene_generation,
            target,
            focus,
            selection,
            surfaces,
            pixel_payloads,
        } => {
            let target_output = match target {
                CommitTarget::Output { name } => name,
            };
            let mut relay = RelayScene {
                scene: CompdScene {
                    target_output,
                    focus,
                    selection,
                    surfaces,
                    scene_epoch,
                    scene_generation,
                },
                pixel_payloads,
            };
            canonicalize_relay_scene(&mut relay);
            if let Err(err) = validate_relay_scene(&relay, true) {
                return Ok(DisplayEvent::Rejected {
                    reason: format!("compd rejected malformed reconciliation scene: {err}"),
                });
            }
            if relay_state.should_reject_version(relay.version()) {
                return Ok(DisplayEvent::Rejected {
                    reason: format!(
                        "compd rejected stale reconciliation scene epoch={} generation={} floor={:?}",
                        relay.scene.scene_epoch,
                        relay.scene.scene_generation,
                        relay_state.version_floor()
                    ),
                });
            }
            let reconcile_command = relay.take_reconcile_command(epoch);
            let reconcile_result = send_display_command(&reconcile_command);
            relay.restore_payloads_from_command(reconcile_command);
            match reconcile_result {
                Ok(event @ DisplayEvent::Reconciled { .. }) => {
                    relay_state.note_forwarded(&relay);
                    Ok(event)
                }
                Ok(event) => Ok(event),
                Err(err) => {
                    let reason = format!("compd reconciliation forward failed: {err}");
                    relay_state.defer_latest(&relay, reason.clone());
                    Ok(DisplayEvent::Rejected { reason })
                }
            }
        }
        other => send_display_command(&other),
    }
}

struct SocketGuard {
    endpoint: ServiceEndpoint,
}

impl SocketGuard {
    fn new(endpoint: ServiceEndpoint) -> Self {
        Self { endpoint }
    }
}

impl Drop for SocketGuard {
    fn drop(&mut self) {
        let _ = self.endpoint.cleanup_stale();
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CompdScene {
    target_output: String,
    focus: FocusTarget,
    #[serde(default)]
    selection: WaylandSelectionState,
    surfaces: Vec<SurfaceSnapshot>,
    #[serde(default)]
    scene_epoch: u64,
    #[serde(default)]
    scene_generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DisplayLinkState {
    Connected { epoch: u64 },
    Disconnected,
    Reconnecting { attempt: u8 },
    Reconciling { epoch: u64 },
    AwaitingFreshPresentation { epoch: u64 },
    Failed,
}

static DISPLAY_LINK: OnceLock<Mutex<DisplayLinkState>> = OnceLock::new();

fn display_link() -> &'static Mutex<DisplayLinkState> {
    DISPLAY_LINK.get_or_init(|| Mutex::new(DisplayLinkState::Disconnected))
}

fn load_scene(path: Option<&PathBuf>) -> Result<CompdScene> {
    match path {
        Some(path) => {
            let raw = fs::read_to_string(path)
                .with_context(|| format!("failed to read scene file {}", path.display()))?;
            serde_json::from_str(&raw)
                .with_context(|| format!("failed to decode scene file {}", path.display()))
        }
        None => Ok(mock_demo_scene()),
    }
}

fn mock_demo_scene() -> CompdScene {
    CompdScene {
        target_output: "eDP-1".into(),
        focus: FocusTarget::Surface { id: "konsole-1".into() },
        selection: WaylandSelectionState::default(),
        surfaces: vec![
            SurfaceSnapshot {
                id: "konsole-1".into(),
                app_id: "org.kde.konsole".into(),
                placement: SurfacePlacement {
                    x: 100,
                    y: 100,
                    width: 800,
                    height: 600,
                    z: 10,
                    visible: true,
                },
                ..Default::default()
            },
            SurfaceSnapshot {
                id: "background-1".into(),
                app_id: "org.kde.plasmashell.wallpaper".into(),
                placement: SurfacePlacement {
                    x: 0,
                    y: 0,
                    width: 1920,
                    height: 1080,
                    z: 0,
                    visible: true,
                },
                ..Default::default()
            },
        ],
        scene_epoch: 1,
        scene_generation: 1,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SceneCommitReceipt {
    surface_count: usize,
    commit_id: u64,
}

fn commit_scene_to_displayd(scene: &CompdScene) -> Result<SceneCommitReceipt> {
    let mut relay = RelayScene { scene: scene.clone(), pixel_payloads: Vec::new() };
    validate_relay_scene(&relay, false)?;
    let command = relay.take_commit_command();
    let result = send_display_command(&command);
    relay.restore_payloads_from_command(command);
    match result {
        Ok(DisplayEvent::SceneCommitted { surface_count, commit_id, .. }) => {
            mark_display_connected(0);
            Ok(SceneCommitReceipt { surface_count, commit_id })
        }
        Ok(DisplayEvent::Rejected { reason }) => {
            if let Ok(Some(receipt)) = matching_snapshot_receipt(&relay) {
                Ok(receipt)
            } else {
                bail!("displayd rejected scene: {reason}")
            }
        }
        Ok(other) => bail!("unexpected displayd response: {other:?}"),
        Err(err) => {
            mark_display_disconnected();
            reconcile_relay_scene_to_displayd(&mut relay).map_err(|reconcile_err| {
                anyhow::anyhow!(
                    "displayd transport lost ({err}); reconciliation failed: {reconcile_err}"
                )
            })
        }
    }
}

fn mark_display_connected(epoch: u64) {
    if let Ok(mut state) = display_link().lock() {
        *state = DisplayLinkState::Connected { epoch };
    }
}

fn mark_display_disconnected() {
    if let Ok(mut state) = display_link().lock() {
        *state = DisplayLinkState::Disconnected;
    }
}

fn reconciliation_commit_id(
    snapshot: Option<CommittedSceneState>,
    relay: &RelayScene,
) -> Result<u64> {
    let snapshot = snapshot.context("displayd reconciliation produced no scene snapshot")?;
    let snapshot_output = match &snapshot.target {
        CommitTarget::Output { name } => name,
    };
    if snapshot_output != &relay.scene.target_output {
        bail!(
            "displayd reconciliation snapshot target mismatch: {} != {}",
            snapshot_output,
            relay.scene.target_output
        );
    }
    if snapshot.scene_epoch != relay.scene.scene_epoch
        || snapshot.scene_generation != relay.scene.scene_generation
    {
        bail!(
            "displayd reconciliation snapshot version mismatch: epoch={} generation={} expected epoch={} generation={}",
            snapshot.scene_epoch,
            snapshot.scene_generation,
            relay.scene.scene_epoch,
            relay.scene.scene_generation
        );
    }
    if snapshot.commit_id == 0 {
        bail!("displayd reconciliation snapshot has invalid commit id");
    }
    Ok(snapshot.commit_id)
}

fn reconcile_relay_scene_to_displayd(relay: &mut RelayScene) -> Result<SceneCommitReceipt> {
    let mut last_error = None;

    for attempt in 1..=DISPLAY_RECONNECT_RETRY_LIMIT {
        {
            let mut state =
                display_link().lock().map_err(|_| anyhow::anyhow!("display link lock poisoned"))?;
            *state = DisplayLinkState::Reconnecting { attempt };
        }

        let reconciliation = match send_display_command(&DisplayCommand::GetReconciliation) {
            Ok(event) => event,
            Err(err) => {
                last_error = Some(err.context("failed reconciliation query"));
                continue;
            }
        };
        let epoch = match reconciliation {
            DisplayEvent::Reconciliation { epoch, .. } => epoch,
            DisplayEvent::Rejected { reason } => {
                bail!("displayd rejected reconciliation query: {reason}")
            }
            other => bail!("unexpected reconciliation response: {other:?}"),
        };

        {
            let mut link =
                display_link().lock().map_err(|_| anyhow::anyhow!("display link lock poisoned"))?;
            *link = DisplayLinkState::Reconciling { epoch };
        }

        match send_display_command(&DisplayCommand::BeginReconciliation { epoch }) {
            Ok(DisplayEvent::Reconciliation { epoch: accepted, .. }) if accepted == epoch => {}
            Ok(DisplayEvent::Rejected { reason }) => {
                bail!("displayd rejected reconciliation begin: {reason}")
            }
            Ok(other) => bail!("unexpected reconciliation begin response: {other:?}"),
            Err(err) => {
                last_error = Some(err.context("failed reconciliation begin"));
                continue;
            }
        }

        let reconcile_command = relay.take_reconcile_command(epoch);
        let reconcile_result = send_display_command(&reconcile_command);
        relay.restore_payloads_from_command(reconcile_command);
        let event = match reconcile_result {
            Ok(event) => event,
            Err(err) => {
                if let Ok(Some(receipt)) = matching_snapshot_receipt(relay) {
                    return Ok(receipt);
                }
                last_error = Some(err.context("failed scene reconciliation"));
                continue;
            }
        };
        match event {
            DisplayEvent::Reconciled { epoch: accepted, scene_generation, .. }
                if accepted == epoch && scene_generation == relay.scene.scene_generation =>
            {
                let snapshot =
                    query_scene_snapshot_from_displayd(Some(relay.scene.target_output.as_str()))?;
                let commit_id = reconciliation_commit_id(snapshot, relay)?;
                let mut link = display_link()
                    .lock()
                    .map_err(|_| anyhow::anyhow!("display link lock poisoned"))?;
                *link = DisplayLinkState::AwaitingFreshPresentation { epoch };
                return Ok(SceneCommitReceipt {
                    surface_count: relay.scene.surfaces.len(),
                    commit_id,
                });
            }
            DisplayEvent::Rejected { reason } => {
                if let Ok(Some(receipt)) = matching_snapshot_receipt(relay) {
                    return Ok(receipt);
                }
                if let Ok(mut link) = display_link().lock() {
                    *link = DisplayLinkState::Failed;
                }
                bail!("displayd rejected scene reconciliation: {reason}")
            }
            other => bail!("unexpected scene reconciliation response: {other:?}"),
        }
    }

    if let Ok(mut state) = display_link().lock() {
        *state = DisplayLinkState::Failed;
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("displayd reconnect retry budget exhausted")))
}

fn query_scene_snapshot_from_displayd(output: Option<&str>) -> Result<Option<CommittedSceneState>> {
    match send_display_command(&DisplayCommand::GetSceneSnapshot {
        output: output.map(str::to_owned),
    })? {
        DisplayEvent::SceneSnapshot { snapshot } => Ok(snapshot),
        DisplayEvent::Rejected { reason } => {
            bail!("displayd rejected scene snapshot query: {reason}")
        }
        other => bail!("unexpected displayd response: {other:?}"),
    }
}

fn query_surface_registry_from_waylandd() -> Result<SurfaceRegistrySnapshot> {
    let mut stream = connect_service_socket(ServiceRole::Waylandd)
        .context("failed to connect to waylandd socket")?;
    let request = IpcEnvelope::new(
        ServiceRole::Compd,
        ServiceRole::Waylandd,
        MessageKind::WaylandCommand(WaylandCommand::GetSurfaceRegistry),
    );
    send_ipc_envelope(&mut stream, &request)
        .context("failed to query surface registry from waylandd")?;

    let mut reader = BufReader::new(stream);
    let response: IpcEnvelope =
        read_ipc_envelope(&mut reader).context("failed to read surface registry from waylandd")?;

    if response.source != ServiceRole::Waylandd {
        bail!("unexpected response source: {}", response.source.as_str());
    }

    if response.destination != ServiceRole::Compd {
        bail!("unexpected response destination: {}", response.destination.as_str());
    }

    match response.kind {
        MessageKind::WaylandEvent(WaylandEvent::SurfaceRegistry { snapshot }) => Ok(snapshot),
        MessageKind::WaylandEvent(WaylandEvent::Rejected { reason }) => {
            bail!("waylandd rejected surface registry query: {reason}")
        }
        other => bail!("unexpected waylandd response: {other:?}"),
    }
}

fn send_selection_handoff_to_waylandd(
    focus: &FocusTarget,
    selection: &WaylandSelectionState,
) -> Result<u64> {
    let mut stream = connect_service_socket(ServiceRole::Waylandd)
        .context("failed to connect to waylandd socket")?;
    let request = IpcEnvelope::new(
        ServiceRole::Compd,
        ServiceRole::Waylandd,
        MessageKind::WaylandCommand(WaylandCommand::ApplySelectionHandoff {
            handoff: WaylandSelectionHandoff { focus: focus.clone(), selection: selection.clone() },
        }),
    );
    send_ipc_envelope(&mut stream, &request)
        .context("failed to send selection handoff to waylandd")?;

    let mut reader = BufReader::new(stream);
    let response: IpcEnvelope =
        read_ipc_envelope(&mut reader).context("failed to read selection handoff response")?;

    if response.source != ServiceRole::Waylandd {
        bail!("unexpected response source: {}", response.source.as_str());
    }

    if response.destination != ServiceRole::Compd {
        bail!("unexpected response destination: {}", response.destination.as_str());
    }

    match response.kind {
        MessageKind::WaylandEvent(WaylandEvent::SelectionHandoffApplied { generation, .. }) => {
            Ok(generation)
        }
        MessageKind::WaylandEvent(WaylandEvent::Rejected { reason }) => {
            bail!("waylandd rejected selection handoff: {reason}")
        }
        other => bail!("unexpected waylandd response: {other:?}"),
    }
}

fn apply_selection_handoff(config: &Config, scene: Option<&CompdScene>) -> Result<()> {
    let Some(scene) = scene else {
        println!("service=compd op=selection_handoff event=skipped reason=no-scene");
        return Ok(());
    };

    match send_selection_handoff_to_waylandd(&scene.focus, &scene.selection) {
        Ok(generation) => {
            println!(
                "service=compd op=selection_handoff event=success generation={} focus={:?} clipboard_owner={} primary_selection_owner={}",
                generation,
                scene.focus,
                format_owner(scene.selection.clipboard_owner.as_deref()),
                format_owner(scene.selection.primary_selection_owner.as_deref())
            );
            Ok(())
        }
        Err(err) => {
            if config.require_waylandd {
                Err(err).context("failed to apply selection handoff to waylandd")
            } else {
                println!("service=compd op=selection_handoff event=failed reason=\"{}\"", err);
                Ok(())
            }
        }
    }
}

fn scene_from_snapshot(snapshot: &CommittedSceneState) -> CompdScene {
    CompdScene {
        target_output: match &snapshot.target {
            CommitTarget::Output { name } => name.clone(),
        },
        focus: snapshot.focus.clone(),
        selection: snapshot.selection.clone(),
        surfaces: snapshot.surfaces.clone(),
        scene_epoch: snapshot.scene_epoch,
        scene_generation: snapshot.scene_generation,
    }
}

#[derive(Debug)]
struct SceneReconcileResult {
    scene: CompdScene,
    dropped_surface_ids: Vec<String>,
    updated_app_ids: usize,
    selection_handoffs: usize,
}

fn reconcile_scene_with_registry(
    scene: CompdScene,
    registry: &SurfaceRegistrySnapshot,
    output_mode: &waybroker_common::OutputMode,
) -> SceneReconcileResult {
    let active_registry: BTreeMap<&str, &WaylandSurfaceState> = registry
        .surfaces
        .iter()
        .filter(|surface| surface.mapped && surface.buffer_attached)
        .map(|surface| (surface.id.as_str(), surface))
        .collect();

    let mut kept_surfaces = Vec::with_capacity(scene.surfaces.len());
    let mut dropped_surface_ids = Vec::new();
    let mut updated_app_ids = 0usize;

    for mut surface in scene.surfaces {
        match active_registry.get(surface.id.as_str()) {
            Some(registry_surface) => {
                if surface.app_id != registry_surface.app_id {
                    surface.app_id = registry_surface.app_id.clone();
                    updated_app_ids += 1;
                }
                let buffer_generation_changed =
                    surface.buffer_generation != registry_surface.buffer_generation;
                surface.buffer_handle = registry_surface.buffer_handle.clone();
                surface.buffer_generation = registry_surface.buffer_generation;
                surface.damage_rects = registry_surface.damage_rects.clone();
                if buffer_generation_changed {
                    surface.pixel_transport = None;
                }
                kept_surfaces.push(surface);
            }
            None => dropped_surface_ids.push(surface.id),
        }
    }

    let focus = reconcile_focus(&scene.focus, &kept_surfaces, &active_registry);
    let (selection, selection_handoffs) =
        reconcile_selection(&registry.selection, &focus, &active_registry);

    // Apply basic layout rules based on roles
    for surface in &mut kept_surfaces {
        if let Some(reg) = active_registry.get(surface.id.as_str()) {
            apply_role_based_layout(surface, &reg.role, output_mode);
        }
    }

    SceneReconcileResult {
        scene: CompdScene {
            target_output: scene.target_output,
            focus,
            selection,
            surfaces: kept_surfaces,
            scene_epoch: scene.scene_epoch,
            scene_generation: scene.scene_generation,
        },
        dropped_surface_ids,
        updated_app_ids,
        selection_handoffs,
    }
}

const STUB_SCREEN_WIDTH: u32 = 1920;
const STUB_SCREEN_HEIGHT: u32 = 1080;
const STUB_PANEL_HEIGHT: u32 = 36;

fn apply_role_based_layout(
    surface: &mut SurfaceSnapshot,
    role: &WaylandSurfaceRole,
    output: &waybroker_common::OutputMode,
) {
    match role {
        WaylandSurfaceRole::Background => {
            surface.placement.x = 0;
            surface.placement.y = 0;
            surface.placement.width = output.width;
            surface.placement.height = output.height;
            surface.placement.z = 0;
        }
        WaylandSurfaceRole::Layer(metadata) => {
            let width = if metadata.anchor & 1 != 0 && metadata.anchor & 2 != 0 {
                output
                    .width
                    .saturating_sub((metadata.margin_left + metadata.margin_right).max(0) as u32)
            } else {
                STUB_SCREEN_WIDTH
            };

            let height = if metadata.exclusive_zone > 0 {
                metadata.exclusive_zone as u32
            } else {
                STUB_PANEL_HEIGHT
            };

            surface.placement.x = metadata.margin_left;

            if metadata.anchor & 4 != 0 {
                // Anchor Top
                surface.placement.y = metadata.margin_top;
            } else if metadata.anchor & 8 != 0 {
                // Anchor Bottom
                surface.placement.y =
                    (output.height.saturating_sub(height)) as i32 - metadata.margin_bottom;
            } else {
                surface.placement.y = 0;
            }

            surface.placement.width = width;
            surface.placement.height = height;

            surface.placement.z = match metadata.layer {
                0 => 10,  // Background
                1 => 20,  // Bottom
                2 => 100, // Top
                3 => 200, // Overlay
                _ => 100,
            };
        }
        #[allow(clippy::collapsible_match)]
        WaylandSurfaceRole::Toplevel => {
            if surface.placement.z < 30 {
                surface.placement.z = 30;
            }
        }
        _ => {}
    }
}

fn reconcile_focus(
    previous_focus: &FocusTarget,
    surfaces: &[SurfaceSnapshot],
    active_registry: &BTreeMap<&str, &WaylandSurfaceState>,
) -> FocusTarget {
    match previous_focus {
        FocusTarget::Surface { id }
            if active_registry
                .get(id.as_str())
                .is_some_and(|surface| is_focusable_role(&surface.role)) =>
        {
            FocusTarget::Surface { id: id.clone() }
        }
        _ => fallback_focus_target(surfaces, active_registry),
    }
}

fn fallback_focus_target(
    surfaces: &[SurfaceSnapshot],
    active_registry: &BTreeMap<&str, &WaylandSurfaceState>,
) -> FocusTarget {
    surfaces
        .iter()
        .filter(|surface| surface.placement.visible)
        .filter_map(|surface| {
            active_registry.get(surface.id.as_str()).and_then(|registry_surface| {
                is_focusable_role(&registry_surface.role)
                    .then_some((surface.placement.z, &surface.id))
            })
        })
        .max_by_key(|(z, _)| *z)
        .map(|(_, id)| FocusTarget::Surface { id: id.clone() })
        .unwrap_or(FocusTarget::None)
}

fn reconcile_selection(
    previous_selection: &WaylandSelectionState,
    focus: &FocusTarget,
    active_registry: &BTreeMap<&str, &WaylandSurfaceState>,
) -> (WaylandSelectionState, usize) {
    let (clipboard_owner, clipboard_payload_id, clipboard_source_serial, clipboard_changed) =
        reconcile_selection_slot(
            previous_selection.clipboard_owner.as_deref(),
            previous_selection.clipboard_payload_id.as_deref(),
            previous_selection.clipboard_source_serial,
            focus,
            active_registry,
        );
    let (
        primary_selection_owner,
        primary_selection_payload_id,
        primary_selection_source_serial,
        primary_changed,
    ) = reconcile_selection_slot(
        previous_selection.primary_selection_owner.as_deref(),
        previous_selection.primary_selection_payload_id.as_deref(),
        previous_selection.primary_selection_source_serial,
        focus,
        active_registry,
    );

    (
        WaylandSelectionState {
            clipboard_owner,
            clipboard_payload_id,
            clipboard_source_serial,
            clipboard_offer: previous_selection.clipboard_offer.clone(),
            primary_selection_owner,
            primary_selection_payload_id,
            primary_selection_source_serial,
            primary_offer: previous_selection.primary_offer.clone(),
        },
        usize::from(clipboard_changed) + usize::from(primary_changed),
    )
}

fn reconcile_selection_slot(
    previous_owner: Option<&str>,
    previous_payload_id: Option<&str>,
    previous_source_serial: Option<u64>,
    focus: &FocusTarget,
    active_registry: &BTreeMap<&str, &WaylandSurfaceState>,
) -> (Option<String>, Option<String>, Option<u64>, bool) {
    match previous_owner {
        Some(id) if active_registry.contains_key(id) => (
            Some(id.to_owned()),
            previous_payload_id.map(str::to_owned),
            previous_source_serial,
            false,
        ),
        Some(_) => {
            let handoff = match focus {
                FocusTarget::Surface { id }
                    if active_registry
                        .get(id.as_str())
                        .is_some_and(|surface| is_focusable_role(&surface.role)) =>
                {
                    Some(id.clone())
                }
                _ => None,
            };
            (handoff, None, None, true)
        }
        None => {
            let metadata_changed =
                previous_payload_id.is_some() || previous_source_serial.is_some();
            (None, None, None, metadata_changed)
        }
    }
}

fn format_owner(owner: Option<&str>) -> &str {
    owner.unwrap_or("none")
}

fn is_focusable_role(role: &WaylandSurfaceRole) -> bool {
    matches!(
        role,
        WaylandSurfaceRole::Toplevel | WaylandSurfaceRole::Popup | WaylandSurfaceRole::Lock
    )
}

#[cfg(test)]
mod tests {
    use super::{
        CompdScene, apply_role_based_layout, mock_demo_scene, reconcile_scene_with_registry,
        scene_from_snapshot,
    };
    use waybroker_common::{
        CommitTarget, CommittedSceneState, FocusTarget, ServiceRole, SurfacePlacement,
        SurfaceRegistrySnapshot, SurfaceSnapshot, WaylandSelectionState, WaylandSurfaceRole,
        WaylandSurfaceState,
    };

    #[test]
    fn mock_scene_has_expected_focus_and_surfaces() {
        let scene = mock_demo_scene();
        assert_eq!(scene.target_output, "eDP-1");
        assert_eq!(scene.focus, FocusTarget::Surface { id: "konsole-1".into() });
        assert_eq!(scene.surfaces.len(), 2);
    }

    #[test]
    fn handles_empty_focus() {
        let scene = CompdScene {
            target_output: "eDP-1".into(),
            focus: FocusTarget::None,
            selection: WaylandSelectionState::default(),
            scene_epoch: 1,
            scene_generation: 1,
            surfaces: vec![],
        };
        assert_eq!(scene.focus, FocusTarget::None);
        assert_eq!(scene.surfaces.len(), 0);
    }

    #[test]
    fn surface_count_matches_after_conversion() {
        let scene = CompdScene {
            target_output: "HDMI-1".into(),
            focus: FocusTarget::None,
            selection: WaylandSelectionState::default(),
            scene_epoch: 1,
            scene_generation: 1,
            surfaces: vec![
                SurfaceSnapshot {
                    id: "s1".into(),
                    app_id: "a1".into(),
                    placement: SurfacePlacement {
                        x: 0,
                        y: 0,
                        width: 100,
                        height: 100,
                        z: 1,
                        visible: true,
                    },
                    ..Default::default()
                },
                SurfaceSnapshot {
                    id: "s2".into(),
                    app_id: "a2".into(),
                    placement: SurfacePlacement {
                        x: 0,
                        y: 0,
                        width: 100,
                        height: 100,
                        z: 2,
                        visible: true,
                    },
                    ..Default::default()
                },
            ],
        };
        assert_eq!(scene.surfaces.len(), 2);
    }

    #[test]
    fn rebuilds_scene_from_displayd_snapshot() {
        let scene = scene_from_snapshot(&CommittedSceneState {
            source: ServiceRole::Compd,
            target: CommitTarget::Output { name: "HDMI-1".into() },
            focus: FocusTarget::Surface { id: "xterm-1".into() },
            selection: waybroker_common::WaylandSelectionState::default(),
            surfaces: vec![SurfaceSnapshot {
                id: "xterm-1".into(),
                app_id: "org.xterm".into(),
                placement: SurfacePlacement {
                    x: 42,
                    y: 24,
                    width: 640,
                    height: 480,
                    z: 7,
                    visible: true,
                },
                ..Default::default()
            }],
            scene_epoch: 0,
            scene_generation: 0,
            commit_id: 3,
            unix_timestamp: 1_778_000_100,
        });

        assert_eq!(scene.target_output, "HDMI-1");
        assert_eq!(scene.focus, FocusTarget::Surface { id: "xterm-1".into() });
        assert_eq!(scene.surfaces.len(), 1);
    }

    #[test]
    fn drops_surfaces_missing_from_wayland_registry() {
        let dummy_output = waybroker_common::OutputMode {
            name: "eDP-1".into(),
            width: 1920,
            height: 1080,
            refresh_hz: 60,
        };
        let reconciled = reconcile_scene_with_registry(
            CompdScene {
                target_output: "eDP-1".into(),
                focus: FocusTarget::Surface { id: "panel-1".into() },
                selection: WaylandSelectionState {
                    clipboard_owner: Some("panel-1".into()),
                    clipboard_payload_id: Some("panel-clipboard-v1".into()),
                    clipboard_source_serial: Some(41),
                    clipboard_offer: None,
                    primary_selection_owner: Some("terminal-1".into()),
                    primary_selection_payload_id: Some("terminal-primary-v7".into()),
                    primary_selection_source_serial: Some(77),
                    primary_offer: None,
                },
                scene_epoch: 1,
                scene_generation: 1,
                surfaces: vec![
                    SurfaceSnapshot {
                        id: "terminal-1".into(),
                        app_id: "old.app".into(),
                        placement: SurfacePlacement {
                            x: 10,
                            y: 10,
                            width: 100,
                            height: 100,
                            z: 5,
                            visible: true,
                        },
                        ..Default::default()
                    },
                    SurfaceSnapshot {
                        id: "panel-1".into(),
                        app_id: "org.kde.panel".into(),
                        placement: SurfacePlacement {
                            x: 0,
                            y: 0,
                            width: 200,
                            height: 30,
                            z: 10,
                            visible: true,
                        },
                        ..Default::default()
                    },
                ],
            },
            &SurfaceRegistrySnapshot {
                generation: 2,
                surfaces: vec![WaylandSurfaceState {
                    id: "terminal-1".into(),
                    app_id: "org.kde.konsole".into(),
                    role: WaylandSurfaceRole::Toplevel,
                    mapped: true,
                    buffer_attached: true,
                    ..Default::default()
                }],
                foreign_toplevels: vec![],
                selection: WaylandSelectionState {
                    clipboard_owner: Some("panel-1".into()),
                    clipboard_payload_id: Some("panel-clipboard-v1".into()),
                    clipboard_source_serial: Some(41),
                    clipboard_offer: None,
                    primary_selection_owner: Some("terminal-1".into()),
                    primary_selection_payload_id: Some("terminal-primary-v7".into()),
                    primary_selection_source_serial: Some(77),
                    primary_offer: None,
                },
                unix_timestamp: 1,
            },
            &dummy_output,
        );

        assert_eq!(reconciled.scene.surfaces.len(), 1);
        assert_eq!(reconciled.scene.surfaces[0].id, "terminal-1");
        assert_eq!(reconciled.scene.surfaces[0].app_id, "org.kde.konsole");
        assert_eq!(reconciled.scene.focus, FocusTarget::Surface { id: "terminal-1".into() });
        assert_eq!(reconciled.scene.selection.clipboard_owner.as_deref(), Some("terminal-1"));
        assert_eq!(reconciled.scene.selection.clipboard_payload_id, None);
        assert_eq!(reconciled.scene.selection.clipboard_source_serial, None);
        assert_eq!(
            reconciled.scene.selection.primary_selection_owner.as_deref(),
            Some("terminal-1")
        );
        assert_eq!(
            reconciled.scene.selection.primary_selection_payload_id.as_deref(),
            Some("terminal-primary-v7")
        );
        assert_eq!(reconciled.scene.selection.primary_selection_source_serial, Some(77));
        assert_eq!(reconciled.dropped_surface_ids, vec!["panel-1"]);
        assert_eq!(reconciled.updated_app_ids, 1);
        assert_eq!(reconciled.selection_handoffs, 1);
    }

    #[test]
    fn falls_back_to_no_focus_when_only_background_survives() {
        let dummy_output = waybroker_common::OutputMode {
            name: "eDP-1".into(),
            width: 1920,
            height: 1080,
            refresh_hz: 60,
        };
        let reconciled = reconcile_scene_with_registry(
            CompdScene {
                target_output: "eDP-1".into(),
                focus: FocusTarget::Surface { id: "terminal-1".into() },
                selection: WaylandSelectionState {
                    clipboard_owner: Some("terminal-1".into()),
                    clipboard_payload_id: Some("terminal-clipboard-v1".into()),
                    clipboard_source_serial: Some(51),
                    clipboard_offer: None,
                    primary_selection_owner: Some("terminal-1".into()),
                    primary_selection_payload_id: Some("terminal-primary-v7".into()),
                    primary_selection_source_serial: Some(77),
                    primary_offer: None,
                },
                scene_epoch: 1,
                scene_generation: 1,
                surfaces: vec![SurfaceSnapshot {
                    id: "background-1".into(),
                    app_id: "org.kde.wallpaper".into(),
                    placement: SurfacePlacement {
                        x: 0,
                        y: 0,
                        width: 1920,
                        height: 1080,
                        z: 0,
                        visible: true,
                    },
                    ..Default::default()
                }],
            },
            &SurfaceRegistrySnapshot {
                generation: 3,
                surfaces: vec![WaylandSurfaceState {
                    id: "background-1".into(),
                    app_id: "org.kde.wallpaper".into(),
                    role: WaylandSurfaceRole::Background,
                    mapped: true,
                    buffer_attached: true,
                    ..Default::default()
                }],
                foreign_toplevels: vec![],
                selection: WaylandSelectionState {
                    clipboard_owner: Some("terminal-1".into()),
                    clipboard_payload_id: Some("terminal-clipboard-v1".into()),
                    clipboard_source_serial: Some(51),
                    clipboard_offer: None,
                    primary_selection_owner: Some("terminal-1".into()),
                    primary_selection_payload_id: Some("terminal-primary-v7".into()),
                    primary_selection_source_serial: Some(77),
                    primary_offer: None,
                },
                unix_timestamp: 1,
            },
            &dummy_output,
        );

        assert_eq!(reconciled.scene.focus, FocusTarget::None);
        assert_eq!(reconciled.scene.selection.clipboard_owner, None);
        assert_eq!(reconciled.scene.selection.clipboard_payload_id, None);
        assert_eq!(reconciled.scene.selection.clipboard_source_serial, None);
        assert_eq!(reconciled.scene.selection.primary_selection_owner, None);
        assert_eq!(reconciled.scene.selection.primary_selection_payload_id, None);
        assert_eq!(reconciled.scene.selection.primary_selection_source_serial, None);
        assert!(reconciled.dropped_surface_ids.is_empty());
        assert_eq!(reconciled.selection_handoffs, 2);
    }

    #[test]
    fn test_layer_shell_layout_logic() {
        let output = waybroker_common::OutputMode {
            name: "eDP-1".into(),
            width: 1920,
            height: 1080,
            refresh_hz: 60,
        };

        let mut bg = SurfaceSnapshot {
            id: "bg".into(),
            app_id: "bg".into(),
            placement: SurfacePlacement {
                x: 10,
                y: 10,
                width: 100,
                height: 100,
                z: 5,
                visible: true,
            },
            ..Default::default()
        };
        apply_role_based_layout(&mut bg, &WaylandSurfaceRole::Background, &output);
        assert_eq!(bg.placement.x, 0);
        assert_eq!(bg.placement.y, 0);
        assert_eq!(bg.placement.width, 1920);
        assert_eq!(bg.placement.height, 1080);
        assert_eq!(bg.placement.z, 0);

        let mut panel = SurfaceSnapshot {
            id: "panel".into(),
            app_id: "panel".into(),
            placement: SurfacePlacement { x: 0, y: 0, width: 0, height: 0, z: 0, visible: true },
            ..Default::default()
        };
        let metadata = waybroker_common::LayerMetadata {
            layer: 2,          // Top
            anchor: 1 | 2 | 4, // Left | Right | Top
            exclusive_zone: 30,
            margin_top: 5,
            margin_bottom: 0,
            margin_left: 10,
            margin_right: 10,
            keyboard_interactivity: 0,
        };
        apply_role_based_layout(&mut panel, &WaylandSurfaceRole::Layer(metadata), &output);
        assert_eq!(panel.placement.x, 10);
        assert_eq!(panel.placement.y, 5);
        assert_eq!(panel.placement.width, 1900); // 1920 - 10 - 10
        assert_eq!(panel.placement.height, 30);
        assert_eq!(panel.placement.z, 100);

        let mut overlay = SurfaceSnapshot {
            id: "overlay".into(),
            app_id: "overlay".into(),
            placement: SurfacePlacement { x: 0, y: 0, width: 0, height: 0, z: 0, visible: true },
            ..Default::default()
        };
        let overlay_metadata = waybroker_common::LayerMetadata {
            layer: 3,  // Overlay
            anchor: 8, // Bottom
            exclusive_zone: 0,
            margin_top: 0,
            margin_bottom: 20,
            margin_left: 0,
            margin_right: 0,
            keyboard_interactivity: 0,
        };
        apply_role_based_layout(
            &mut overlay,
            &WaylandSurfaceRole::Layer(overlay_metadata),
            &output,
        );
        assert_eq!(overlay.placement.y, 1080 - 36 - 20);
        assert_eq!(overlay.placement.z, 200);
    }

    #[test]
    fn test_phase2_scene_stacking_and_focus() {
        let mut scene = CompdScene {
            target_output: "eDP-1".into(),
            focus: FocusTarget::Surface { id: "win2".into() },
            selection: WaylandSelectionState::default(),
            scene_epoch: 1,
            scene_generation: 1,
            surfaces: vec![
                SurfaceSnapshot {
                    id: "win1".into(),
                    app_id: "app1".into(),
                    placement: SurfacePlacement {
                        x: 10,
                        y: 10,
                        width: 200,
                        height: 200,
                        z: 1,
                        visible: true,
                    },
                    ..Default::default()
                },
                SurfaceSnapshot {
                    id: "win2".into(),
                    app_id: "app2".into(),
                    placement: SurfacePlacement {
                        x: 50,
                        y: 50,
                        width: 200,
                        height: 200,
                        z: 2,
                        visible: true,
                    },
                    ..Default::default()
                },
            ],
        };
        scene.surfaces.sort_by_key(|s| s.placement.z);
        assert_eq!(scene.surfaces[0].id, "win1");
        assert_eq!(scene.surfaces[1].id, "win2");
        assert_eq!(scene.focus, FocusTarget::Surface { id: "win2".into() });
    }

    #[test]
    fn test_phase2_occlusion_and_visibility() {
        let win1 =
            SurfacePlacement { x: 100, y: 100, width: 100, height: 100, z: 1, visible: true };
        let win2 = SurfacePlacement { x: 50, y: 50, width: 300, height: 300, z: 2, visible: true };

        let win1_occluded = win1.x >= win2.x
            && win1.y >= win2.y
            && (win1.x + win1.width as i32) <= (win2.x + win2.width as i32)
            && (win1.y + win1.height as i32) <= (win2.y + win2.height as i32);

        assert!(win1_occluded, "win1 should be fully occluded by win2");
    }

    #[test]
    fn test_phase2_snapshot_reconcile_crash_recovery() {
        let snapshot = CommittedSceneState {
            source: ServiceRole::Compd,
            target: CommitTarget::Output { name: "eDP-1".into() },
            focus: FocusTarget::Surface { id: "gone-app".into() },
            selection: WaylandSelectionState::default(),
            surfaces: vec![
                SurfaceSnapshot {
                    id: "gone-app".into(),
                    app_id: "gone.app".into(),
                    placement: SurfacePlacement {
                        x: 0,
                        y: 0,
                        width: 100,
                        height: 100,
                        z: 1,
                        visible: true,
                    },
                    ..Default::default()
                },
                SurfaceSnapshot {
                    id: "surviving-app".into(),
                    app_id: "surviving.app".into(),
                    placement: SurfacePlacement {
                        x: 10,
                        y: 10,
                        width: 100,
                        height: 100,
                        z: 2,
                        visible: true,
                    },
                    ..Default::default()
                },
            ],
            scene_epoch: 0,
            scene_generation: 0,
            commit_id: 10,
            unix_timestamp: 1234567,
        };

        let registry = SurfaceRegistrySnapshot {
            generation: 1,
            surfaces: vec![WaylandSurfaceState {
                id: "surviving-app".into(),
                app_id: "surviving.app".into(),
                role: WaylandSurfaceRole::Toplevel,
                mapped: true,
                buffer_attached: true,
                ..Default::default()
            }],
            foreign_toplevels: vec![],
            selection: WaylandSelectionState::default(),
            unix_timestamp: 1234567,
        };

        let output_mode = waybroker_common::OutputMode {
            name: "eDP-1".into(),
            width: 1920,
            height: 1080,
            refresh_hz: 60,
        };

        let initial_scene = scene_from_snapshot(&snapshot);
        let reconciled = reconcile_scene_with_registry(initial_scene, &registry, &output_mode);

        assert_eq!(reconciled.scene.surfaces.len(), 1);
        assert_eq!(reconciled.scene.surfaces[0].id, "surviving-app");
        assert_eq!(reconciled.dropped_surface_ids, vec!["gone-app"]);
    }

    fn completion_relay_scene(generation: u64) -> super::RelayScene {
        let handle = waybroker_common::PixelTransportHandle {
            client_id: 7,
            surface_id: "surface-1".into(),
            buffer_generation: generation,
            scene_generation: generation,
        };
        super::RelayScene {
            scene: CompdScene {
                target_output: "eDP-1".into(),
                focus: FocusTarget::Surface { id: "surface-1".into() },
                selection: WaylandSelectionState::default(),
                surfaces: vec![SurfaceSnapshot {
                    id: "surface-1".into(),
                    app_id: "completion.app".into(),
                    placement: SurfacePlacement {
                        x: 0,
                        y: 0,
                        width: 2,
                        height: 2,
                        z: 1,
                        visible: true,
                    },
                    buffer_handle: Some(format!("buffer-{generation}")),
                    buffer_generation: generation,
                    pixel_transport: Some(handle.clone()),
                    creation_sequence: generation,
                    ..Default::default()
                }],
                scene_epoch: 1,
                scene_generation: generation,
            },
            pixel_payloads: vec![waybroker_common::PixelTransportPayload {
                handle,
                pixels: vec![0x55; 16],
                width: 2,
                height: 2,
                stride: 8,
                format: 0,
            }],
        }
    }

    #[test]
    fn completion_compd_scene_budget_accepts_valid_scene() {
        let relay = completion_relay_scene(4);
        let usage = super::validate_relay_scene(&relay, true).unwrap();
        assert_eq!(usage.surfaces, 1);
        assert_eq!(usage.pixel_payloads, 1);
        assert_eq!(usage.pixel_bytes, 16);
    }

    #[test]
    fn completion_compd_scene_budget_rejects_duplicate_surface_identity() {
        let mut relay = completion_relay_scene(4);
        relay.scene.surfaces.push(relay.scene.surfaces[0].clone());
        let error = super::validate_relay_scene(&relay, true).unwrap_err();
        assert!(error.to_string().contains("duplicate surface identity"));
    }

    #[test]
    fn completion_compd_scene_budget_rejects_orphan_payload() {
        let mut relay = completion_relay_scene(4);
        relay.pixel_payloads[0].handle.surface_id = "orphan".into();
        let error = super::validate_relay_scene(&relay, true).unwrap_err();
        assert!(error.to_string().contains("orphan PixelTransport"));
    }

    #[test]
    fn completion_compd_scene_budget_rejects_payload_generation_mismatch() {
        let mut relay = completion_relay_scene(4);
        relay.pixel_payloads[0].handle.scene_generation = 3;
        let error = super::validate_relay_scene(&relay, true).unwrap_err();
        assert!(
            error.to_string().contains("payload does not match canonical surface handle")
                || error.to_string().contains("payload generation does not match scene generation")
        );
    }

    #[test]
    fn completion_compd_scene_budget_rejects_payload_length_mismatch() {
        let mut relay = completion_relay_scene(4);
        relay.pixel_payloads[0].pixels.pop();
        let error = super::validate_relay_scene(&relay, true).unwrap_err();
        assert!(error.to_string().contains("byte length mismatch"));
    }

    #[test]
    fn completion_compd_scene_rejects_missing_focus_surface() {
        let mut relay = completion_relay_scene(4);
        relay.scene.focus = FocusTarget::Surface { id: "missing".into() };
        let error = super::validate_relay_scene(&relay, true).unwrap_err();
        assert!(error.to_string().contains("focused surface missing"));
    }

    #[test]
    fn completion_compd_scene_rejects_missing_selection_owner() {
        let mut relay = completion_relay_scene(4);
        relay.scene.selection.clipboard_owner = Some("missing".into());
        let error = super::validate_relay_scene(&relay, true).unwrap_err();
        assert!(error.to_string().contains("selection owner missing"));
    }

    #[test]
    fn completion_compd_canonical_order_is_deterministic() {
        let mut relay = completion_relay_scene(4);
        relay.scene.focus = FocusTarget::None;
        relay.scene.surfaces.clear();
        relay.pixel_payloads.clear();

        let make_surface = |id: &str, layer: u32, z: i32, creation: u64| SurfaceSnapshot {
            id: id.into(),
            app_id: "completion.app".into(),
            placement: SurfacePlacement {
                width: 1,
                height: 1,
                z,
                visible: true,
                ..Default::default()
            },
            layer_class: layer,
            creation_sequence: creation,
            ..Default::default()
        };
        relay.scene.surfaces = vec![
            make_surface("c", 1, 5, 2),
            make_surface("a", 0, 99, 1),
            make_surface("b", 1, 5, 1),
        ];

        super::canonicalize_relay_scene(&mut relay);
        let ids =
            relay.scene.surfaces.iter().map(|surface| surface.id.as_str()).collect::<Vec<_>>();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    #[test]
    fn completion_compd_pending_scene_is_latest_state_only() {
        let mut state = super::CompdRelayState::default();
        state.defer_latest(&completion_relay_scene(10), "first".into());
        state.defer_latest(&completion_relay_scene(12), "newest".into());
        state.defer_latest(&completion_relay_scene(11), "stale".into());

        let pending = state.pending.as_ref().unwrap();
        assert_eq!(pending.version, (1, 12));
        assert_eq!(pending.reason, "newest");
        assert_eq!(pending.usage.pixel_bytes, 16);
        assert_eq!(state.superseded_count, 1);
    }

    #[test]
    fn completion_compd_version_floor_rejects_replayed_forwarded_scene() {
        let mut state = super::CompdRelayState::default();
        let forwarded = completion_relay_scene(10);
        state.note_forwarded(&forwarded);

        assert!(state.should_reject_version((1, 9)));
        assert!(state.should_reject_version((1, 10)));
        assert!(!state.should_reject_version((1, 11)));
        assert!(state.should_reject_version((0, 0)));
    }

    #[test]
    fn completion_compd_equal_pending_generation_remains_retryable() {
        let mut state = super::CompdRelayState::default();
        state.defer_latest(&completion_relay_scene(12), "displayd unavailable".into());

        assert!(!state.should_reject_version((1, 12)));
        assert!(state.should_reject_version((1, 11)));
        assert!(!state.should_reject_version((1, 13)));
    }

    #[test]
    fn completion_compd_scene_words_are_deterministic_and_complete() {
        let relay = completion_relay_scene(4);
        let words = super::scene_surface_words(&relay.scene.surfaces);
        assert_eq!(words.len(), 8);
        assert_eq!(words[2], 2);
        assert_eq!(words[3], 2);
        assert_eq!(words[5], 1);
        assert_eq!(words[6], 4);
    }

    #[test]
    fn completion_compd_metadata_only_recovery_is_explicitly_supported() {
        let scene = scene_from_snapshot(&CommittedSceneState {
            source: ServiceRole::Compd,
            target: CommitTarget::Output { name: "eDP-1".into() },
            focus: FocusTarget::None,
            selection: WaylandSelectionState::default(),
            surfaces: vec![SurfaceSnapshot {
                id: "metadata-only".into(),
                app_id: "completion.app".into(),
                placement: SurfacePlacement {
                    width: 1,
                    height: 1,
                    visible: true,
                    ..Default::default()
                },
                ..Default::default()
            }],
            scene_epoch: 2,
            scene_generation: 7,
            commit_id: 9,
            unix_timestamp: 1,
        });
        let relay = super::RelayScene { scene, pixel_payloads: Vec::new() };
        super::validate_relay_scene(&relay, false).unwrap();
    }

    #[test]
    fn completion_compd_commit_command_preserves_scene_identity_and_payloads() {
        let relay = completion_relay_scene(17);
        match relay.to_commit_command() {
            waybroker_common::DisplayCommand::CommitScene {
                scene_epoch,
                scene_generation,
                surfaces,
                pixel_payloads,
                ..
            } => {
                assert_eq!(scene_epoch, 1);
                assert_eq!(scene_generation, 17);
                assert_eq!(surfaces.len(), 1);
                assert_eq!(pixel_payloads.len(), 1);
                assert_eq!(pixel_payloads[0].handle.scene_generation, 17);
            }
            other => panic!("expected CommitScene, got {other:?}"),
        }
    }

    #[test]
    fn completion_compd_surface_budget_is_bounded() {
        let mut relay = completion_relay_scene(4);
        relay.scene.focus = FocusTarget::None;
        relay.pixel_payloads.clear();
        relay.scene.surfaces = (0..=super::MAX_RELAY_SURFACES)
            .map(|index| SurfaceSnapshot {
                id: format!("surface-{index}"),
                app_id: "completion.app".into(),
                placement: SurfacePlacement {
                    width: 1,
                    height: 1,
                    visible: true,
                    ..Default::default()
                },
                ..Default::default()
            })
            .collect();
        let error = super::validate_relay_scene(&relay, false).unwrap_err();
        assert!(error.to_string().contains("surface budget exceeded"));
    }

    #[test]
    fn completion_compd_registry_reconcile_clears_stale_pixel_transport_handle() {
        let handle = waybroker_common::PixelTransportHandle {
            client_id: 1,
            surface_id: "surface-1".into(),
            buffer_generation: 1,
            scene_generation: 1,
        };
        let scene = CompdScene {
            target_output: "eDP-1".into(),
            focus: FocusTarget::Surface { id: "surface-1".into() },
            selection: WaylandSelectionState::default(),
            surfaces: vec![SurfaceSnapshot {
                id: "surface-1".into(),
                app_id: "completion.app".into(),
                placement: SurfacePlacement {
                    width: 10,
                    height: 10,
                    visible: true,
                    ..Default::default()
                },
                buffer_generation: 1,
                pixel_transport: Some(handle),
                ..Default::default()
            }],
            scene_epoch: 1,
            scene_generation: 1,
        };
        let registry = SurfaceRegistrySnapshot {
            generation: 2,
            surfaces: vec![WaylandSurfaceState {
                id: "surface-1".into(),
                app_id: "completion.app".into(),
                role: WaylandSurfaceRole::Toplevel,
                mapped: true,
                buffer_attached: true,
                buffer_handle: Some("buffer-2".into()),
                buffer_generation: 2,
                ..Default::default()
            }],
            foreign_toplevels: vec![],
            selection: WaylandSelectionState::default(),
            unix_timestamp: 1,
        };
        let output = waybroker_common::OutputMode {
            name: "eDP-1".into(),
            width: 1920,
            height: 1080,
            refresh_hz: 60,
        };

        let reconciled = reconcile_scene_with_registry(scene, &registry, &output);
        assert_eq!(reconciled.scene.surfaces.len(), 1);
        assert_eq!(reconciled.scene.surfaces[0].buffer_generation, 2);
        assert!(reconciled.scene.surfaces[0].pixel_transport.is_none());
    }

    #[test]
    fn completion_compd_reconciliation_uses_actual_displayd_commit_id() {
        let relay = completion_relay_scene(21);
        let snapshot = CommittedSceneState {
            source: ServiceRole::Compd,
            target: CommitTarget::Output { name: "eDP-1".into() },
            focus: relay.scene.focus.clone(),
            selection: relay.scene.selection.clone(),
            surfaces: relay.scene.surfaces.clone(),
            scene_epoch: 1,
            scene_generation: 21,
            commit_id: 77,
            unix_timestamp: 1,
        };
        assert_eq!(super::reconciliation_commit_id(Some(snapshot), &relay).unwrap(), 77);
    }

    #[test]
    fn completion_compd_reconciliation_rejects_snapshot_version_mismatch() {
        let relay = completion_relay_scene(21);
        let snapshot = CommittedSceneState {
            source: ServiceRole::Compd,
            target: CommitTarget::Output { name: "eDP-1".into() },
            focus: relay.scene.focus.clone(),
            selection: relay.scene.selection.clone(),
            surfaces: relay.scene.surfaces.clone(),
            scene_epoch: 1,
            scene_generation: 20,
            commit_id: 77,
            unix_timestamp: 1,
        };
        assert!(super::reconciliation_commit_id(Some(snapshot), &relay).is_err());
    }

    #[test]
    fn completion_compd_pending_marker_does_not_retain_pixel_payload_buffers() {
        assert!(std::mem::size_of::<super::PendingRelayScene>() <= 128);
        let mut state = super::CompdRelayState::default();
        let relay = completion_relay_scene(30);
        state.defer_latest(&relay, "displayd unavailable".into());
        let pending = state.pending.as_ref().unwrap();
        assert_eq!(pending.version, (1, 30));
        assert_eq!(pending.usage.pixel_bytes, 16);
    }

    #[test]
    fn completion_compd_scene_budget_rejects_buffer_generation_mismatch() {
        let mut relay = completion_relay_scene(4);
        relay.scene.surfaces[0].buffer_generation = 5;
        let error = super::validate_relay_scene(&relay, true).unwrap_err();
        assert!(error.to_string().contains("buffer generation mismatch"));
    }

    #[test]
    fn completion_compd_scene_budget_rejects_coordinate_overflow() {
        let mut relay = completion_relay_scene(4);
        relay.scene.surfaces[0].placement.x = i32::MAX;
        relay.scene.surfaces[0].placement.width = 2;
        let error = super::validate_relay_scene(&relay, true).unwrap_err();
        assert!(error.to_string().contains("placement overflows"));
    }

    #[test]
    fn completion_compd_10k_scene_storm_retains_only_latest_pending_marker() {
        let mut state = super::CompdRelayState::default();
        for generation in 1..=10_000 {
            let relay = completion_relay_scene(generation);
            state.defer_latest(&relay, format!("deferred-{generation}"));
        }

        let pending = state.pending.as_ref().expect("latest pending scene");
        assert_eq!(pending.version, (1, 10_000));
        assert_eq!(pending.reason, "deferred-10000");
        assert_eq!(pending.usage.pixel_bytes, 16);
        assert_eq!(state.deferred_count, 10_000);
        assert_eq!(state.superseded_count, 9_999);
        assert!(std::mem::size_of::<super::PendingRelayScene>() <= 128);
    }
}
