//! Explicit acceptance executable: uniquely-created temporary credentials only.
//! Both success and failure stdout are closed, sanitized JSON protocols.
#[path = "harness/external.rs"]
mod external;
#[path = "harness/lifecycle.rs"]
mod lifecycle;
use lifecycle::lifecycle_child;
#[path = "harness/probe.rs"]
mod probe;
#[path = "harness/report.rs"]
mod report;
use desktop_bridge::{
    ChatEvent, ChatStartRequest, ConnectionState, DesktopBridge, DesktopPreferences,
    LoadModelRequest, RuntimeState,
};
use report::{Cleanup, FailureReport, Fault};
use runtime_api::{
    Config,
    token::{create_private_dir, init_private_token, write_private_new},
};
use runtime_cli::instance::{Discovery, InstanceLock};
use runtime_types::{Message, Role};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use uuid::Uuid;
type Result<T> = std::result::Result<T, Fault>;
macro_rules! at {
    ($stage:ident, $name:literal, $expression:expr) => {{
        $stage = $name;
        $expression?
    }};
}
fn ensure(condition: bool) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(Fault::new("assertion_failed"))
    }
}
fn arguments(args: Vec<std::ffi::OsString>) -> Result<(PathBuf, PathBuf)> {
    if args.len() != 4 || args[0] != "--runtime" || args[2] != "--model" {
        return Err(Fault::new("invalid_arguments"));
    }
    let runtime = PathBuf::from(&args[1]);
    let model = PathBuf::from(&args[3]);
    if !runtime.is_absolute() || !model.is_absolute() {
        return Err(Fault::new("invalid_arguments"));
    }
    Ok((runtime, model))
}
fn path_shape(path: &Path) -> Value {
    let text = path.to_string_lossy();
    json!({"contains_non_ascii":!text.is_ascii(),"contains_space":text.contains(' ')})
}
async fn start(bridge: &DesktopBridge) -> Result<desktop_bridge::DesktopSnapshot> {
    bridge
        .start(false)
        .await
        .map_err(|error| Fault::startup(error, bridge.startup_diagnostics()))
}
async fn child_main(args: Vec<std::ffi::OsString>) -> std::result::Result<(), Box<FailureReport>> {
    let mut stage = "child_arguments";
    let result: Result<()> = async {
        if args.len() != 8
            || args[0] != "--lifecycle-child"
            || args[1] != "--runtime"
            || args[3] != "--data-dir"
            || !(args[5] == "keep" || args[5] == "stop")
            || lifecycle::report_directory(&args).is_none()
        {
            return Err(Fault::new("invalid_arguments"));
        }
        let bridge = at!(
            stage,
            "child_construct_bridge",
            DesktopBridge::new(PathBuf::from(&args[4]), PathBuf::from(&args[2]))
        );
        at!(stage, "child_start", start(&bridge).await);
        at!(
            stage,
            "child_save_preferences",
            bridge
                .settings_save(DesktopPreferences {
                    close_runtime_on_exit: args[5] == "stop",
                    ..Default::default()
                })
                .await
        );
        at!(stage, "child_close", bridge.close().await);
        Ok(())
    }
    .await;
    result.map_err(|fault| Box::new(FailureReport::new(stage, fault, Cleanup::not_needed(true))))
}
fn request(max: u32) -> ChatStartRequest {
    ChatStartRequest {
        model_id: "desktop-qa".into(),
        messages: vec![Message::new(Role::User, "请用中文解释为什么天空是蓝色的。")],
        max_output_tokens: max,
    }
}
async fn consume(bridge: &DesktopBridge, id: Uuid, cancel: bool) -> Result<(usize, u64)> {
    tokio::time::timeout(Duration::from_secs(90), async {
        let mut bytes = 0;
        let mut cancelled = false;
        loop {
            let batch = bridge.chat_next(id).await?;
            for event in batch.events {
                match event {
                    ChatEvent::Delta { text } => {
                        bytes += text.len();
                        if cancel && !cancelled {
                            bridge.chat_cancel(id).await?;
                            cancelled = true;
                        }
                    }
                    ChatEvent::Completed { usage, .. } if !cancel => {
                        return Ok((bytes, usage.total_tokens));
                    }
                    ChatEvent::Cancelled if cancel && cancelled => return Ok((bytes, 0)),
                    ChatEvent::Failed { code, .. } => {
                        return Err(Fault::bridge(desktop_bridge::BridgeError {
                            code,
                            message: String::new(),
                        }));
                    }
                    other if other.is_terminal() => return Err(Fault::new("assertion_failed")),
                    _ => (),
                }
            }
        }
    })
    .await
    .map_err(|_| Fault::new("timeout"))?
}
async fn ready(bridge: &DesktopBridge) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let s = bridge.snapshot().await?;
            if s.runtime.is_some_and(|r| {
                matches!(r.state, RuntimeState::Ready) && r.active_request.is_none()
            }) {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .map_err(|_| Fault::new("timeout"))?
}
fn observe_cleanup(root: &Path, cleanup: &mut Cleanup) {
    cleanup.instance_lock = match InstanceLock::try_acquire(root) {
        Ok(Some(_)) => "free",
        Ok(None) => "held",
        Err(_) => "unavailable",
    }
    .into();
    cleanup.discovery = match std::fs::symlink_metadata(root.join("runtime/instance.json")) {
        Ok(_) => "present",
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => "absent",
        Err(_) => "unavailable",
    }
    .into();
    cleanup.temporary_data_retained = root.exists();
}
async fn cleanup(root: &Path, runtime: &Path) -> Cleanup {
    let result: Result<()> = async {
        let bridge = DesktopBridge::new(root.to_owned(), runtime.to_owned())?;
        bridge.stop().await?;
        Ok(())
    }
    .await;
    let mut cleanup = Cleanup::not_needed(root.exists());
    cleanup.status = if result.is_ok() {
        "confirmed"
    } else {
        "unconfirmed"
    }
    .into();
    if let Err(fault) = result {
        cleanup.code = Some(fault.code.into());
        cleanup.bridge_code = fault.bridge_code;
        cleanup.os_error = fault.os_error;
    }
    observe_cleanup(root, &mut cleanup);
    if cleanup.status == "confirmed"
        && (cleanup.instance_lock != "free" || cleanup.discovery != "absent")
    {
        cleanup.status = "unconfirmed".into();
        cleanup.code = Some("cleanup_unconfirmed".into());
    }
    cleanup
}
async fn run(
    root: PathBuf,
    runtime: PathBuf,
    model: PathBuf,
) -> std::result::Result<Value, Box<FailureReport>> {
    let mut stage = "create_private_directory";
    let mut bridge_created = false;
    let data_shape = path_shape(&root);
    let runtime_shape = path_shape(&runtime);
    let model_shape = path_shape(&model);
    let external_model_path = model.clone();
    let outcome:Result<Value>=async {
        at!(stage,"create_private_directory",create_private_dir(&root));
        at!(stage,"initialize_token",init_private_token(&root));
        let mut config=Config::default();config.api.listen=std::net::SocketAddr::from(([127,0,0,1],0));config.inference.context_size=2048;config.inference.threads=Some(2);config.inference.batch_size=128;
        let text=config.to_toml().map_err(|_|Fault::new("configuration_error"))?;
        at!(stage,"write_config",write_private_new(&root.join("config.toml"),text.as_bytes()));
        let bridge=Arc::new(at!(stage,"construct_bridge",DesktopBridge::new(root.clone(),runtime.clone())));
        bridge_created=true;
        at!(stage,"launch_initial_child",lifecycle_child(&runtime,&root,false).await);
        let started=at!(stage,"start_after_child_exit",start(&bridge).await);
        ensure(matches!(started.connection,ConnectionState::Connected))?;
        let first_instance=at!(stage,"read_initial_instance",Discovery::read(&root)).instance_id;
        let attached=Arc::new(DesktopBridge::new(root.clone(),runtime.clone())?);
        at!(stage,"attach_existing",start(&attached).await);
        ensure(Discovery::read(&root)?.instance_id==first_instance)?;
        at!(stage,"close_attached_window",attached.close_ui_only().await);
        let model=at!(stage,"import_model",bridge.import_model(model,"desktop-qa".into()).await);
        ensure(model.available)?;
        let page=at!(stage,"list_models",bridge.models_page(None, None).await);
        ensure(page.data.len()==1&&page.data[0].id.as_str()=="desktop-qa")?;
        stage="reject_running_idle_change";
        match bridge.save_idle(600).await {Err(error) if error.code=="runtime_running"=>(),Err(error)=>return Err(error.into()),Ok(_)=>return Err(Fault::new("assertion_failed"))}
        at!(stage,"load_model",bridge.load_model(LoadModelRequest{model_id:"desktop-qa".into(),context_size:2048,threads:2,batch_size:128}).await);
        let verified=at!(stage,"verify_local_validation",bridge.models_page(None,None).await);
        let validation=verified.data[0].local_validation.as_ref().ok_or_else(||Fault::new("assertion_failed"))?;
        ensure(validation.state==model_store::local_validation::ValidationState::Passed && validation.load_success && validation.generation_pass && validation.error_code.is_none())?;
        let receipts=model_store::local_validation::Receipts::read(&root).map_err(|_|Fault::new("assertion_failed"))?;
        ensure(receipts.schema_version==1 && receipts.entries.len()==1 && receipts.entries[0].scope.model_sha256==model.sha256)?;
        let repeated=at!(stage,"repeat_model_test",bridge.model_test(LoadModelRequest{model_id:"desktop-qa".into(),context_size:2048,threads:2,batch_size:128}).await);
        ensure(repeated.state==model_store::local_validation::ValidationState::Passed && repeated.load_success && repeated.generation_pass && repeated.error_code.is_none())?;
        let replaced=model_store::local_validation::Receipts::read(&root).map_err(|_|Fault::new("assertion_failed"))?;
        ensure(replaced.entries.len()==1 && replaced.entries[0].observation==repeated)?;
        let first=at!(stage,"first_chat_start",bridge.chat_start(request(32))).request_id;
        let(first_bytes,usage)=at!(stage,"first_chat_consume",consume(&bridge,first,false).await);
        ensure(first_bytes>0&&usage>0)?;
        let cancel=at!(stage,"cancel_chat_start",bridge.chat_start(request(512))).request_id;
        let(cancel_bytes,_)=at!(stage,"cancel_chat_consume",consume(&bridge,cancel,true).await);
        at!(stage,"wait_ready",ready(&bridge).await);
        let repeat=at!(stage,"repeat_chat_start",bridge.chat_start(request(16))).request_id;
        let(repeat_bytes,_)=at!(stage,"repeat_chat_consume",consume(&bridge,repeat,false).await);
        ensure(repeat_bytes>0)?;
        at!(stage,"close_window",bridge.close().await);
        let after_close=Arc::new(DesktopBridge::new(root.clone(),runtime.clone())?);
        let observed=at!(stage,"verify_default_close",after_close.snapshot().await);
        ensure(matches!(observed.connection,ConnectionState::Connected)&&Discovery::read(&root)?.instance_id==first_instance)?;
        at!(stage,"launch_keep_child",lifecycle_child(&runtime,&root,false).await);
        let observed=at!(stage,"verify_process_exit",after_close.snapshot().await);
        ensure(matches!(observed.connection,ConnectionState::Connected)&&Discovery::read(&root)?.instance_id==first_instance)?;
        at!(stage,"unload_model",after_close.unload_model().await);
        at!(stage,"reload_model",after_close.load_model(LoadModelRequest{model_id:"desktop-qa".into(),context_size:2048,threads:2,batch_size:128}).await);
        at!(stage,"launch_stop_child",lifecycle_child(&runtime,&root,true).await);
        at!(stage,"save_close_preference",after_close.settings_save(DesktopPreferences{close_runtime_on_exit:true,..Default::default()}).await);
        stage="repeated_close";
        let(a,b)=tokio::join!(after_close.close(),after_close.close());a?;b?;
        stage="verify_instance_released";
        ensure(Discovery::read(&root).is_err()&&InstanceLock::try_acquire(&root)?.is_some())?;
        let stopped=Arc::new(DesktopBridge::new(root.clone(),runtime.clone())?);
        let saved=at!(stage,"save_stopped_idle",stopped.save_idle(600).await);
        ensure(saved.settings.idle_unload_seconds==600)?;
        at!(stage,"restart_runtime",start(&stopped).await);
        at!(stage,"stop_runtime",stopped.stop().await);
        let offline=at!(stage,"verify_offline_inventory",stopped.models_page(None,None).await);
        ensure(offline.source==desktop_bridge::ModelsSource::Local && offline.data.len()==1 && offline.data[0].local_validation.as_ref().is_some_and(|v|v.state==model_store::local_validation::ValidationState::Passed))?;
        let external_library = external::run(&root, &runtime, &external_model_path, &mut stage).await?;
        Ok(json!({"external_library":external_library,"local_text_validation":true,"repeat_text_validation":true,"offline_inventory":true,"success":true,"real_model":true,"threads":2,"context_size":2048,"batch_size":128,"first_output_bytes":first_bytes,"cancel_partial_bytes":cancel_bytes,"repeat_output_bytes":repeat_bytes,"first_usage_tokens":usage,"model_size_bytes":model.size_bytes,"model_sha256":model.sha256,"data_dir_path_shape":data_shape,"runtime_path_shape":runtime_shape,"model_path_shape":model_shape,"same_instance_attach":true,"default_close_kept_runtime":true,"actual_process_exit_kept_runtime":true,"actual_process_close_runtime_reaped":true,"repeated_close":true,"close_runtime_released_instance":true,"runtime_setting_rejected_while_running":true,"runtime_setting_persisted_stopped":true}))
    }.await;
    // Cleanup is independent evidence, never a replacement for the first cause.
    let mut cleanup = if bridge_created {
        cleanup(&root, &runtime).await
    } else {
        Cleanup::not_needed(root.exists())
    };
    let value = match outcome {
        Ok(value) => value,
        Err(fault) => {
            lifecycle::apply_reap_observation(&fault, &mut cleanup);
            return Err(Box::new(FailureReport::new(stage, fault, cleanup)));
        }
    };
    if cleanup.status != "confirmed" {
        return Err(Box::new(FailureReport::new(
            "cleanup_stop",
            Fault::new("cleanup_unconfirmed"),
            cleanup,
        )));
    }
    if let Err(error) = std::fs::remove_dir_all(&root) {
        cleanup.temporary_data_retained = root.exists();
        return Err(Box::new(FailureReport::new(
            "remove_temporary_data",
            Fault::io(error),
            cleanup,
        )));
    }
    Ok(value)
}
fn emit_failure(report: &FailureReport) {
    let bytes = serde_json::to_vec(&report).expect("primitive diagnostic serialization");
    if !report.validate() || bytes.len() > report::MAX_REPORT_BYTES {
        std::process::exit(1);
    }
    println!("{}", std::str::from_utf8(&bytes).expect("JSON is UTF-8"));
}
#[tokio::main]
async fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "--probe-launch") {
        let strategy = match args.as_slice() {
            [_] => Some(report::LaunchStrategy::Breakaway),
            [_, value] => report::LaunchStrategy::parse(value),
            _ => None,
        };
        let Some(strategy) = strategy else {
            emit_failure(&FailureReport::new(
                "arguments",
                Fault::new("invalid_arguments"),
                Cleanup::not_needed(false),
            ));
            std::process::exit(1);
        };
        let report = probe::run(strategy).await;
        let Some(bytes) = report.encode() else {
            std::process::exit(1);
        };
        println!("{}", std::str::from_utf8(&bytes).expect("JSON is UTF-8"));
        // A valid bounded observation is a successful probe execution, even
        // when the observed Windows registration or spawn failed.
        return;
    }
    if args.first().is_some_and(|s| s == "--signal-probe-child") {
        if args.len() != 2 || !probe::child(Path::new(&args[1])).await {
            std::process::exit(2);
        }
        return;
    }
    if args.first().is_some_and(|s| s == "--lifecycle-child") {
        let Some(directory) = lifecycle::report_directory(&args) else {
            std::process::exit(2);
        };
        let outcome = child_main(args).await;
        if lifecycle::publish_report(&directory, &outcome).is_err() {
            std::process::exit(2);
        }
        if outcome.is_err() {
            std::process::exit(1);
        }
        return;
    }
    let outcome = match arguments(args) {
        Ok((runtime, model)) => {
            run(
                std::env::temp_dir().join(format!("Nexa 桌面 bridge 验证 {}", Uuid::new_v4())),
                runtime,
                model,
            )
            .await
        }
        Err(fault) => Err(Box::new(FailureReport::new(
            "arguments",
            fault,
            Cleanup::not_needed(false),
        ))),
    };
    match outcome {
        Ok(value) => println!("{value}"),
        Err(report) => {
            emit_failure(&report);
            std::process::exit(1);
        }
    }
}
