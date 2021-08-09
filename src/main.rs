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
        dhmsg 0.0.0.0 -p 9901 discover  (unicast discover to 0.0.0.0:9901)
    dhcpv6:
        dhmsg ::0 -p 9901 solicit       (unicast solicit to ::0:9901)")]
pub struct Args {
    /// ip address to send to
    #[argh(positional)]
    pub target: IpAddr,
    /// select a msg type (can't use solicit with v4, or discover with v6)
    #[argh(subcommand)]
    pub msg: MsgType,
    /// address to bind to [default: INADDR_ANY]
    #[argh(option, short = 'b')]
    pub bind: Option<IpAddr>,
    /// request specific ip [default: None]
    #[argh(option, short = 'r')]
    pub req_addr: Option<IpAddr>,
    /// which port use. [default: 67 (v4) or 546 (v6)]
    #[argh(option, short = 'p')]
    pub port: Option<u16>,
    /// query timeout in seconds [default: 3]
    #[argh(option, short = 't', default = "default_timeout()")]
    pub timeout: u64,
    /// select the log output format
    #[argh(option, default = "LogStructure::Pretty")]
    pub output: LogStructure,
}

#[derive(PartialEq, Debug, Clone, Copy, FromArgs)]
#[argh(subcommand)]
pub enum MsgType {
    Discover(Discover),
    Request(Request),
    Solicit(Solicit),
}

#[derive(FromArgs, PartialEq, Debug, Clone, Copy)]
/// Send a DISCOVER msg
#[argh(subcommand, name = "discover")]
pub struct Discover {
    /// supply a mac address for DHCPv4 [default: first avail mac]
    #[argh(option, short = 'c', default = "get_mac()")]
    pub chaddr: MacAddress,
}

#[derive(FromArgs, PartialEq, Debug, Clone, Copy)]
/// Send a REQUEST msg
#[argh(subcommand, name = "request")]
pub struct Request {
    /// supply a mac address for DHCPv4 [default: first avail mac]
    #[argh(option, short = 'c', default = "get_mac()")]
    pub chaddr: MacAddress,
    /// address to bind to [default: INADDR_ANY]
    #[argh(option)]
    pub yiaddr: Option<IpAddr>,
}

#[derive(FromArgs, PartialEq, Debug, Clone, Copy)]
/// Send a SOLICIT msg
#[argh(subcommand, name = "solicit")]
pub struct Solicit {}

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
