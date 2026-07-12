fn main() -> std::process::ExitCode {
    match pmoke::run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            pmoke::report_error(&error);
            std::process::ExitCode::FAILURE
        }
    }
}
