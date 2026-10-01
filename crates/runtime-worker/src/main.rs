use std::process::ExitCode;

fn main() -> ExitCode {
    // Never print panic payloads, arguments, model paths, or generated text.
    std::panic::set_hook(Box::new(|_| {}));
    if std::env::args_os().len() != 1 {
        eprintln!("worker: invalid invocation");
        return ExitCode::FAILURE;
    }
    if runtime_worker::run().is_err() {
        eprintln!("worker: transport or execution failure");
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
