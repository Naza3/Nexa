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
    match runtime_cli::command::execute(options).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ai-runtime: {error}");
            ExitCode::FAILURE
        }
    }
}
