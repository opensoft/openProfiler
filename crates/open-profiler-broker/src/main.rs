use opensoft_open_profiler_broker::cli;
use std::io::Write;

fn main() {
    let mut arguments = Vec::new();
    for argument in std::env::args_os().skip(1) {
        match argument.into_string() {
            Ok(value) => arguments.push(value),
            Err(_) => {
                // Refused rather than lossily decoded: a flag value the broker
                // cannot read exactly is a flag value it must not act on.
                let mut errors = std::io::stderr().lock();
                let _ = writeln!(
                    errors,
                    "{{\"schema_version\":1,\"kind\":\"openprofiler_broker_error\",\
                     \"code\":\"usage\",\"message\":\"an argument is not valid Unicode\",\
                     \"exit_code\":2}}"
                );
                std::process::exit(2);
            }
        }
    }

    let code = cli::run(
        &arguments,
        &mut std::io::stdin().lock(),
        &mut std::io::stdout().lock(),
        &mut std::io::stderr().lock(),
    );
    std::process::exit(code);
}
