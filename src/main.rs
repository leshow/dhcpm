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
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket},
    path::PathBuf,
    sync::Arc,
    time::Instant,
};

use anyhow::Result;
use argh::FromArgs;
use crossbeam_channel::{Receiver, Sender};
use dhcproto::v4;
use mac_address::MacAddress;
use opts::LogStructure;
use tracing::{error, info, trace};

mod opts;
mod runner;
#[cfg(feature = "rhai")]
mod script;
use opts::{parse_opts, parse_params};
use runner::Runner;

use crate::runner::Msg;

#[allow(clippy::collapsible_else_if)]
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

    opts::init_tracing(&args);
    trace!(?args);

    let shutdown_rx = ctrl_channel()?;
    let bind_addr: SocketAddr = args.bind.unwrap();
    let soc = Arc::new(UdpSocket::bind(bind_addr)?);

    // messages put on `send_tx` will go out on the socket
    let (send_tx, send_rx) = crossbeam_channel::bounded(1);
    // messages put on `recv_tx` were received from the socket
    let (recv_tx, recv_rx) = crossbeam_channel::bounded(1);

    runner::sender_thread(send_rx, soc.clone());
    runner::recv_thread(recv_tx, soc);

    let start = Instant::now();
    if let Some(path) = &args.script {
        #[cfg(not(feature = "rhai"))]
        return Err(anyhow::anyhow!(
            "dhcpm must be compiled with \"rhai\" feature to use the script flag"
        ));

        #[cfg(feature = "rhai")]
        {
            info!("evaluating rhai script");
            if let Err(err) = script::main(path) {
                error!(?err, "error running rhai script");
            }
            info!(elapsed = %util::PrettyTime(start.elapsed()), "script completed");
            return Ok(());
        }
    }

    // clone new args so we still have the original in case we need to
    // do a request after
    let mut new_args = args.clone();
    let msg = run_it(
        // just a bit of a hack to change the message type to discover
        move || match &new_args.msg {
            Some(MsgType::Dora(dora)) => {
                new_args.msg = Some(MsgType::Discover(dora.discover()));
                new_args
            }
            _ => new_args,
        },
        shutdown_rx.clone(),
        send_tx.clone(),
        recv_rx.clone(),
    )?;

    // then to request for the next run
    let new_args = match (&args.msg, msg) {
        (Some(MsgType::Dora(dora)), Msg::V4(msg)) => {
            let mut new_args = args.clone();
            new_args.msg = Some(MsgType::Request(dora.request(msg.yiaddr())));
            new_args
        }
        // exit if we were just meant to send 1 message
        _ => {
            drop(send_tx);
            drop(recv_rx);
            return Ok(());
        }
    };
    run_it(move || new_args, shutdown_rx, send_tx, recv_rx)?;

    info!(elapsed = %util::PrettyTime(start.elapsed()), "total time");

    Ok(())
}

fn run_it<F: FnOnce() -> Args>(
    f: F,
    shutdown_rx: Receiver<()>,
    send_tx: Sender<(Msg, SocketAddr)>,
    recv_rx: Receiver<(Msg, SocketAddr)>,
) -> Result<Msg> {
    let args = f();
    let runner = Runner {
        args,
        shutdown_rx,
        send_tx,
        recv_rx,
    };
    match runner.run() {
        Err(err) => {
            error!(%err, "got an error");
            Err(err)
        }
        Ok(msg) => Ok(msg),
    }
}

fn ctrl_channel() -> Result<Receiver<()>> {
    let (sender, receiver) = crossbeam_channel::bounded(1);
    ctrlc::set_handler(move || {
        let _ = sender.send(());
    })?;

    Ok(receiver)
}

#[derive(Debug, FromArgs, Clone, PartialEq)]
#[argh(description = "dhcpm is a cli tool for sending dhcpv4/v6 messages

ex  dhcpv4:
        dhcpm 0.0.0.0 -p 9901 discover  (unicast discover to 0.0.0.0:9901)
        dhcpm 255.255.255.255 discover (broadcast discover to default dhcp port)
        dhcpm 192.168.0.1 dora (unicast DORA to 192.168.0.1)
        dhcpm 192.168.0.1 dora -o 118,C0A80001 (unicast DORA, incl opt 118:192.168.0.1)
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
    pub msg: Option<MsgType>,
    /// address to bind to [default: INADDR_ANY:0]
    #[argh(option, short = 'b')]
    pub bind: Option<SocketAddr>,
    /// which port use. [default: 67 (v4) or 546 (v6)]
    #[argh(option, short = 'p')]
    pub port: Option<u16>,
    /// query timeout in seconds [default: 3]
    #[argh(option, short = 't', default = "opts::default_timeout()")]
    pub timeout: u64,
    /// select the log output format
    #[argh(option, default = "LogStructure::Pretty")]
    pub output: LogStructure,
    /// pass in a path to a rhai script (https://github.com/rhaiscript/rhai)
    /// NOTE: must compile dhcpm with `rhai` feature
    #[argh(option)]
    pub script: Option<PathBuf>,
}

impl Args {
    pub fn get_target(&self) -> (SocketAddr, bool) {
        match self.target {
            IpAddr::V4(addr) if addr.is_broadcast() => {
                ((self.target, self.port.unwrap()).into(), true)
            }
            IpAddr::V4(addr) => ((addr, self.port.unwrap()).into(), false),
            IpAddr::V6(addr) if addr.is_multicast() => ((addr, self.port.unwrap()).into(), false),
            IpAddr::V6(addr) => ((IpAddr::V6(addr), self.port.unwrap()).into(), false),
        }
    }
}

#[derive(PartialEq, Debug, Clone, FromArgs)]
#[argh(subcommand)]
pub enum MsgType {
    Discover(DiscoverArgs),
    Request(RequestArgs),
    Release(ReleaseArgs),
    Inform(InformArgs),
    Dora(DoraArgs),
    Solicit(SolicitArgs),
}

#[derive(FromArgs, PartialEq, Debug, Clone)]
/// Send a DISCOVER msg
#[argh(subcommand, name = "discover")]
pub struct DiscoverArgs {
    /// supply a mac address for DHCPv4 [default: first avail mac]
    #[argh(option, short = 'c', default = "opts::get_mac()")]
    pub chaddr: MacAddress,
    /// address of client [default: None]
    #[argh(option, default = "Ipv4Addr::UNSPECIFIED")]
    pub ciaddr: Ipv4Addr,
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
    /// add opts to the message
    /// [ex: these are equivalent- "118,hex,C0A80001" or "118,ip,192.168.0.1"]
    #[argh(option, short = 'o', from_str_fn(parse_opts))]
    pub opt: Vec<v4::DhcpOption>,
    /// params to include: [default: 1,3,6,15 (Subnet, Router, DnsServer, DomainName]
    #[argh(option, from_str_fn(parse_params), default = "opts::default_params()")]
    pub params: Vec<v4::OptionCode>,
}

impl Default for DiscoverArgs {
    fn default() -> Self {
        Self {
            chaddr: opts::get_mac(),
            ciaddr: Ipv4Addr::UNSPECIFIED,
            giaddr: Ipv4Addr::UNSPECIFIED,
            req_addr: None,
            subnet_select: None,
            relay_link: None,
            opt: Vec::new(),
            params: opts::default_params(),
        }
    }
}

impl DiscoverArgs {
    fn build(&self, broadcast: bool) -> v4::Message {
        let mut msg = v4::Message::new(
            self.ciaddr,
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
        // insert parse params
        msg.opts_mut()
            .insert(v4::DhcpOption::ParameterRequestList(self.params.clone()));
        // insert manually entered opts
        for opt in &self.opt {
            msg.opts_mut().insert(opt.clone());
        }
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

#[derive(FromArgs, PartialEq, Debug, Clone)]
/// Send a REQUEST msg
#[argh(subcommand, name = "request")]
pub struct RequestArgs {
    /// supply a mac address for DHCPv4 [default: first avail mac]
    #[argh(option, short = 'c', default = "opts::get_mac()")]
    pub chaddr: MacAddress,
    /// address for client [default: None]
    #[argh(option, short = 'y', default = "Ipv4Addr::UNSPECIFIED")]
    pub yiaddr: Ipv4Addr,
    /// address of client [default: None]
    #[argh(option, default = "Ipv4Addr::UNSPECIFIED")]
    pub ciaddr: Ipv4Addr,
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
    /// add opts to the message
    /// [ex: these are equivalent- "118,hex,C0A80001" or "118,ip,192.168.0.1"]
    #[argh(option, short = 'o', from_str_fn(parse_opts))]
    pub opt: Vec<v4::DhcpOption>,
    /// params to include: [default: 1,3,6,15 (Subnet, Router, DnsServer, DomainName]
    #[argh(option, from_str_fn(parse_params), default = "opts::default_params()")]
    pub params: Vec<v4::OptionCode>,
}

impl RequestArgs {
    fn build(&self, broadcast: bool) -> v4::Message {
        let mut msg = v4::Message::new(
            self.ciaddr,
            self.yiaddr,
            Ipv4Addr::UNSPECIFIED,
            self.giaddr,
            &self.chaddr.bytes(),
        );

        if broadcast {
            msg.set_flags(v4::Flags::default().set_broadcast());
        }

        msg.opts_mut()
            .insert(v4::DhcpOption::MessageType(v4::MessageType::Request));
        msg.opts_mut().insert(v4::DhcpOption::ClientIdentifier(
            self.chaddr.bytes().to_vec(),
        ));
        // insert parse params
        msg.opts_mut()
            .insert(v4::DhcpOption::ParameterRequestList(self.params.clone()));
        // insert manually entered opts
        for opt in &self.opt {
            msg.opts_mut().insert(opt.clone());
        }
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

#[derive(FromArgs, PartialEq, Debug, Clone)]
/// Send a RELEASE msg
#[argh(subcommand, name = "release")]
pub struct ReleaseArgs {
    /// supply a mac address for DHCPv4 [default: first avail mac]
    #[argh(option, short = 'c', default = "opts::get_mac()")]
    pub chaddr: MacAddress,
    /// giaddr [default: 0.0.0.0]
    #[argh(option, short = 'g', default = "Ipv4Addr::UNSPECIFIED")]
    pub giaddr: Ipv4Addr,
    /// address of client [default: None]
    #[argh(option, default = "Ipv4Addr::UNSPECIFIED")]
    pub ciaddr: Ipv4Addr,
    /// yiaddr [default: None]
    #[argh(option, short = 'y', default = "Ipv4Addr::UNSPECIFIED")]
    pub yiaddr: Ipv4Addr,
    /// server identifier [default: None]
    #[argh(option, short = 's')]
    pub sident: Option<Ipv4Addr>,
    /// subnet selection opt 118 [default: None]
    #[argh(option)]
    pub subnet_select: Option<Ipv4Addr>,
    /// relay link select opt 82 subopt 5 [default: None]
    #[argh(option)]
    pub relay_link: Option<Ipv4Addr>,
    /// add opts to the message
    /// [ex: these are equivalent- "118,hex,C0A80001" or "118,ip,192.168.0.1"]
    #[argh(option, short = 'o', from_str_fn(parse_opts))]
    pub opt: Vec<v4::DhcpOption>,
    /// params to include: [default: 1,3,6,15 (Subnet, Router, DnsServer, DomainName]
    #[argh(option, from_str_fn(parse_params), default = "opts::default_params()")]
    pub params: Vec<v4::OptionCode>,
}

impl ReleaseArgs {
    fn build(&self) -> v4::Message {
        let mut msg = v4::Message::new(
            self.ciaddr,
            self.yiaddr,
            Ipv4Addr::UNSPECIFIED,
            self.giaddr,
            &self.chaddr.bytes(),
        );

        msg.opts_mut()
            .insert(v4::DhcpOption::MessageType(v4::MessageType::Release));
        msg.opts_mut().insert(v4::DhcpOption::ClientIdentifier(
            self.chaddr.bytes().to_vec(),
        ));
        msg.opts_mut()
            .insert(v4::DhcpOption::ParameterRequestList(self.params.clone()));
        // insert manually entered opts
        for opt in &self.opt {
            msg.opts_mut().insert(opt.clone());
        }

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

#[derive(FromArgs, PartialEq, Debug, Clone)]
/// Send an INFORM msg
#[argh(subcommand, name = "inform")]
pub struct InformArgs {
    /// supply a mac address for DHCPv4 [default: first avail mac]
    #[argh(option, short = 'c', default = "opts::get_mac()")]
    pub chaddr: MacAddress,
    /// address for client [default: 0.0.0.0]
    #[argh(option, short = 'y', default = "Ipv4Addr::UNSPECIFIED")]
    pub yiaddr: Ipv4Addr,
    /// address of client [default: 0.0.0.0]
    #[argh(option, default = "Ipv4Addr::UNSPECIFIED")]
    pub ciaddr: Ipv4Addr,
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
    /// add opts to the message
    /// [ex: these are equivalent- "118,hex,C0A80001" or "118,ip,192.168.0.1"]
    #[argh(option, short = 'o', from_str_fn(parse_opts))]
    pub opt: Vec<v4::DhcpOption>,
    /// params to include: [default: 1,3,6,15 (Subnet, Router, DnsServer, DomainName]
    #[argh(option, from_str_fn(parse_params), default = "opts::default_params()")]
    pub params: Vec<v4::OptionCode>,
}

impl InformArgs {
    fn build(&self) -> v4::Message {
        let mut msg = v4::Message::new(
            self.ciaddr,
            self.yiaddr,
            Ipv4Addr::UNSPECIFIED,
            self.giaddr,
            &self.chaddr.bytes(),
        );

        msg.opts_mut()
            .insert(v4::DhcpOption::MessageType(v4::MessageType::Inform));
        msg.opts_mut().insert(v4::DhcpOption::ClientIdentifier(
            self.chaddr.bytes().to_vec(),
        ));
        msg.opts_mut()
            .insert(v4::DhcpOption::ParameterRequestList(self.params.clone()));
        // insert manually entered opts
        for opt in &self.opt {
            msg.opts_mut().insert(opt.clone());
        }
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

#[derive(FromArgs, PartialEq, Debug, Clone)]
/// Sends Discover then Request
#[argh(subcommand, name = "dora")]
pub struct DoraArgs {
    /// supply a mac address for DHCPv4 [default: first avail mac]
    #[argh(option, short = 'c', default = "opts::get_mac()")]
    pub chaddr: MacAddress,
    /// address of client [default: None]
    #[argh(option, default = "Ipv4Addr::UNSPECIFIED")]
    pub ciaddr: Ipv4Addr,
    /// address for client [default: 0.0.0.0]
    #[argh(option, short = 'y', default = "Ipv4Addr::UNSPECIFIED")]
    pub yiaddr: Ipv4Addr,
    /// server identifier [default: None]
    #[argh(option, short = 's')]
    pub sident: Option<Ipv4Addr>,
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
    /// add opts to the message
    /// [ex: these are equivalent- "118,hex,C0A80001" or "118,ip,192.168.0.1"]
    #[argh(option, short = 'o', from_str_fn(parse_opts))]
    pub opt: Vec<v4::DhcpOption>,
    /// params to include: [default: 1,3,6,15 (Subnet, Router, DnsServer, DomainName]
    #[argh(option, from_str_fn(parse_params), default = "opts::default_params()")]
    pub params: Vec<v4::OptionCode>,
}

impl DoraArgs {
    pub fn discover(&self) -> DiscoverArgs {
        DiscoverArgs {
            chaddr: self.chaddr,
            ciaddr: self.ciaddr,
            req_addr: self.req_addr,
            giaddr: self.giaddr,
            subnet_select: self.subnet_select,
            relay_link: self.relay_link,
            opt: self.opt.clone(),
            params: self.params.clone(),
        }
    }
    pub fn request(&self, req_addr: Ipv4Addr) -> RequestArgs {
        RequestArgs {
            chaddr: self.chaddr,
            ciaddr: self.ciaddr,
            yiaddr: self.yiaddr,
            // insert the IP we got back in OFFER
            req_addr: Some(req_addr),
            sident: self.sident,
            giaddr: self.giaddr,
            subnet_select: self.subnet_select,
            relay_link: self.relay_link,
            opt: self.opt.clone(),
            params: self.params.clone(),
        }
    }
}

#[derive(FromArgs, PartialEq, Debug, Clone, Copy)]
/// Send a SOLICIT msg (dhcpv6)
#[argh(subcommand, name = "solicit")]
pub struct SolicitArgs {}

pub mod util {
    use std::{fmt, time::Duration};

    #[derive(Copy, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
    pub struct PrettyTime(pub Duration);

    impl fmt::Display for PrettyTime {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            let secs = self.0.as_secs_f32().to_string();
            write!(f, "{}s", if secs.len() <= 5 { &secs } else { &secs[0..=5] })
        }
    }

    impl fmt::Debug for PrettyTime {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{:?}", self.0)
        }
    }

    #[derive(Copy, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
    pub struct PrettyPrint<T>(pub T);

    impl<T: fmt::Debug> fmt::Display for PrettyPrint<T> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{:#?}", &self.0)
        }
    }

    impl<T: fmt::Debug> fmt::Debug for PrettyPrint<T> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{:?}", self.0)
        }
    }
}
