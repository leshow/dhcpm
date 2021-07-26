#![warn(
    missing_debug_implementations,
    // missing_docs, // TODO
    rust_2018_idioms,
    non_snake_case,
    non_upper_case_globals
)]
#![deny(broken_intra_doc_links)]
#![allow(clippy::cognitive_complexity)]

use std::{net::IpAddr, str::FromStr};

use anyhow::{anyhow, Error, Result};
use clap::Clap;
use tokio::{
    runtime::Builder,
    signal,
    sync::{broadcast, mpsc},
};
use tracing::{error, info, trace};
use tracing_subscriber::{
    fmt::{self, format::Pretty},
    prelude::__tracing_subscriber_SubscriberExt,
    util::SubscriberInitExt,
    EnvFilter,
};

mod runner;
mod shutdown;

use runner::Runner;

fn main() -> Result<()> {
    let mut args = Args::parse();

    // set default port for family if none provided
    if args.port.is_none() {
        match args.family {
            Family::INET6 => {
                args.port = Some(546);
            }
            Family::INET => {
                args.port = Some(67);
            }
        }
    }

    init_tracing(&args);

    trace!("{:?}", args);
    let rt = Builder::new_current_thread().enable_all().build()?;
    trace!(?rt, "tokio runtime created");

    // shutdown mechanism courtesy of https://github.com/tokio-rs/mini-redis
    rt.block_on(async move {
        // When the provided `shutdown` future completes, we must send a shutdown
        // message to all active connections. We use a broadcast channel for this
        // purpose. The call below ignores the receiver of the broadcast pair, and when
        // a receiver is needed, the subscribe() method on the sender is used to create
        // one.
        let (notify_shutdown, _) = broadcast::channel(1);
        let (shutdown_complete_tx, shutdown_complete_rx) = mpsc::channel(1);

        let mut runner = Runner {
            args,
            notify_shutdown,
            shutdown_complete_rx,
            shutdown_complete_tx,
        };
        tokio::select! {
            res = runner.run() => {
                if let Err(err) = res {
                    error!(?err, "nailgun exited with an error");
                }
            },
            res = sig() => {
                info!("caught signal handler-- exiting");
                if let Err(err) = res {
                    error!(?err);
                }
            },
        }
        let Runner {
            mut shutdown_complete_rx,
            shutdown_complete_tx,
            notify_shutdown,
            ..
        } = runner;
        trace!("sending shutdown signal");
        // When `notify_shutdown` is dropped, all tasks which have `subscribe`d will
        // receive the shutdown signal and can exit
        drop(notify_shutdown);
        // Drop final `Sender` so the `Receiver` below can complete
        drop(shutdown_complete_tx);

        // Wait for all active connections to finish processing. As the `Sender`
        // handle held by the listener has been dropped above, the only remaining
        // `Sender` instances are held by connection handler tasks. When those drop,
        // the `mpsc` channel will close and `recv()` will return `None`.
        let _ = shutdown_complete_rx.recv().await;

        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

async fn sig() -> Result<()> {
    signal::ctrl_c().await.map_err(|err| anyhow!(err))
}

fn init_tracing(args: &Args) {
    match args.output {
        LogStructure::Pretty => {
            let fmt_layer = fmt::layer()
                .fmt_fields(Pretty::with_source_location(Pretty::default(), false))
                .with_target(false);
            let filter_layer = EnvFilter::try_from_default_env()
                .or_else(|_| EnvFilter::try_new("info"))
                .unwrap();

            tracing_subscriber::registry()
                .with(filter_layer)
                .with(fmt_layer)
                .init();
        }
        LogStructure::Debug => {
            let fmt_layer = fmt::layer();
            let filter_layer = EnvFilter::try_from_default_env()
                .or_else(|_| EnvFilter::try_new("info"))
                .unwrap();

            tracing_subscriber::registry()
                .with(filter_layer)
                .with(fmt_layer)
                .init();
        }
        LogStructure::Json => {
            let fmt_layer = fmt::layer().json();
            let filter_layer = EnvFilter::try_from_default_env()
                .or_else(|_| EnvFilter::try_new("info"))
                .unwrap();

            tracing_subscriber::registry()
                .with(filter_layer)
                .with(fmt_layer)
                .init();
        }
    }
}

/// dhig is a cli tool for sending dhcpv4/v6 messages
#[derive(Debug, Clap, Clone, PartialEq, Eq)]
#[clap(author, about, version)]
pub struct Args {
    /// IP address to send to
    pub ip: IpAddr,
    /// which port use. Default is 67 for dhcpv4 and 546 for dhcpv6
    #[clap(long, short = 'p')]
    pub port: Option<u16>,
    /// which internet family to use, (inet/inet6)
    #[clap(long, short = 'F', default_value = "inet")]
    pub family: Family,
    /// query timeout in seconds. Default is 2.
    #[clap(long, short = 't', default_value = "2")]
    pub timeout: u64,
    /// select the log output format
    #[clap(long, default_value = "pretty")]
    pub output: LogStructure,
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum LogStructure {
    Debug,
    Pretty,
    Json,
}

impl FromStr for LogStructure {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match &s.to_ascii_lowercase()[..] {
            "json" => Ok(LogStructure::Json),
            "pretty" => Ok(LogStructure::Pretty),
            "debug" => Ok(LogStructure::Debug),
            _ => Err(anyhow!(
                "unknown log structure type: {:?} must be \"json\" or \"compact\" or \"pretty\"",
                s
            )),
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum Family {
    INET,
    INET6,
}

impl FromStr for Family {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match &s.to_ascii_lowercase()[..] {
            "inet" => Ok(Family::INET),
            "inet6" => Ok(Family::INET6),
            _ => Err(anyhow!(
                "unknown family type: {:?} must be \"inet\" or \"inet6\"",
                s
            )),
        }
    }
}
