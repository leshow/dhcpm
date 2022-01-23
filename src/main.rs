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
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    str::FromStr,
};

use anyhow::{anyhow, Error, Result};
use argh::FromArgs;
use crossbeam_channel::Receiver;
use dhcproto::v4;
use mac_address::MacAddress;
use tracing::{error, trace};
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

    let mut default_port = false;
    // set default port if none provided
    if args.port.is_none() {
        default_port = true;
        if args.target.is_ipv6() {
            args.port = Some(546);
        } else {
            args.port = Some(67);
        }
    }

    if args.bind.is_none() {
        if args.target.is_ipv6() {
            if default_port {
                args.bind = Some(SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 547));
            } else {
                args.bind = Some(SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0));
            }
        } else {
            if default_port {
                args.bind = Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 68));
            } else {
                args.bind = Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0));
            }
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
#[argh(description = "dhcpm is a cli tool for sending dhcpv4/v6 messages

ex  dhcpv4:
        dhcpm 0.0.0.0 -p 9901 discover  (unicast discover to 0.0.0.0:9901)
        dhcpm 255.255.255.255 discover (broadcast discover to default dhcp port)
    dhcpv6:
        dhcpm ::0 -p 9901 solicit       (unicast solicit to [::0]:9901)
        dhcpm ff02::1:2 solicit         (multicast solicit to default port)
        ")]
pub struct Args {
    /// ip address to send to
    #[argh(positional)]
    pub target: IpAddr,
    /// select a msg type (can't use solicit with v4, or discover with v6)
    #[argh(subcommand)]
    pub msg: MsgType,
    /// address to bind to [default: INADDR_ANY:0]
    #[argh(option, short = 'b')]
    pub bind: Option<SocketAddr>,
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
    Discover(DiscoverArgs),
    Request(RequestArgs),
    Release(ReleaseArgs),
    Solicit(SolicitArgs),
}

#[derive(FromArgs, PartialEq, Debug, Clone, Copy)]
/// Send a DISCOVER msg
#[argh(subcommand, name = "discover")]
pub struct DiscoverArgs {
    /// supply a mac address for DHCPv4 [default: first avail mac]
    #[argh(option, short = 'c', default = "get_mac()")]
    pub chaddr: MacAddress,
    /// request specific ip [default: None]
    #[argh(option, short = 'r')]
    pub req_addr: Option<Ipv4Addr>,
    /// giaddr [default: 0.0.0.0]
    #[argh(option, short = 'g', default = "Ipv4Addr::UNSPECIFIED")]
    pub giaddr: Ipv4Addr,
    /// subnet selection opt 118 [default: None]
    #[argh(option)]
    pub subnet_select: Option<Ipv4Addr>,
    /// relay link select opt 82 subopt 5 [default: None]
    #[argh(option)]
    pub relay_link: Option<Ipv4Addr>,
}

impl DiscoverArgs {
    fn build(self, broadcast: bool) -> v4::Message {
        let mut msg = v4::Message::new(
            Ipv4Addr::UNSPECIFIED,
            Ipv4Addr::UNSPECIFIED,
            Ipv4Addr::UNSPECIFIED,
            self.giaddr,
            &self.chaddr.bytes(),
        );

        if broadcast {
            msg.set_flags(v4::Flags::default().set_broadcast());
        }
        msg.opts_mut()
            .insert(v4::DhcpOption::MessageType(v4::MessageType::Discover));
        msg.opts_mut().insert(v4::DhcpOption::ClientIdentifier(
            self.chaddr.bytes().to_vec(),
        ));
        msg.opts_mut()
            .insert(v4::DhcpOption::ParameterRequestList(vec![
                v4::OptionCode::SubnetMask,
                v4::OptionCode::Router,
                v4::OptionCode::DomainNameServer,
                v4::OptionCode::DomainName,
            ]));
        // TODO: add more?
        // add requested ip
        if let Some(ip) = self.req_addr {
            msg.opts_mut()
                .insert(v4::DhcpOption::RequestedIpAddress(ip));
        }
        if let Some(ip) = self.relay_link {
            let mut info = v4::relay::RelayAgentInformation::default();
            info.insert(v4::relay::RelayInfo::LinkSelection(ip));
            msg.opts_mut()
                .insert(v4::DhcpOption::RelayAgentInformation(info));
        }

        if let Some(ip) = self.subnet_select {
            msg.opts_mut().insert(v4::DhcpOption::SubnetSelection(ip));
        }
        msg
    }
}

#[derive(FromArgs, PartialEq, Debug, Clone, Copy)]
/// Send a REQUEST msg
#[argh(subcommand, name = "request")]
pub struct RequestArgs {
    /// supply a mac address for DHCPv4 [default: first avail mac]
    #[argh(option, short = 'c', default = "get_mac()")]
    pub chaddr: MacAddress,
    /// address for client [default: None]
    #[argh(option, short = 'y', default = "Ipv4Addr::UNSPECIFIED")]
    pub yiaddr: Ipv4Addr,
    /// server identifier [default: None]
    #[argh(option, short = 's')]
    pub sident: Option<Ipv4Addr>,
    /// specify dhcp option for requesting ip [default: None]
    #[argh(option, short = 'r')]
    pub req_addr: Option<Ipv4Addr>,
    /// giaddr [default: 0.0.0.0]
    #[argh(option, short = 'g', default = "Ipv4Addr::UNSPECIFIED")]
    pub giaddr: Ipv4Addr,
    /// subnet selection opt 118 [default: None]
    #[argh(option)]
    pub subnet_select: Option<Ipv4Addr>,
    /// relay link select opt 82 subopt 5 [default: None]
    #[argh(option)]
    pub relay_link: Option<Ipv4Addr>,
}

impl RequestArgs {
    fn build(self) -> v4::Message {
        let mut msg = v4::Message::new(
            Ipv4Addr::UNSPECIFIED,
            self.yiaddr,
            Ipv4Addr::UNSPECIFIED,
            self.giaddr,
            &self.chaddr.bytes(),
        );

        msg.opts_mut()
            .insert(v4::DhcpOption::MessageType(v4::MessageType::Request));
        msg.opts_mut().insert(v4::DhcpOption::ClientIdentifier(
            self.chaddr.bytes().to_vec(),
        ));
        msg.opts_mut()
            .insert(v4::DhcpOption::ParameterRequestList(vec![
                v4::OptionCode::SubnetMask,
                v4::OptionCode::Router,
                v4::OptionCode::DomainNameServer,
                v4::OptionCode::DomainName,
            ]));

        if let Some(ip) = self.sident {
            msg.opts_mut().insert(v4::DhcpOption::ServerIdentifier(ip));
        }

        if let Some(ip) = self.req_addr {
            msg.opts_mut()
                .insert(v4::DhcpOption::RequestedIpAddress(ip));
        }
        if let Some(ip) = self.relay_link {
            let mut info = v4::relay::RelayAgentInformation::default();
            info.insert(v4::relay::RelayInfo::LinkSelection(ip));
            msg.opts_mut()
                .insert(v4::DhcpOption::RelayAgentInformation(info));
        }

        if let Some(ip) = self.subnet_select {
            msg.opts_mut().insert(v4::DhcpOption::SubnetSelection(ip));
        }
        msg
    }
}

#[derive(FromArgs, PartialEq, Debug, Clone, Copy)]
/// Send a RELEASE msg
#[argh(subcommand, name = "release")]
pub struct ReleaseArgs {
    /// supply a mac address for DHCPv4 [default: first avail mac]
    #[argh(option, short = 'c', default = "get_mac()")]
    pub chaddr: MacAddress,
    /// address for client [default: None]
    #[argh(option, short = 'y', default = "Ipv4Addr::UNSPECIFIED")]
    pub yiaddr: Ipv4Addr,
    /// server identifier [default: None]
    #[argh(option, short = 's')]
    pub sident: Option<Ipv4Addr>,
    /// giaddr [default: 0.0.0.0]
    #[argh(option, short = 'g', default = "Ipv4Addr::UNSPECIFIED")]
    pub giaddr: Ipv4Addr,
    /// subnet selection opt 118 [default: None]
    #[argh(option)]
    pub subnet_select: Option<Ipv4Addr>,
    /// relay link select opt 82 subopt 5 [default: None]
    #[argh(option)]
    pub relay_link: Option<Ipv4Addr>,
}

impl ReleaseArgs {
    fn build(self) -> v4::Message {
        let mut msg = v4::Message::new(
            Ipv4Addr::UNSPECIFIED,
            self.yiaddr,
            Ipv4Addr::UNSPECIFIED,
            self.giaddr,
            &self.chaddr.bytes(),
        );

        msg.opts_mut()
            .insert(v4::DhcpOption::MessageType(v4::MessageType::Request));
        msg.opts_mut().insert(v4::DhcpOption::ClientIdentifier(
            self.chaddr.bytes().to_vec(),
        ));
        msg.opts_mut()
            .insert(v4::DhcpOption::ParameterRequestList(vec![
                v4::OptionCode::SubnetMask,
                v4::OptionCode::Router,
                v4::OptionCode::DomainNameServer,
                v4::OptionCode::DomainName,
            ]));

        if let Some(ip) = self.sident {
            msg.opts_mut().insert(v4::DhcpOption::ServerIdentifier(ip));
        }

        if let Some(ip) = self.relay_link {
            let mut info = v4::relay::RelayAgentInformation::default();
            info.insert(v4::relay::RelayInfo::LinkSelection(ip));
            msg.opts_mut()
                .insert(v4::DhcpOption::RelayAgentInformation(info));
        }

        if let Some(ip) = self.subnet_select {
            msg.opts_mut().insert(v4::DhcpOption::SubnetSelection(ip));
        }
        msg
    }
}

#[derive(FromArgs, PartialEq, Debug, Clone, Copy)]
/// Send a SOLICIT msg
#[argh(subcommand, name = "solicit")]
pub struct SolicitArgs {}

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
