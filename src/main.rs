use anyhow::{Context, Result, anyhow};
use log::{error, info, warn};
use std::{
    io::{self, Read, Write},
    net::SocketAddr,
    sync::Arc,
};
use xcss::server_runtime::{
    BoundListeners, HttpServer, ProcessSignals, ProductDescriptor, ServerRuntime, health_check,
};
use xczs::args::{Args, build_cli};
use xczs::{logger, server::Server};

fn main() -> std::process::ExitCode {
    let raw_args = std::env::args_os().collect::<Vec<_>>();
    let json = raw_args.iter().any(|argument| argument == "--json");
    let result = if raw_args.len() == 2 && raw_args[1] == "web-assets" {
        xczs::server::web_assets_manifest().map(|manifest| print!("{manifest}"))
    } else {
        run(raw_args)
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            let envelope = if let Some(error) = error.downcast_ref::<xcss::config::ConfigError>() {
                error.envelope()
            } else if let Some(error) = error.downcast_ref::<xcss::server_cli::CliError>() {
                error.0.clone()
            } else if let Some(error) = error.downcast_ref::<xcss::state_file::Error>() {
                xcss::server_cli::state_error(error)
            } else if let Some(error) = error.downcast_ref::<xcss::server_cli::SnapshotError>() {
                xcss::server_cli::snapshot_error(error)
            } else {
                if !json {
                    eprintln!("{error:#}");
                }
                xcss::server_cli::ErrorEnvelope::with_code(
                    xcss::server_cli::ErrorCode::new("current_state_invalid").unwrap(),
                    "The command could not validate or operate on the current configuration and data.",
                )
            };
            let exit = if envelope.code.as_str() == "invalid_cli_input" {
                2
            } else {
                1
            };
            xcss::server_cli::report_error(&envelope, json, exit)
        }
    }
}

#[tokio::main]
async fn run(raw_args: Vec<std::ffi::OsString>) -> Result<()> {
    let cmd = build_cli();
    let matches = match cmd.try_get_matches_from(raw_args) {
        Ok(matches) => matches,
        Err(error)
            if matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) =>
        {
            error.print()?;
            return Ok(());
        }
        Err(_) => {
            return Err(
                xcss::server_cli::CliError(xcss::server_cli::ErrorEnvelope::with_code(
                    xcss::server_cli::ErrorCode::new("invalid_cli_input").unwrap(),
                    "Command arguments do not satisfy the current CLI contract; use --help.",
                ))
                .into(),
            );
        }
    };
    let mode = matches
        .subcommand_name()
        .expect("CLI requires a subcommand")
        .to_owned();
    let json = matches.get_flag("json");
    let config_path = xczs::args::selected_config_path(&matches)?;
    let initial_username = matches
        .subcommand_matches("init")
        .and_then(|init| init.get_one::<String>("username"))
        .cloned();
    let reset_username = matches
        .subcommand_matches("admin")
        .and_then(|admin| admin.subcommand_matches("reset-password"))
        .and_then(|reset| reset.get_one::<String>("username"))
        .cloned();
    let args = Args::parse(matches)?;
    match mode.as_str() {
        "init" => {
            let username = initial_username.as_deref().unwrap_or("admin");
            let password = read_initial_password(std::io::stdin().lock())?;
            Server::initialize(args, username, &password)?;
            xcss::log::LogRecord::common("xczs", xcss::log::CommonEvent::InitializationCompleted)?
                .emit_stderr()?;
            println!(
                "{}",
                serde_json::json!({"status":"initialized", "ready":false})
            );
            return Ok(());
        }
        "config" => {
            println!(
                "{}",
                serde_json::json!({"status":"valid", "sources":args.config_sources, "config_path":config_path})
            );
            return Ok(());
        }
        "check" => {
            let schema = Server::validate_current(&args)?;
            println!(
                "{}",
                serde_json::json!({"status":"valid", "schema_identity":schema, "state_paths":[args.state_dir, args.serve_path]})
            );
            return Ok(());
        }
        "admin" => {
            let password = read_initial_password(std::io::stdin().lock())?;
            Server::reset_password(
                &args,
                reset_username.as_deref().unwrap_or("admin"),
                &password,
            )?;
            println!("{}", serde_json::json!({"status":"password_reset"}));
            return Ok(());
        }
        "status" => {
            let report = xcss::server_cli::query_status(args.bind, "xczs")
                .await
                .map_err(xcss::server_cli::CliError)?;
            if !report.ready {
                return Err(xcss::server_cli::CliError(
                    xcss::server_cli::ErrorEnvelope::with_code(
                        xcss::server_cli::ErrorCode::new("service_not_ready").unwrap(),
                        "The service answered but its business readiness checks failed.",
                    ),
                )
                .into());
            }
            xcss::server_cli::print_report(&report, json)?;
            return Ok(());
        }
        _ => {}
    }
    let result = run_server(args, json).await;
    if let Err(error) = &result {
        error!("Server failed: {error:#}");
        log::logger().flush();
    }
    result
}

async fn run_server(args: Args, json: bool) -> Result<()> {
    let data_dir = args.state_dir.as_ref().context("data_dir is required")?;
    xcss::server_cli::runtime_allowed(data_dir).map_err(xcss::server_cli::CliError)?;
    Server::validate_current(&args)?;
    let directory = xcss::state_file::PrivateStateDirectory::open(data_dir)?;
    let _common_lock = directory.try_instance_lock()?;
    xcss::server_cli::runtime_allowed(data_dir).map_err(xcss::server_cli::CliError)?;
    Server::validate_current(&args)?;
    logger::init(data_dir, args.log_level.parse()?)
        .map_err(|e| anyhow!("Failed to init logger, {e}"))?;
    info!(target:"common.config.loaded", "configuration loaded");
    xcss::server_runtime::install_panic_hook();
    let max_connections = args.max_connections;
    let mut signals = ProcessSignals::install()?;
    let listeners =
        BoundListeners::bind([args.bind]).context("Failed to bind configured listen address")?;
    let bound_address = listeners.addresses()[0];
    let port = bound_address.port();
    let runtime_handle = tokio::runtime::Handle::current();
    let mut startup = tokio::task::spawn_blocking(move || {
        let _entered = runtime_handle.enter();
        Server::builder(args).build()
    });
    let product = tokio::select! {
        biased;
        signal = signals.recv() => {
            info!("Shutdown requested during startup reason={}", signal?);
            final_exit(0);
        },
        result = &mut startup => Arc::new(result.context("Server startup task failed")??),
    };
    let server = product.server().clone();
    let health_server = server.clone();
    let runtime = ServerRuntime::builder(ProductDescriptor {
        id: "xczs".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        common_revision: env!("XCSS_REVISION").into(),
        profile: "server-filesystem".into(),
        capabilities: vec![
            "embedded-web".into(),
            "admin-static".into(),
            "memory-sessions".into(),
            "server-runtime".into(),
            "server-health".into(),
            "filesystem-root".into(),
            "linux-openat2".into(),
            "durable-operations".into(),
        ],
    })
    .with_schema_identity(server.schema_identity()?)
    .register_health_check(
        "filesystem",
        health_check(move || {
            let server = health_server.clone();
            async move { server.probe_readiness().await }
        }),
    )
    .register_background_task(
        "shutdown-log",
        xcss::server_runtime::TaskCriticality::Degrading,
        |mut shutdown| async move {
            if !*shutdown.borrow() {
                let _ = shutdown.changed().await;
            }
            info!(target:"common.runtime.shutdown_started", "shutdown started");
            Ok(())
        },
    )
    .build()
    .await?;
    let service = server.http_service(runtime.handle())?;
    let mut transport = HttpServer::new(listeners, signals);
    transport.limits.max_connections = max_connections;
    transport.participant = Some(product);
    info!(target:"common.runtime.started", "runtime started");
    let listening = if json {
        serde_json::json!({"service":"xczs", "state":"listening", "port":port}).to_string() + "\n"
    } else {
        print_listening(bound_address)
    };
    if let Err(error) = write_listening(std::io::stdout().lock(), &listening) {
        warn!("Failed to write listening address to stdout: {error}");
    }
    match runtime.serve(transport, service).await {
        Ok(report) => {
            info!(target:"common.runtime.stopped", "runtime stopped");
            info!(
                "Graceful shutdown complete clean={} phase={:?}",
                report.clean, report.phase
            );
            final_exit(0);
        }
        Err(error) => {
            error!("Server shutdown failed: {error}");
            // Keep the error's business-state retention owner alive until the
            // executable terminates; never let runtime Drop release stuck I/O.
            final_exit(1);
        }
    }
}

fn final_exit(code: i32) -> ! {
    log::logger().flush();
    std::process::exit(code)
}

fn validate_cli_password(password: &str) -> Result<()> {
    xcss::admin_auth::validate_password(password)
        .context("Password violates the current administrator policy")
}

fn print_listening(address: SocketAddr) -> String {
    format!("Listening on http://{address}/")
}

fn write_listening(mut output: impl Write, listening: &str) -> io::Result<()> {
    writeln!(output, "{listening}")
}

fn read_initial_password(mut input: impl std::io::Read) -> Result<String> {
    let mut bytes = Vec::new();
    input
        .by_ref()
        .take((xcss::admin_auth::PASSWORD_MAX_BYTES + 3) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.ends_with(b"\n") {
        bytes.pop();
        if bytes.ends_with(b"\r") {
            bytes.pop();
        }
    }
    let password = String::from_utf8(bytes).context("Password must be UTF-8")?;
    validate_cli_password(&password)?;
    Ok(password)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn initial_password_is_bounded_and_accepts_one_line_ending_only() {
        for input in [
            "initial-password",
            "initial-password\n",
            "initial-password\r\n",
        ] {
            assert_eq!(
                read_initial_password(input.as_bytes()).unwrap(),
                "initial-password"
            );
        }
        for input in [
            String::new(),
            "initial-password\nsecond".into(),
            "initial-password\n\n".into(),
            "x".repeat(xcss::admin_auth::PASSWORD_MAX_BYTES + 1),
        ] {
            assert!(read_initial_password(input.as_bytes()).is_err());
        }
    }

    #[test]
    fn cli_password_validation_uses_the_foundation_administrator_policy() {
        assert!(validate_cli_password("").is_err());
        for password in [
            "p".repeat(xcss::admin_auth::PASSWORD_MIN_BYTES),
            "p".repeat(xcss::admin_auth::PASSWORD_MAX_BYTES),
            "é".repeat(xcss::admin_auth::PASSWORD_MAX_BYTES / "é".len()),
        ] {
            assert!(validate_cli_password(&password).is_ok());
        }
        for password in [
            "p".repeat(xcss::admin_auth::PASSWORD_MIN_BYTES - 1),
            "valid-password\n".to_string(),
            "p".repeat(xcss::admin_auth::PASSWORD_MAX_BYTES + 1),
            format!(
                "a{}",
                "é".repeat(xcss::admin_auth::PASSWORD_MAX_BYTES / "é".len())
            ),
        ] {
            assert!(validate_cli_password(&password).is_err());
        }
    }
}
