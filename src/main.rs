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
use argh::FromArgs;
use crossbeam_channel::Receiver;
use mac_address::MacAddress;
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
    let mut args: Args = argh::from_env();

    // set default port if none provided
    if args.port.is_none() {
        if args.target.is_ipv6() {
            args.port = Some(546);
        } else {
            args.port = Some(67);
        }
    }

    if args.bind.is_none() {
        if args.target.is_ipv6() {
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
    let (sender, receiver) = crossbeam_channel::bounded(1);
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
                .with(fmt::layer().fmt_fields(Pretty::default()))
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

#[derive(Debug, FromArgs, Clone, PartialEq)]
#[argh(description = "dhmsg is a cli tool for sending dhcpv4/v6 messages

ex  dhcpv4:
        dhmsg 0.0.0.0 discover -p 9901  (unicast discover to 0.0.0.0:9901)
    dhcpv6:
        dhmsg ::0 solicit -p 9901       (unicast solicit to ::0:9901)")]
pub struct Args {
    /// IP address to send to
    #[argh(positional)]
    pub target: IpAddr,
    /// select a msg type (can't use solicit with v4, or discover with v6)
    #[argh(positional)]
    pub msg: MsgType,
    /// address to bind to
    #[argh(option, short = 'b')]
    pub bind: Option<IpAddr>,
    /// which port use. Default is 67 for dhcpv4 and 546 for dhcpv6
    #[argh(option, short = 'p')]
    pub port: Option<u16>,
    /// supply a mac address for DHCPv4
    #[argh(option, short = 'c', default = "get_mac()")]
    pub chaddr: MacAddress,
    /// query timeout in seconds. Default is 3.
    #[argh(option, short = 't', default = "default_timeout()")]
    pub timeout: u64,
    /// select the log output format
    #[argh(option, default = "LogStructure::Pretty")]
    pub output: LogStructure,
}

#[derive(PartialEq, Debug, Clone, Copy)]
pub enum MsgType {
    Discover,
    Request,
    Solicit,
}

impl FromStr for MsgType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "discover" => Ok(Self::Discover),
            "request" => Ok(Self::Request),
            "solicit" => Ok(Self::Request),
            _ => Err(anyhow!("unsupported message type")),
        }
    }
}

fn default_timeout() -> u64 {
    3
}

fn get_mac() -> MacAddress {
    mac_address::get_mac_address()
        .expect("unable to get MAC addr")
        .unwrap()
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
