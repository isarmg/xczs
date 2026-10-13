use anyhow::{Context, Result, anyhow};
use log::{error, info, warn};
use std::{
    io::{self, Write},
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
    let result = run(raw_args);
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
                eprintln!("{error:#}");
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
            xcss::server_cli::report_error(&envelope, false, exit)
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
    let args = Args::parse(matches)?;
    let result = run_server(args).await;
    if let Err(error) = &result {
        error!("Server failed: {error:#}");
        log::logger().flush();
    }
    result
}

async fn run_server(args: Args) -> Result<()> {
    let data_dir = args.state_dir.as_ref().context("data_dir is required")?;
    xcss::server_cli::runtime_allowed(data_dir).map_err(xcss::server_cli::CliError)?;
    let directory = xcss::state_file::PrivateStateDirectory::open(data_dir)?;
    let _common_lock = directory.try_instance_lock()?;
    xcss::server_cli::runtime_allowed(data_dir).map_err(xcss::server_cli::CliError)?;
    logger::init(&args.root, args.log_level.parse()?)
        .map_err(|e| anyhow!("Failed to init logger, {e}"))?;
    info!(target:"common.config.loaded", "configuration loaded");
    xcss::server_runtime::install_panic_hook();
    let max_connections = args.max_connections;
    let mut signals = ProcessSignals::install()?;
    let listeners =
        BoundListeners::bind([args.bind]).context("Failed to bind configured listen address")?;
    let bound_address = listeners.addresses()[0];
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
            "platform-sqlite".into(),
            "admin-persistent".into(),
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
    let listening = print_listening(bound_address);
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

fn print_listening(address: SocketAddr) -> String {
    format!("Listening on http://{address}/")
}

fn write_listening(mut output: impl Write, listening: &str) -> io::Result<()> {
    writeln!(output, "{listening}")
}
