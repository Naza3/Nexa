//! Explicit acceptance executable: newly-created temporary credentials only.
//! stdout contains numeric/boolean evidence, never prompts, output, or tokens.
use desktop_bridge::{
    ChatEvent, ChatStartRequest, ConnectionState, DesktopBridge, DesktopPreferences,
    LoadModelRequest, RuntimeState,
};
use runtime_api::{
    Config,
    token::{create_private_dir, init_private_token, write_private_new},
};
use runtime_cli::instance::{Discovery, InstanceLock};
use runtime_types::{Message, Role};
use serde_json::json;
use std::{path::PathBuf, sync::Arc, time::Duration};
use uuid::Uuid;
type AnyResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;
fn arguments() -> AnyResult<(PathBuf, PathBuf)> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 4 || args[0] != "--runtime" || args[2] != "--model" {
        return Err("Usage: nexa-desktop-harness --runtime ABS_AI_RUNTIME --model ABS_GGUF".into());
    }
    let runtime = PathBuf::from(&args[1]);
    let model = PathBuf::from(&args[3]);
    if !runtime.is_absolute() || !model.is_absolute() {
        return Err("Acceptance inputs must be absolute local paths".into());
    }
    Ok((runtime, model))
}
fn path_shape(path: &std::path::Path) -> serde_json::Value {
    let text = path.to_string_lossy();
    json!({"contains_non_ascii":!text.is_ascii(),"contains_space":text.contains(' ')})
}
fn lifecycle_child(runtime: &std::path::Path, root: &std::path::Path, stop: bool) -> AnyResult<()> {
    let status = std::process::Command::new(std::env::current_exe()?)
        .arg("--lifecycle-child")
        .arg("--runtime")
        .arg(runtime)
        .arg("--data-dir")
        .arg(root)
        .arg(if stop { "stop" } else { "keep" })
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::inherit())
        .status()?;
    if !status.success() {
        return Err("independent desktop lifecycle child failed".into());
    }
    Ok(())
}
async fn child_main() -> AnyResult<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 6
        || args[0] != "--lifecycle-child"
        || args[1] != "--runtime"
        || args[3] != "--data-dir"
        || !(args[5] == "keep" || args[5] == "stop")
    {
        return Err("invalid internal lifecycle-child arguments".into());
    }
    let bridge = DesktopBridge::new(PathBuf::from(&args[4]), PathBuf::from(&args[2]))?;
    bridge.start(false).await?;
    bridge
        .settings_save(DesktopPreferences {
            close_runtime_on_exit: args[5] == "stop",
            ..Default::default()
        })
        .await?;
    bridge.close().await?;
    Ok(())
}
fn request(max: u32) -> ChatStartRequest {
    ChatStartRequest {
        model_id: "desktop-qa".into(),
        messages: vec![Message::new(Role::User, "请用中文解释为什么天空是蓝色的。")],
        max_output_tokens: max,
    }
}
async fn consume(bridge: &DesktopBridge, id: Uuid, cancel: bool) -> AnyResult<(usize, u64)> {
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
                        return Err(format!("generation failed: {code}").into());
                    }
                    other if other.is_terminal() => return Err("unexpected chat terminal".into()),
                    _ => (),
                }
            }
        }
    })
    .await
    .map_err(|_| "generation acceptance deadline")?
}
async fn ready(bridge: &DesktopBridge) -> AnyResult<()> {
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
    .map_err(|_| "runtime did not return to ready")?
}
async fn run(root: PathBuf, runtime: PathBuf, model: PathBuf) -> AnyResult<serde_json::Value> {
    // A unique directory is the only permitted credential/config destination.
    create_private_dir(&root)?;
    init_private_token(&root)?;
    let mut config = Config::default();
    config.api.listen = "127.0.0.1:0".parse()?;
    write_private_new(&root.join("config.toml"), config.to_toml()?.as_bytes())?;
    let bridge = Arc::new(DesktopBridge::new(root.clone(), runtime.clone())?);
    let data_shape = path_shape(&root);
    let runtime_shape = path_shape(&runtime);
    let model_shape = path_shape(&model);
    let outcome=async {
        // The child starts the service, closes its bridge and ACTUALLY exits.
        // Parent proof below detects lifetime coupling to the launching UI.
        lifecycle_child(&runtime,&root,false)?;
        let started=bridge.start(false).await?;if !matches!(started.connection,ConnectionState::Connected){return Err("runtime not connected".into());}
        let first_instance=Discovery::read(&root)?.instance_id;
        let attached=Arc::new(DesktopBridge::new(root.clone(),runtime.clone())?);
        attached.start(false).await?;
        if Discovery::read(&root)?.instance_id!=first_instance{return Err("attach replaced runtime".into());}
        attached.close_ui_only().await?;
        let model=bridge.import_model(model,"desktop-qa".into()).await?;
        if !model.available{return Err("real model unavailable".into());}
        let page=bridge.models_page(None).await?;if page.data.len()!=1||page.data[0].id.as_str()!="desktop-qa"{return Err("model page mismatch".into());}
        if bridge.save_idle(600).await.unwrap_err().code!="runtime_running"{return Err("running runtime setting was not rejected".into());}
        bridge.load_model(LoadModelRequest{model_id:"desktop-qa".into(),context_size:2048,threads:2,batch_size:128}).await?;
        let (first_bytes,usage)=consume(&bridge,bridge.chat_start(request(32))?.request_id,false).await?;
        if first_bytes==0||usage==0{return Err("real generation had no output/usage".into());}
        let (cancel_bytes,_)=consume(&bridge,bridge.chat_start(request(512))?.request_id,true).await?;
        ready(&bridge).await?;
        let (repeat_bytes,_)=consume(&bridge,bridge.chat_start(request(16))?.request_id,false).await?;
        if repeat_bytes==0{return Err("post-cancel generation empty".into());}
        bridge.close().await?;
        let after_close=Arc::new(DesktopBridge::new(root.clone(),runtime.clone())?);
        if !matches!(after_close.snapshot().await?.connection,ConnectionState::Connected)||Discovery::read(&root)?.instance_id!=first_instance{return Err("default UI close stopped runtime".into());}
        lifecycle_child(&runtime,&root,false)?;
        if !matches!(after_close.snapshot().await?.connection,ConnectionState::Connected)||Discovery::read(&root)?.instance_id!=first_instance{return Err("actual UI-process exit stopped runtime".into());}
        after_close.unload_model().await?;
        after_close.load_model(LoadModelRequest{model_id:"desktop-qa".into(),context_size:2048,threads:2,batch_size:128}).await?;
        // This independent controller exits after shutdown with a worker loaded.
        lifecycle_child(&runtime,&root,true)?;
        after_close.settings_save(DesktopPreferences{close_runtime_on_exit:true,..Default::default()}).await?;
        let (first_close,second_close)=tokio::join!(after_close.close(),after_close.close());first_close?;second_close?;
        if Discovery::read(&root).is_ok()||InstanceLock::try_acquire(&root)?.is_none(){return Err("close with runtime did not release lock/discovery".into());}
        let stopped=Arc::new(DesktopBridge::new(root.clone(),runtime.clone())?);
        if stopped.save_idle(600).await?.settings.idle_unload_seconds!=600{return Err("stopped runtime setting not persisted".into());}
        stopped.start(false).await?;stopped.stop().await?;
        Ok(json!({"success":true,"real_model":true,"threads":2,"context_size":2048,"batch_size":128,"first_output_bytes":first_bytes,"cancel_partial_bytes":cancel_bytes,"repeat_output_bytes":repeat_bytes,"first_usage_tokens":usage,"model_size_bytes":model.size_bytes,"model_sha256":model.sha256,"data_dir_path_shape":data_shape,"runtime_path_shape":runtime_shape,"model_path_shape":model_shape,"same_instance_attach":true,"default_close_kept_runtime":true,"actual_process_exit_kept_runtime":true,"actual_process_close_runtime_reaped":true,"repeated_close":true,"close_runtime_released_instance":true,"runtime_setting_rejected_while_running":true,"runtime_setting_persisted_stopped":true}))
    }.await;
    // Even assertion failures attempt authenticated, confirmed stop, never PID kill.
    let cleanup = DesktopBridge::new(root.clone(), runtime)?.stop().await;
    if cleanup.is_err() {
        return Err("acceptance cleanup unconfirmed; temporary directory retained".into());
    }
    outcome
}
#[tokio::main]
async fn main() {
    if std::env::args_os()
        .nth(1)
        .is_some_and(|s| s == "--lifecycle-child")
    {
        if let Err(error) = child_main().await {
            eprintln!("desktop lifecycle child failed: {error}");
            std::process::exit(1);
        }
        return;
    }
    let result = async {
        let (runtime, model) = arguments()?;
        let root = std::env::temp_dir().join(format!("Nexa 桌面 bridge 验证 {}", Uuid::new_v4()));
        let result = run(root.clone(), runtime, model).await;
        match &result {
            Ok(_) => {
                std::fs::remove_dir_all(root)?;
            }
            Err(_) => { /* retain private directory after uncertain cleanup; never print its path */
            }
        }
        result
    }
    .await;
    match result {
        Ok(value) => println!("{value}"),
        Err(error) => {
            eprintln!("desktop acceptance failed: {error}");
            std::process::exit(1);
        }
    }
}
