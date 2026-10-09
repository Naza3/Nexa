use crate::{
    client::VerifiedConnection,
    instance::{Discovery, InstanceLock, wait_stopped},
};
use hyper::Method;
use model_store::{ImportCancellation, ImportRequest, ModelSource, ModelStore};
use process_host::{ProcessHost, ProcessHostConfig};
use runtime_api::{
    ApiState, Config,
    dto::ModelSummary,
    lan::LanSecurityContext,
    security::SecurityContext,
    token::{init_private_lan_token, load_private_token},
};
use runtime_core::Runtime;
use runtime_types::ModelId;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use uuid::Uuid;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
pub const HELP: &str = "Nexa local runtime\n\
Usage: ai-runtime [--data-dir PATH] COMMAND\n\
  init\n  serve\n  models import --id ID --file PATH\n  models list\n\
  load ID [--backend cpu] [--context N] [--gpu-layers 0] [--threads N] [--batch N]\n\
  unload\n  status\n  devices\n  cancel UUID\n  stop\n  version --json\n\
Exit codes: success/not-running stop=0; operation/cleanup failure=1; invalid arguments=2.";
#[derive(Debug)]
pub struct Options {
    pub data_dir: PathBuf,
    pub command: Command,
}
#[derive(Debug)]
pub enum Command {
    Init,
    Serve,
    Import { id: ModelId, file: PathBuf },
    List,
    Load(Value),
    Unload,
    Status,
    Devices,
    Cancel(Uuid),
    Stop,
    Version,
}
pub fn parse(mut args: Vec<OsString>) -> Result<Options> {
    let data_dir = if args.first().is_some_and(|x| x == "--data-dir") {
        if args.len() < 3 {
            return Err("--data-dir requires a path and command".into());
        }
        let value = PathBuf::from(args.remove(1));
        args.remove(0);
        if value.as_os_str().is_empty() {
            return Err("data directory cannot be empty".into());
        }
        value
    } else {
        default_data_dir()?
    };
    let Some(name) = args.first().and_then(|value| value.to_str()) else {
        return Err("a command is required".into());
    };
    let command = match name {
        "init" if args.len() == 1 => Command::Init,
        "serve" if args.len() == 1 => Command::Serve,
        "unload" if args.len() == 1 => Command::Unload,
        "status" if args.len() == 1 => Command::Status,
        "devices" if args.len() == 1 => Command::Devices,
        "stop" if args.len() == 1 => Command::Stop,
        "version" if args.len() == 2 && args[1] == "--json" => Command::Version,
        "cancel" if args.len() == 2 => {
            Command::Cancel(Uuid::parse_str(text(&args[1])?).map_err(|_| "cancel requires a UUID")?)
        }
        "models" if args.len() == 2 && args[1] == "list" => Command::List,
        "models" if args.get(1).is_some_and(|x| x == "import") => {
            let mut pairs = pairs(&args[2..], &["--id", "--file"])?;
            let id = ModelId::new(text(pairs.remove("--id").ok_or("--id is required")?)?)
                .map_err(|_| "invalid model ID")?;
            let file = PathBuf::from(pairs.remove("--file").ok_or("--file is required")?);
            if file.as_os_str().is_empty() {
                return Err("--file cannot be empty".into());
            }
            Command::Import { id, file }
        }
        "load" if args.len() >= 2 => {
            let id = ModelId::new(text(&args[1])?).map_err(|_| "invalid model ID")?;
            let pairs = pairs(
                &args[2..],
                &[
                    "--backend",
                    "--context",
                    "--gpu-layers",
                    "--threads",
                    "--batch",
                ],
            )?;
            let mut value = json!({"model":id});
            for (key, input) in pairs {
                let field = match key {
                    "--backend" => "backend",
                    "--context" => "context_size",
                    "--gpu-layers" => "gpu_layers",
                    "--threads" => "threads",
                    _ => "batch_size",
                };
                value[field] = if key == "--backend" {
                    json!(text(input)?)
                } else {
                    json!(
                        text(input)?
                            .parse::<u32>()
                            .map_err(|_| "load values must be unsigned integers")?
                    )
                };
            }
            Command::Load(value)
        }
        _ => return Err("unsupported command or invalid arguments".into()),
    };
    Ok(Options { data_dir, command })
}
fn text(value: &OsStr) -> Result<&str> {
    value
        .to_str()
        .ok_or_else(|| "argument must be UTF-8".into())
}
fn pairs<'a>(args: &'a [OsString], allowed: &[&str]) -> Result<BTreeMap<&'a str, &'a OsStr>> {
    let mut pairs = BTreeMap::new();
    for pair in args.chunks(2) {
        if pair.len() != 2 {
            return Err("each option requires a value".into());
        }
        let key = text(&pair[0])?;
        if !allowed.contains(&key) {
            return Err("unknown option".into());
        }
        if pairs.insert(key, pair[1].as_os_str()).is_some() {
            return Err("duplicate option".into());
        }
    }
    Ok(pairs)
}
fn default_data_dir() -> Result<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA")
            .map(|p| PathBuf::from(p).join("Nexa"))
            .ok_or_else(|| "LOCALAPPDATA unavailable; specify --data-dir".into())
    }
    #[cfg(not(windows))]
    {
        if let Some(path) = std::env::var_os("XDG_DATA_HOME") {
            let path = PathBuf::from(path);
            if path.is_absolute() {
                return Ok(path.join("nexa"));
            }
        }
        std::env::var_os("HOME")
            .map(|p| PathBuf::from(p).join(".local/share/nexa"))
            .ok_or_else(|| "home directory unavailable; specify --data-dir".into())
    }
}
fn read_config(root: &Path) -> Result<Config> {
    Ok(runtime_api::configuration::read(root)?.config)
}
fn require_initialized(root: &Path) -> Result<Config> {
    let config = read_config(root)?;
    let _ = load_private_token(root)?;
    Ok(config)
}
fn print(value: Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}
fn local_source(file: &Path) -> Result<PathBuf> {
    let value = file.to_string_lossy();
    if value.contains("://") || value.starts_with("\\\\") || value.starts_with("//") {
        return Err("only local regular model files are supported".into());
    }
    if !fs::symlink_metadata(file)
        .map_err(|_| "model source is unavailable")?
        .file_type()
        .is_file()
    {
        return Err("model source must be a regular file, not a symlink".into());
    }
    let canonical = fs::canonicalize(file).map_err(|_| "cannot resolve selected model file")?;
    #[cfg(windows)]
    {
        if let Some(normal) = canonical
            .to_str()
            .and_then(|s| s.strip_prefix(r"\\?\"))
            .filter(|s| s.as_bytes().get(1) == Some(&b':') && s.as_bytes().get(2) == Some(&b'\\'))
        {
            return Ok(PathBuf::from(normal));
        }
    }
    if canonical.to_str().is_none() {
        return Err("model path must be representable as UTF-8 for the local API".into());
    }
    Ok(canonical)
}
pub async fn execute(options: Options) -> Result<()> {
    let root = options.data_dir;
    if matches!(options.command, Command::Version) {
        return print(
            json!({"name":"Nexa","version":env!("CARGO_PKG_VERSION"),"protocol_version":runtime_types::PROTOCOL_VERSION,"target_os":std::env::consts::OS,"target_arch":std::env::consts::ARCH,"management_native_linkage":false}),
        );
    }
    if matches!(options.command, Command::Init) {
        let lock = InstanceLock::try_acquire(&root)?
            .ok_or("data directory is in use; stop the service before init")?;
        if lock.has_discovery() {
            return Err(
                "runtime_stop_unconfirmed: previous service cleanup is not confirmed".into(),
            );
        }
        runtime_api::configuration::initialize(&root)?;
        return print(json!({"initialized":true}));
    }
    if matches!(options.command, Command::Serve) {
        return serve(&root).await;
    }
    // stop on an absent directory is idempotent and creates no long-lived data.
    if matches!(options.command, Command::Stop) && !root.exists() {
        return print(json!({"stopped":true,"was_running":false}));
    }
    let config = if matches!(options.command, Command::Stop) {
        // Stop uses discovery, ownership and endpoint proof; a damaged saved
        // configuration must not prevent shutting down the authenticated owner.
        let _ = load_private_token(&root)?;
        None
    } else {
        Some(require_initialized(&root)?)
    };
    let lock = InstanceLock::try_acquire(&root)?;
    if let Some(lock) = lock {
        return match options.command {
            Command::Import { id, file } => {
                let value = tokio::task::spawn_blocking(move || -> Result<Value> {
                    let _lock = lock;
                    let file = local_source(&file)?;
                    let store = ModelStore::open(&root)?;
                    let request = ImportRequest::new(
                        id.clone(),
                        id.as_str(),
                        ModelSource::local("user-selected local file"),
                    );
                    let model = store.import_file(file, request, &ImportCancellation::default())?;
                    Ok(serde_json::to_value(ModelSummary::from(model))?)
                })
                .await??;
                print(value)
            }
            Command::List => {
                let value = tokio::task::spawn_blocking(move || -> Result<Value> {
                    let _lock = lock;
                    let store = ModelStore::open(&root)?;
                    let models: Vec<_> = store
                        .list()?
                        .into_iter()
                        .map(|m| ModelSummary::from_store(m, &store))
                        .collect();
                    Ok(json!({"object":"list","data":models}))
                })
                .await??;
                print(value)
            }
            Command::Stop if !lock.has_discovery() => {
                print(json!({"stopped":true,"was_running":false}))
            }
            Command::Stop => {
                Err("service is not running, but a prior instance cleanup was not confirmed".into())
            }
            _ => Err("service is not running; start serve explicitly".into()),
        };
    }
    let discovery = Discovery::read(&root)?;
    let mut client = VerifiedConnection::connect(
        discovery.listen,
        discovery.instance_id,
        load_private_token(&root)?,
    )
    .await?;
    let value = match options.command {
        Command::Import { id, file } => {
            client
                .json(
                    Method::POST,
                    "/runtime/models/import",
                    Some(&json!({"id":id,"file":local_source(&file)?})),
                )
                .await?
        }
        Command::List => {
            let mut all = Vec::new();
            let mut path = "/runtime/models?limit=128".to_string();
            let mut last = None;
            loop {
                let page = client.json(Method::GET, &path, None).await?;
                all.extend(
                    page.get("data")
                        .and_then(Value::as_array)
                        .ok_or("invalid model-list page")?
                        .iter()
                        .cloned(),
                );
                match page.get("next_after").and_then(Value::as_str) {
                    Some(next) => {
                        let id = ModelId::new(next).map_err(|_| "invalid model-list cursor")?;
                        if last
                            .as_ref()
                            .is_some_and(|previous: &String| previous.as_str() >= next)
                        {
                            return Err("model-list cursor did not advance".into());
                        }
                        last = Some(next.to_string());
                        path = format!("/runtime/models?limit=128&after={}", id.as_str());
                    }
                    None => break,
                }
            }
            json!({"object":"list","data":all})
        }
        Command::Load(value) => {
            let id = ModelId::new(
                value
                    .get("model")
                    .and_then(Value::as_str)
                    .ok_or("model required")?,
            )?;
            let external = model_store::library::ModelLibrary::read(&root)?
                .is_some_and(|library| library.entry(&id).is_some());
            if external {
                client
                    .json_with_verification(
                        Method::POST,
                        "/runtime/load",
                        Some(&value),
                        config
                            .as_ref()
                            .ok_or("configuration unavailable for load")?
                            .model_verification_timeout(),
                    )
                    .await?
            } else {
                client
                    .json(Method::POST, "/runtime/load", Some(&value))
                    .await?
            }
        }
        Command::Unload => {
            client
                .json(Method::POST, "/runtime/unload", Some(&json!({})))
                .await?
        }
        Command::Status => client.json(Method::GET, "/runtime/status", None).await?,
        Command::Devices => client.json(Method::GET, "/runtime/devices", None).await?,
        Command::Cancel(id) => {
            client
                .json(
                    Method::POST,
                    &format!("/runtime/requests/{id}/cancel"),
                    Some(&json!({})),
                )
                .await?
        }
        Command::Stop => {
            let response = client
                .json(Method::POST, "/runtime/shutdown", Some(&json!({})))
                .await?;
            drop(client);
            wait_stopped(&root, discovery.instance_id, Duration::from_secs(30)).await?;
            response
        }
        _ => unreachable!(),
    };
    print(value)
}
struct BoundApiListeners {
    local: tokio::net::TcpListener,
    lan: Option<tokio::net::TcpListener>,
    lan_startup_error: Option<runtime_api::lan::LanStartupError>,
}
/// Only LAN socket binding may degrade; validation and loopback failures remain fatal.
/// Tests inject bind results without opening a non-loopback test service.
async fn bind_api_listeners<F, Fut>(config: &Config, mut bind: F) -> Result<BoundApiListeners>
where
    F: FnMut(std::net::SocketAddr) -> Fut,
    Fut: std::future::Future<Output = io::Result<tokio::net::TcpListener>>,
{
    config.validate()?;
    let local = bind(config.api.listen)
        .await
        .map_err(|_| "cannot bind configured loopback endpoint")?;
    let mut lan_startup_error = None;
    let lan = if config.lan_api.enabled {
        let address = config
            .lan_api
            .listen
            .ok_or("missing LAN listener address")?;
        match bind(address).await {
            Ok(listener) => Some(listener),
            Err(error) => {
                lan_startup_error = Some(runtime_api::lan::LanStartupError::from_io(&error));
                None
            }
        }
    } else {
        None
    };
    Ok(BoundApiListeners {
        local,
        lan,
        lan_startup_error,
    })
}
async fn serve(root: &Path) -> Result<()> {
    let _ = require_initialized(root)?;
    let lock =
        InstanceLock::try_acquire(root)?.ok_or("another instance owns this data directory")?;
    // A settings writer may have changed LAN enablement before lock acquisition.
    // Only this lock-protected snapshot may authorize any network binding.
    let config = require_initialized(root)?;
    let token = load_private_token(root)?;
    let executable = std::env::current_exe()?.canonicalize()?;
    let worker = executable
        .parent()
        .ok_or("cannot resolve executable directory")?
        .join(if cfg!(windows) {
            "ai-runtime-worker.exe"
        } else {
            "ai-runtime-worker"
        });
    if !fs::symlink_metadata(&worker)
        .map_err(|_| "packaged worker is missing beside ai-runtime")?
        .file_type()
        .is_file()
    {
        return Err("packaged worker must be a regular file beside ai-runtime".into());
    }
    let BoundApiListeners {
        local: listener,
        lan: lan_listener,
        lan_startup_error,
    } = bind_api_listeners(&config, |address| async move {
        tokio::net::TcpListener::bind(address).await
    })
    .await?;
    let listen = listener.local_addr()?;
    // All sockets are bound before creating credentials or a runtime/worker.
    let lan = if let Some(listener) = lan_listener {
        let lan_token = init_private_lan_token(root)?;
        if lan_token.matches_authorization(token.bearer_header_value().as_bytes()) {
            return Err("LAN and management credentials must be independent".into());
        }
        let security = Arc::new(LanSecurityContext::new(lan_token, &config.lan_api)?);
        Some((listener, security, config.lan_api.clone()))
    } else {
        None
    };
    let instance_id = Uuid::new_v4();
    let security = Arc::new(
        SecurityContext::new(
            token,
            instance_id,
            listen,
            config.api.trusted_origins.clone(),
        )
        .map_err(|_| "invalid server security configuration")?,
    );
    // Install Ctrl+C before potentially long startup hashing. If interrupted,
    // finish the owned startup I/O before releasing the instance lock.
    let mut interrupt = InterruptGuard(tokio::spawn(tokio::signal::ctrl_c()));
    let store = ApiState::open_store(root.to_path_buf()).await?;
    let host = ProcessHost::new(ProcessHostConfig::new(worker))?;
    let diagnostics = host.diagnostics();
    let resolver = store.clone();
    let runtime = Runtime::spawn(
        config.runtime_config(),
        move |id: &ModelId| resolver.resolve(id),
        host,
    )?;
    let mut state = ApiState::new(runtime, store.clone(), config, Some(diagnostics));
    state.lan_startup_error = lan_startup_error;
    let shutdown = state.shutdown.clone();
    if let Err(error) = state.initialize_registry().await {
        shutdown.begin();
        let cleanup = state.wait_shutdown().await;
        cleanup?;
        return Err(error.into());
    }
    state.mark_lan_listening(lan.is_some());
    let publication =
        Discovery::current(instance_id, listen).and_then(|discovery| lock.publish(&discovery));
    if let Err(error) = publication {
        shutdown.begin();
        state.wait_shutdown().await?;
        return Err(error.into());
    }
    let app = runtime_api::router(state.clone(), security);
    let result = {
        let lan = lan.map(|(listener, security, config)| {
            (
                listener,
                runtime_api::routes::lan_router(state.clone(), security),
                config,
            )
        });
        let serving = runtime_api::transport::serve_with_lan(listener, app, lan, shutdown.clone());
        tokio::pin!(serving);
        tokio::select! {result=&mut serving=>result,signal=&mut interrupt.0=>{shutdown.begin();let result=serving.await;match signal { Ok(signal)=>signal.and(result),Err(_)=>Err(io::Error::other("interrupt handler failed")) }}}
    };
    state.mark_lan_listening(false);
    shutdown.begin();
    let cleanup = state.wait_shutdown().await;
    // Even startup/transport failure follows the same reaping path. A failed
    // cleanup keeps its marker and produces a nonzero exit code.
    cleanup?;
    drop(state);
    drop(store);
    lock.remove_own(instance_id)?;
    result?;
    Ok(())
}

struct InterruptGuard(tokio::task::JoinHandle<io::Result<()>>);
impl Drop for InterruptGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse_args(args: &[&str]) -> Result<Options> {
        parse(args.iter().map(OsString::from).collect())
    }
    #[test]
    fn strict_command_surface() {
        for args in [
            &["start"][..],
            &["models", "delete", "a"],
            &["version"],
            &["status", "--data-dir", "x"],
            &["load", "a", "--context", "2048", "--context", "2048"],
            &["cancel", "not-uuid"],
        ] {
            assert!(parse_args(args).is_err(), "{args:?}");
        }
        assert!(
            parse_args(&[
                "load",
                "qa-small",
                "--context",
                "2048",
                "--threads",
                "2",
                "--batch",
                "128"
            ])
            .is_ok()
        );
        assert!(parse_args(&["--data-dir", "temporary", "init"]).is_ok());
    }
}

#[cfg(test)]
mod lan_start_tests {
    use super::*;
    #[tokio::test]
    async fn bind_failures_are_typed_without_retries_or_changed_addresses() {
        use runtime_api::lan::LanStartupError;
        for (kind, expected) in [
            (
                io::ErrorKind::AddrNotAvailable,
                LanStartupError::AddressUnavailable,
            ),
            (io::ErrorKind::AddrInUse, LanStartupError::PortInUse),
            (
                io::ErrorKind::PermissionDenied,
                LanStartupError::PermissionDenied,
            ),
            (io::ErrorKind::Other, LanStartupError::BindFailed),
        ] {
            let mut config = Config::default();
            config.api.listen = "127.0.0.1:0".parse().unwrap();
            config.lan_api = runtime_api::LanApiConfig {
                enabled: true,
                listen: Some("192.168.10.2:18081".parse().unwrap()),
                allowed_cidrs: vec!["192.168.10.3/32".into()],
            };
            let original = config.to_toml().unwrap();
            let mut attempted = Vec::new();
            let result = bind_api_listeners(&config, |address| {
                attempted.push(address);
                async move {
                    if address.ip().is_loopback() {
                        tokio::net::TcpListener::bind(address).await
                    } else {
                        Err(io::Error::new(kind, "untrusted OS detail"))
                    }
                }
            })
            .await
            .unwrap();
            assert_eq!(
                attempted,
                vec![config.api.listen, config.lan_api.listen.unwrap()]
            );
            assert_eq!(result.lan_startup_error, Some(expected));
            assert!(result.lan.is_none());
            let client = tokio::net::TcpStream::connect(result.local.local_addr().unwrap())
                .await
                .unwrap();
            assert!(result.local.accept().await.is_ok());
            drop(client);
            assert_eq!(config.to_toml().unwrap(), original);
        }
    }
    #[tokio::test]
    async fn successful_explicit_attempt_has_no_stale_startup_error() {
        let mut config = Config::default();
        config.api.listen = "127.0.0.1:0".parse().unwrap();
        config.lan_api = runtime_api::LanApiConfig {
            enabled: true,
            listen: Some("192.168.10.2:18081".parse().unwrap()),
            allowed_cidrs: vec!["192.168.10.3/32".into()],
        };
        let mut attempted = Vec::new();
        // Synthetic binder only: no test opens a non-loopback socket.
        let result = bind_api_listeners(&config, |address| {
            attempted.push(address);
            async { tokio::net::TcpListener::bind("127.0.0.1:0").await }
        })
        .await
        .unwrap();
        assert_eq!(
            attempted,
            vec![config.api.listen, config.lan_api.listen.unwrap()]
        );
        assert!(result.lan.is_some());
        assert!(result.lan_startup_error.is_none());
    }
    #[tokio::test]
    async fn invalid_configuration_and_loopback_bind_still_fail_closed() {
        let mut config = Config::default();
        config.lan_api.enabled = true;
        assert!(
            bind_api_listeners(&config, |_| async {
                panic!("must validate before binding")
            })
            .await
            .is_err()
        );
        config.lan_api.enabled = false;
        let mut calls = 0;
        assert!(
            bind_api_listeners(&config, |_| {
                calls += 1;
                async { Err(io::Error::from(io::ErrorKind::AddrInUse)) }
            })
            .await
            .is_err()
        );
        assert_eq!(calls, 1);
    }
    #[tokio::test]
    async fn second_bind_failure_keeps_local_and_disabled_lan_never_calls_second_binder() {
        let mut config = Config::default();
        config.api.listen = "127.0.0.1:0".parse().unwrap();
        config.lan_api = runtime_api::LanApiConfig {
            enabled: true,
            listen: Some("192.168.10.2:18081".parse().unwrap()),
            allowed_cidrs: vec!["192.168.10.3/32".into()],
        };
        let bound = Arc::new(std::sync::Mutex::new(None));
        let observed = bound.clone();
        let result = bind_api_listeners(&config, move |address| {
            let observed = observed.clone();
            async move {
                if !address.ip().is_loopback() {
                    return Err(io::Error::new(
                        io::ErrorKind::AddrNotAvailable,
                        "fixture LAN bind failed",
                    ));
                }
                let listener = tokio::net::TcpListener::bind(address).await?;
                *observed.lock().unwrap() = Some(listener.local_addr()?);
                Ok(listener)
            }
        })
        .await;
        let result = result.unwrap();
        assert!(result.lan.is_none());
        assert_eq!(
            result.lan_startup_error,
            Some(runtime_api::lan::LanStartupError::AddressUnavailable)
        );
        let address = bound.lock().unwrap().unwrap();
        assert!(tokio::net::TcpListener::bind(address).await.is_err());
        assert!(config.lan_api.enabled);
        drop(result);
        let rebound = tokio::net::TcpListener::bind(address).await.unwrap();
        drop(rebound);
        config.lan_api.enabled = false;
        let BoundApiListeners {
            local,
            lan,
            lan_startup_error,
        } = bind_api_listeners(&config, |address| async move {
            assert!(address.ip().is_loopback());
            tokio::net::TcpListener::bind(address).await
        })
        .await
        .unwrap();
        assert!(lan.is_none());
        assert!(lan_startup_error.is_none());
        drop(local);
    }
}
