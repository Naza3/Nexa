use std::process::ExitCode;
#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.is_empty() || args == ["--help"] || args == ["-h"] {
        println!("{}", runtime_cli::command::HELP);
        return ExitCode::SUCCESS;
    }
    let options = match runtime_cli::command::parse(args) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("{error}\n{}", runtime_cli::command::HELP);
            return ExitCode::from(2);
        }
    };
    let desktop_report = matches!(options.command, runtime_cli::command::Command::Serve)
        && std::env::var_os(runtime_cli::DESKTOP_STARTUP_REPORT_ENV).is_some_and(|v| v == "1");
    match runtime_cli::command::execute(options).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if desktop_report {
                runtime_cli::report_desktop_startup_error(error.as_ref(), std::io::stdout().lock());
            }
            eprintln!("ai-runtime: {error}");
            ExitCode::FAILURE
        }
    }
}
