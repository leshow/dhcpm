#![warn(
    missing_debug_implementations,
    // missing_docs, // TODO
    rust_2018_idioms,
    non_snake_case,
    non_upper_case_globals
)]
#![deny(broken_intra_doc_links)]
#![allow(clippy::cognitive_complexity)]

use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    str::FromStr,
};

use anyhow::{anyhow, Error, Result};
use clap::Clap;
use crossbeam_channel::Receiver;
use tracing::{error, info, trace};
use tracing_subscriber::{
    fmt::{self, format::Pretty},
    prelude::__tracing_subscriber_SubscriberExt,
    util::SubscriberInitExt,
    EnvFilter,
};

mod runner;
use runner::Runner;

fn main() -> Result<()> {
    let mut args = Args::parse();

    // set default port if none provided
    if args.port.is_none() {
        if args.ip.is_ipv6() {
            args.port = Some(546);
        } else {
            args.port = Some(67);
        }
    }

    if args.bind.is_none() {
        if args.ip.is_ipv6() {
            args.bind = Some(IpAddr::V6(Ipv6Addr::UNSPECIFIED));
        } else {
            args.bind = Some(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
        }
    }

    init_tracing(&args);
    trace!("{:?}", args);
    let shutdown_rx = ctrl_channel()?;
    let mut runner = Runner { args, shutdown_rx };
    if let Err(err) = runner.run() {
        error!(%err, "encountered error");
        return Err(err);
    }

    Ok(())
}

fn ctrl_channel() -> Result<Receiver<()>> {
    let (sender, receiver) = crossbeam_channel::bounded(10);
    ctrlc::set_handler(move || {
        let _ = sender.send(());
    })?;

    Ok(receiver)
}

fn init_tracing(args: &Args) {
    let filter_layer = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("info"))
        .unwrap();
    match args.output {
        LogStructure::Pretty => {
            tracing_subscriber::registry()
                .with(filter_layer)
                .with(
                    fmt::layer()
                        .fmt_fields(Pretty::with_source_location(Pretty::default(), false))
                        .with_target(false),
                )
                .init();
        }
        LogStructure::Debug => {
            tracing_subscriber::registry()
                .with(filter_layer)
                .with(fmt::layer())
                .init();
        }
        LogStructure::Json => {
            tracing_subscriber::registry()
                .with(filter_layer)
                .with(fmt::layer().json())
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
    /// address to bind to
    #[clap(long, short = 'b')]
    pub bind: Option<IpAddr>,
    /// which port use. Default is 67 for dhcpv4 and 546 for dhcpv6
    #[clap(long, short = 'p')]
    pub port: Option<u16>,
    /// query timeout in seconds. Default is 3.
    #[clap(long, short = 't', default_value = "3")]
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
