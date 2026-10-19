use std::{
    env, fs,
    fs::OpenOptions,
    io::{BufReader, ErrorKind, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use waybroker_common::{
    IpcEnvelope, LockCommand, LockState, MessageKind, ServiceBanner, ServiceEndpoint, ServiceRole,
    ServiceStream, bind_service_socket, ensure_runtime_dir, is_recoverable_accept_error,
    now_unix_timestamp, read_json_line, send_json_line, session_artifact_path,
    validate_session_instance_id,
};

const LOCK_STATE_ARTIFACT: &str = "lockd-state";
const LOCK_STATE_ARTIFACT_VERSION: u32 = 1;
const CLIENT_IO_TIMEOUT: Duration = Duration::from_secs(2);
const LEGACY_SESSION_INSTANCE_ID: &str = "legacy-single-session";

fn main() -> Result<()> {
    let config = Config::from_args(env::args().skip(1))?;
    let banner = ServiceBanner::new(ServiceRole::Lockd, "lockscreen and auth ui");
    println!("{}", banner.render());

    if config.serve_ipc {
        serve_ipc(&config)?;
    } else {
        println!(
            "service=lockd state=idle session_instance_id={} (use --serve-ipc to start lock service)",
            config.session_instance_id
        );
    }

    Ok(())
}

#[derive(Debug)]
struct Config {
    serve_ipc: bool,
    serve_once: bool,
    fail_resume: bool,
    session_instance_id: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            serve_ipc: false,
            serve_once: false,
            fail_resume: false,
            session_instance_id: LEGACY_SESSION_INSTANCE_ID.to_string(),
        }
    }
}

impl Config {
    fn from_args(mut args: impl Iterator<Item = String>) -> Result<Self> {
        let mut config = Self::default();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--serve-ipc" => config.serve_ipc = true,
                "--once" => config.serve_once = true,
                "--fail-resume" => config.fail_resume = true,
                "--session-instance-id" => {
                    let id = args.next().context("--session-instance-id requires an id")?;
                    if !validate_session_instance_id(&id) {
                        bail!(
                            "invalid session-instance-id: \"{}\". Must be 1-128 chars of [A-Za-z0-9._-]",
                            id
                        );
                    }
                    config.session_instance_id = id;
                }
                "--help" | "-h" => {
                    println!(
                        "usage: lockd [--serve-ipc] [--once] [--fail-resume] [--session-instance-id ID]"
                    );
                    std::process::exit(0);
                }
                _ => bail!("unknown argument: {arg}"),
            }
        }

        Ok(config)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct PersistedLockState {
    version: u32,
    session_instance_id: String,
    state: LockState,
    generation: u64,
    unix_timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LockRuntimeState {
    persisted: PersistedLockState,
}

impl LockRuntimeState {
    fn fresh(session_instance_id: &str, state: LockState) -> Self {
        Self {
            persisted: PersistedLockState {
                version: LOCK_STATE_ARTIFACT_VERSION,
                session_instance_id: session_instance_id.to_string(),
                state,
                generation: 1,
                unix_timestamp: now_unix_timestamp(),
            },
        }
    }

    fn state(&self) -> LockState {
        self.persisted.state
    }

    fn generation(&self) -> u64 {
        self.persisted.generation
    }

    fn session_instance_id(&self) -> &str {
        &self.persisted.session_instance_id
    }

    fn state_response(&self) -> LockCommand {
        LockCommand::State {
            state: self.state(),
            generation: self.generation(),
            session_instance_id: self.session_instance_id().to_string(),
        }
    }

    fn transition(&mut self, state: LockState, state_path: &Path) -> Result<bool> {
        if self.persisted.state == state {
            return Ok(false);
        }

        let generation = self
            .persisted
            .generation
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("lock state generation exhausted"))?;
        let next = PersistedLockState {
            version: LOCK_STATE_ARTIFACT_VERSION,
            session_instance_id: self.persisted.session_instance_id.clone(),
            state,
            generation,
            unix_timestamp: now_unix_timestamp(),
        };

        persist_lock_state(state_path, &next)?;
        self.persisted = next;
        Ok(true)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StartupState {
    Fresh,
    Recovered,
    FailClosed(String),
}

fn lock_state_path(session_instance_id: &str) -> PathBuf {
    session_artifact_path(session_instance_id, LOCK_STATE_ARTIFACT)
}

fn load_or_initialize_lock_state(
    session_instance_id: &str,
    state_path: &Path,
) -> Result<(LockRuntimeState, StartupState)> {
    let raw = match fs::read_to_string(state_path) {
        Ok(raw) => Some(raw),
        Err(err) if err.kind() == ErrorKind::NotFound => None,
        Err(err) => {
            return Err(err)
                .with_context(|| format!("failed to read lock state {}", state_path.display()));
        }
    };

    if let Some(raw) = raw {
        match serde_json::from_str::<PersistedLockState>(&raw) {
            Ok(snapshot)
                if snapshot.version == LOCK_STATE_ARTIFACT_VERSION
                    && snapshot.session_instance_id == session_instance_id
                    && snapshot.generation > 0 =>
            {
                return Ok((LockRuntimeState { persisted: snapshot }, StartupState::Recovered));
            }
            Ok(snapshot) => {
                let reason = format!(
                    "invalid persisted lock state metadata version={} session_instance_id={} generation={}",
                    snapshot.version, snapshot.session_instance_id, snapshot.generation
                );
                let runtime = LockRuntimeState::fresh(session_instance_id, LockState::BlankOnly);
                persist_lock_state(state_path, &runtime.persisted)?;
                return Ok((runtime, StartupState::FailClosed(reason)));
            }
            Err(err) => {
                let reason = format!("failed to decode persisted lock state: {err}");
                let runtime = LockRuntimeState::fresh(session_instance_id, LockState::BlankOnly);
                persist_lock_state(state_path, &runtime.persisted)?;
                return Ok((runtime, StartupState::FailClosed(reason)));
            }
        }
    }

    let runtime = LockRuntimeState::fresh(session_instance_id, LockState::Unlocked);
    persist_lock_state(state_path, &runtime.persisted)?;
    Ok((runtime, StartupState::Fresh))
}

fn persist_lock_state(path: &Path, snapshot: &PersistedLockState) -> Result<()> {
    let parent = path.parent().context("lock state artifact has no parent directory")?;
    if !parent.exists() {
        bail!("lock state artifact parent does not exist: {}", parent.display());
    }

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("lock state artifact filename is not valid UTF-8")?;
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).expect("time").as_nanos();
    let temp_path = path.with_file_name(format!(".{file_name}.tmp-{}-{nonce}", std::process::id()));

    let encoded =
        serde_json::to_vec_pretty(snapshot).context("failed to serialize persistent lock state")?;

    let write_result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
            .with_context(|| format!("failed to create {}", temp_path.display()))?;
        file.write_all(&encoded)
            .with_context(|| format!("failed to write {}", temp_path.display()))?;
        file.write_all(b"\n")
            .with_context(|| format!("failed to terminate {}", temp_path.display()))?;
        file.sync_all().with_context(|| format!("failed to sync {}", temp_path.display()))?;
        fs::rename(&temp_path, path).with_context(|| {
            format!("failed to atomically replace lock state {}", path.display())
        })?;
        Ok(())
    })();

    if write_result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    write_result
}

fn serve_ipc(config: &Config) -> Result<()> {
    let _ = ensure_runtime_dir()?;
    let listener = bind_service_socket(ServiceRole::Lockd)?;
    let _socket_guard = SocketGuard::new(listener.endpoint().clone());
    println!("service=lockd op=listen event=socket_bound path={}", listener.endpoint());

    let state_path = lock_state_path(&config.session_instance_id);
    let (mut runtime, startup_state) =
        load_or_initialize_lock_state(&config.session_instance_id, &state_path)?;

    match &startup_state {
        StartupState::Fresh => println!(
            "service=lockd op=state_restore event=fresh session_instance_id={} state={:?} generation={} path={}",
            runtime.session_instance_id(),
            runtime.state(),
            runtime.generation(),
            state_path.display()
        ),
        StartupState::Recovered => println!(
            "service=lockd op=state_restore event=recovered session_instance_id={} state={:?} generation={} path={}",
            runtime.session_instance_id(),
            runtime.state(),
            runtime.generation(),
            state_path.display()
        ),
        StartupState::FailClosed(reason) => println!(
            "service=lockd op=state_restore event=fail_closed session_instance_id={} state={:?} generation={} reason=\"{}\" path={}",
            runtime.session_instance_id(),
            runtime.state(),
            runtime.generation(),
            reason,
            state_path.display()
        ),
    }

    println!(
        "service=lockd op=status event=ready current_state={:?} generation={} session_instance_id={}",
        runtime.state(),
        runtime.generation(),
        runtime.session_instance_id()
    );

    let mut served = 0usize;
    loop {
        match listener.accept() {
            Ok((stream, _peer)) => {
                served = served.saturating_add(1);
                if let Err(err) = handle_client(stream, &mut runtime, config, &state_path) {
                    println!(
                        "service=lockd op=client event=failed served_request={} reason=\"{:#}\"",
                        served, err
                    );
                }

                if config.serve_once {
                    break;
                }
            }
            Err(err) if is_recoverable_accept_error(&err) => {
                println!("service=lockd op=accept event=recoverable_error reason=\"{err}\"");
            }
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("failed to accept on {}", listener.endpoint()));
            }
        }
    }

    println!("service=lockd op=terminate event=finished served_requests={served}");
    Ok(())
}

fn handle_client(
    mut stream: ServiceStream,
    runtime: &mut LockRuntimeState,
    config: &Config,
    state_path: &Path,
) -> Result<()> {
    stream
        .set_read_timeout(Some(CLIENT_IO_TIMEOUT))
        .context("failed to set lockd client read timeout")?;
    stream
        .set_write_timeout(Some(CLIENT_IO_TIMEOUT))
        .context("failed to set lockd client write timeout")?;

    let request: IpcEnvelope = {
        let mut reader = BufReader::new(stream.try_clone()?);
        read_json_line(&mut reader)?
    };

    let response = build_response(request, runtime, config, state_path);
    send_json_line(&mut stream, &response)?;
    Ok(())
}

fn is_control_source(source: ServiceRole) -> bool {
    source == ServiceRole::Sessiond
}

fn is_status_source(source: ServiceRole) -> bool {
    matches!(source, ServiceRole::Sessiond | ServiceRole::Watchdog)
}

fn reject(source: ServiceRole, reason: impl Into<String>) -> IpcEnvelope {
    IpcEnvelope::new(
        ServiceRole::Lockd,
        source,
        MessageKind::LockCommand(LockCommand::Rejected { reason: reason.into() }),
    )
}

fn build_response(
    request: IpcEnvelope,
    runtime: &mut LockRuntimeState,
    config: &Config,
    state_path: &Path,
) -> IpcEnvelope {
    let source = request.source;

    if request.destination != ServiceRole::Lockd {
        return reject(
            source,
            format!("lockd received message addressed to {}", request.destination.as_str()),
        );
    }

    let response_kind = match request.kind {
        MessageKind::LockCommand(LockCommand::SetLockState { state }) => {
            if !is_control_source(source) {
                println!(
                    "service=lockd op=set_lock_state event=rejected source={} reason=\"unauthorized source\"",
                    source.as_str()
                );
                LockCommand::Rejected {
                    reason: format!("{} is not authorized to change lock state", source.as_str()),
                }
            } else if config.fail_resume {
                println!("service=lockd op=set_lock_state event=failed reason=\"fault injection\"");
                LockCommand::Rejected { reason: "fault injection".into() }
            } else {
                let old_state = runtime.state();
                match runtime.transition(state, state_path) {
                    Ok(changed) => {
                        println!(
                            "service=lockd op=state_transition event={} from={:?} to={:?} generation={} session_instance_id={}",
                            if changed { "success" } else { "unchanged" },
                            old_state,
                            state,
                            runtime.generation(),
                            runtime.session_instance_id()
                        );
                        LockCommand::SetLockState { state }
                    }
                    Err(err) => {
                        println!(
                            "service=lockd op=state_transition event=failed from={:?} to={:?} reason=\"{:#}\"",
                            old_state, state, err
                        );
                        LockCommand::Rejected {
                            reason: format!("failed to persist lock state: {err:#}"),
                        }
                    }
                }
            }
        }
        MessageKind::LockCommand(LockCommand::AuthPrompt { reason }) => {
            if !is_control_source(source) {
                println!(
                    "service=lockd op=auth_prompt event=rejected source={} reason=\"unauthorized source\"",
                    source.as_str()
                );
                LockCommand::Rejected {
                    reason: format!("{} is not authorized to request auth UI", source.as_str()),
                }
            } else if config.fail_resume {
                println!(
                    "service=lockd op=auth_prompt event=failed reason=\"fault injection\" prompt_reason=\"{}\"",
                    reason
                );
                LockCommand::Rejected { reason: "fault injection".into() }
            } else if runtime.state() == LockState::Unlocked {
                println!(
                    "service=lockd op=auth_prompt event=rejected reason=\"lock state is unlocked\" prompt_reason=\"{}\"",
                    reason
                );
                LockCommand::Rejected {
                    reason: "auth prompt requires locked or blank-only state".into(),
                }
            } else {
                println!(
                    "service=lockd op=auth_prompt event=success state={:?} generation={} reason=\"{}\"",
                    runtime.state(),
                    runtime.generation(),
                    reason
                );
                LockCommand::AuthPrompt { reason }
            }
        }
        MessageKind::LockCommand(LockCommand::GetState) => {
            if !is_status_source(source) {
                println!(
                    "service=lockd op=get_state event=rejected source={} reason=\"unauthorized source\"",
                    source.as_str()
                );
                LockCommand::Rejected {
                    reason: format!("{} is not authorized to query lock state", source.as_str()),
                }
            } else {
                println!(
                    "service=lockd op=get_state event=success state={:?} generation={} session_instance_id={}",
                    runtime.state(),
                    runtime.generation(),
                    runtime.session_instance_id()
                );
                runtime.state_response()
            }
        }
        MessageKind::LockCommand(LockCommand::State { .. })
        | MessageKind::LockCommand(LockCommand::Rejected { .. }) => LockCommand::Rejected {
            reason: "lockd does not accept response-only lock commands".into(),
        },
        other => {
            return reject(source, format!("lockd does not handle {other:?}"));
        }
    };

    IpcEnvelope::new(ServiceRole::Lockd, source, MessageKind::LockCommand(response_kind))
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

#[cfg(test)]
mod tests {
    use super::{
        Config, LockRuntimeState, StartupState, build_response, load_or_initialize_lock_state,
        persist_lock_state,
    };
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };
    use waybroker_common::{IpcEnvelope, LockCommand, LockState, MessageKind, ServiceRole};

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lockd-test-{label}-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).expect("time").as_nanos()
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn request(source: ServiceRole, command: LockCommand) -> IpcEnvelope {
        IpcEnvelope::new(source, ServiceRole::Lockd, MessageKind::LockCommand(command))
    }

    fn response_command(response: IpcEnvelope) -> LockCommand {
        match response.kind {
            MessageKind::LockCommand(command) => command,
            other => panic!("expected lock command response, got {other:?}"),
        }
    }

    #[test]
    fn parses_and_validates_session_instance_id() {
        let config = Config::from_args(
            ["--serve-ipc", "--session-instance-id", "session-a"].into_iter().map(String::from),
        )
        .expect("parse config");
        assert!(config.serve_ipc);
        assert_eq!(config.session_instance_id, "session-a");

        let invalid =
            Config::from_args(["--session-instance-id", "../escape"].into_iter().map(String::from));
        assert!(invalid.is_err());
    }

    #[test]
    fn transition_persists_before_runtime_state_changes() {
        let dir = temp_dir("transition");
        let path = dir.join("state.json");
        let mut runtime = LockRuntimeState::fresh("session-a", LockState::Unlocked);
        persist_lock_state(&path, &runtime.persisted).expect("persist initial");

        assert!(runtime.transition(LockState::Locked, &path).expect("transition"));
        assert_eq!(runtime.state(), LockState::Locked);
        assert_eq!(runtime.generation(), 2);

        let raw = fs::read_to_string(&path).expect("read persisted state");
        let persisted: super::PersistedLockState =
            serde_json::from_str(&raw).expect("decode state");
        assert_eq!(persisted.state, LockState::Locked);
        assert_eq!(persisted.generation, 2);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn failed_persistence_does_not_mutate_runtime_state() {
        let dir = temp_dir("transaction");
        let path = dir.join("state-as-directory");
        fs::create_dir(&path).expect("create blocking directory");

        let mut runtime = LockRuntimeState::fresh("session-a", LockState::Unlocked);
        assert!(runtime.transition(LockState::Locked, &path).is_err());
        assert_eq!(runtime.state(), LockState::Unlocked);
        assert_eq!(runtime.generation(), 1);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn restart_recovers_persisted_lock_state() {
        let dir = temp_dir("recover");
        let path = dir.join("state.json");
        let mut runtime = LockRuntimeState::fresh("session-a", LockState::Unlocked);
        persist_lock_state(&path, &runtime.persisted).expect("persist initial");
        runtime.transition(LockState::Locked, &path).expect("persist locked");

        let (reloaded, startup) =
            load_or_initialize_lock_state("session-a", &path).expect("reload state");
        assert_eq!(startup, StartupState::Recovered);
        assert_eq!(reloaded.state(), LockState::Locked);
        assert_eq!(reloaded.generation(), 2);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn malformed_persisted_state_fails_closed() {
        let dir = temp_dir("corrupt");
        let path = dir.join("state.json");
        fs::write(&path, "{not-json").expect("write malformed state");

        let (runtime, startup) =
            load_or_initialize_lock_state("session-a", &path).expect("fail closed");
        assert!(matches!(startup, StartupState::FailClosed(_)));
        assert_eq!(runtime.state(), LockState::BlankOnly);
        assert_eq!(runtime.generation(), 1);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn persisted_state_for_other_session_fails_closed() {
        let dir = temp_dir("session-mismatch");
        let path = dir.join("state.json");
        let other = LockRuntimeState::fresh("session-b", LockState::Locked);
        persist_lock_state(&path, &other.persisted).expect("persist other session");

        let (runtime, startup) =
            load_or_initialize_lock_state("session-a", &path).expect("fail closed");
        assert!(matches!(startup, StartupState::FailClosed(_)));
        assert_eq!(runtime.state(), LockState::BlankOnly);
        assert_eq!(runtime.session_instance_id(), "session-a");

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn unauthorized_source_cannot_change_state() {
        let dir = temp_dir("unauthorized");
        let path = dir.join("state.json");
        let mut runtime = LockRuntimeState::fresh("session-a", LockState::Unlocked);
        persist_lock_state(&path, &runtime.persisted).expect("persist initial");

        let response = build_response(
            request(ServiceRole::Compd, LockCommand::SetLockState { state: LockState::Locked }),
            &mut runtime,
            &Config::default(),
            &path,
        );

        assert!(matches!(response_command(response), LockCommand::Rejected { .. }));
        assert_eq!(runtime.state(), LockState::Unlocked);
        assert_eq!(runtime.generation(), 1);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn wrong_destination_is_rejected_without_mutation() {
        let dir = temp_dir("destination");
        let path = dir.join("state.json");
        let mut runtime = LockRuntimeState::fresh("session-a", LockState::Unlocked);
        persist_lock_state(&path, &runtime.persisted).expect("persist initial");

        let mut envelope =
            request(ServiceRole::Sessiond, LockCommand::SetLockState { state: LockState::Locked });
        envelope.destination = ServiceRole::Compd;
        let response = build_response(envelope, &mut runtime, &Config::default(), &path);

        assert!(matches!(response_command(response), LockCommand::Rejected { .. }));
        assert_eq!(runtime.state(), LockState::Unlocked);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn fail_resume_rejects_without_mutating_state() {
        let dir = temp_dir("fail-resume");
        let path = dir.join("state.json");
        let mut runtime = LockRuntimeState::fresh("session-a", LockState::Unlocked);
        persist_lock_state(&path, &runtime.persisted).expect("persist initial");
        let config = Config { fail_resume: true, ..Config::default() };

        let response = build_response(
            request(ServiceRole::Sessiond, LockCommand::SetLockState { state: LockState::Locked }),
            &mut runtime,
            &config,
            &path,
        );

        assert_eq!(
            response_command(response),
            LockCommand::Rejected { reason: "fault injection".into() }
        );
        assert_eq!(runtime.state(), LockState::Unlocked);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn auth_prompt_requires_locked_or_blank_only_state() {
        let dir = temp_dir("auth");
        let path = dir.join("state.json");
        let mut runtime = LockRuntimeState::fresh("session-a", LockState::Unlocked);
        persist_lock_state(&path, &runtime.persisted).expect("persist initial");

        let rejected = build_response(
            request(ServiceRole::Sessiond, LockCommand::AuthPrompt { reason: "resume".into() }),
            &mut runtime,
            &Config::default(),
            &path,
        );
        assert!(matches!(response_command(rejected), LockCommand::Rejected { .. }));

        runtime.transition(LockState::Locked, &path).expect("lock");
        let accepted = build_response(
            request(ServiceRole::Sessiond, LockCommand::AuthPrompt { reason: "resume".into() }),
            &mut runtime,
            &Config::default(),
            &path,
        );
        assert_eq!(response_command(accepted), LockCommand::AuthPrompt { reason: "resume".into() });

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn get_state_reports_generation_and_session_identity() {
        let dir = temp_dir("status");
        let path = dir.join("state.json");
        let mut runtime = LockRuntimeState::fresh("session-a", LockState::Locked);
        persist_lock_state(&path, &runtime.persisted).expect("persist initial");

        let response = build_response(
            request(ServiceRole::Watchdog, LockCommand::GetState),
            &mut runtime,
            &Config::default(),
            &path,
        );

        assert_eq!(
            response_command(response),
            LockCommand::State {
                state: LockState::Locked,
                generation: 1,
                session_instance_id: "session-a".into(),
            }
        );

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn idempotent_state_transition_does_not_advance_generation() {
        let dir = temp_dir("idempotent");
        let path = dir.join("state.json");
        let mut runtime = LockRuntimeState::fresh("session-a", LockState::Locked);
        persist_lock_state(&path, &runtime.persisted).expect("persist initial");

        assert!(!runtime.transition(LockState::Locked, &path).expect("no-op"));
        assert_eq!(runtime.generation(), 1);

        let _ = fs::remove_dir_all(dir);
    }
}
