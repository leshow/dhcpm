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
    os::unix::prelude::{FromRawFd, IntoRawFd},
    sync::Arc,
    time::Instant,
};

#[cfg(feature = "script")]
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use argh::FromArgs;
use crossbeam_channel::{Receiver, Sender};
use dhcproto::{v4, v6};
use mac_address::MacAddress;
use opts::LogStructure;
use pnet_datalink::NetworkInterface;
use tracing::{error, info, trace};

mod decline;
mod discover;
mod inforeq;
mod inform;
mod opts;
mod release;
mod request;
mod runner;
#[cfg(feature = "script")]
mod script;

use opts::{parse_mac, parse_opts, parse_params};
use runner::TimeoutRunner;

use crate::{
    decline::DeclineArgs, discover::DiscoverArgs, inforeq::InformationReqArgs, inform::InformArgs,
    release::ReleaseArgs, request::RequestArgs, util::Msg,
};

#[allow(clippy::collapsible_else_if)]
fn main() -> Result<()> {
    let mut args: Args = argh::from_env();

    let mut default_port = false;
    // set default port to send if none provided
    if args.port.is_none() {
        default_port = true;
        if args.target.is_ipv6() {
            args.port = Some(v6::SERVER_PORT);
        } else {
            args.port = Some(v4::SERVER_PORT);
        }
    }

    if args.bind.is_none() {
        if args.target.is_ipv6() {
            if default_port {
                args.bind = Some(SocketAddr::new(
                    IpAddr::V6(Ipv6Addr::UNSPECIFIED),
                    v6::CLIENT_PORT,
                ));
            } else {
                args.bind = Some(SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0));
            }
        } else {
            if default_port {
                args.bind = Some(SocketAddr::new(
                    IpAddr::V4(Ipv4Addr::UNSPECIFIED),
                    v4::CLIENT_PORT,
                ));
            } else {
                args.bind = Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0));
            }
        }
    }

    opts::init_tracing(&args);
    trace!(?args);
    // let bind_addr = "[::0]:546".parse::<SocketAddr>().unwrap();
    // let socket = UdpSocket::bind("[::0]:0".parse::<SocketAddr>().unwrap()).context("a")?;
    // socket
    //     .join_multicast_v6(&"ff02::1:2".parse::<Ipv6Addr>().unwrap(), 0)
    //     .context("b")?;
    // let bind_addr: SocketAddr = args.bind.context("bind address must be specified")?;
    let interface = find_interface(&args.interface)?;
    info!(?interface);
    let bind_addr = args.bind.unwrap();
    let socket = socket2::Socket::new(
        if args.target.is_ipv6() {
            socket2::Domain::IPV6
        } else {
            socket2::Domain::IPV4
        },
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )?;
    if args.target.is_ipv6() {
        socket.set_only_v6(true).context("only ipv6")?;
        socket
            .set_reuse_address(true)
            .context("failed to set_reuse_address")?;
        socket
            .set_reuse_port(true)
            .context("failed to set_reuse_address")?;
    } else {
        socket.set_broadcast(true)?;
    }

    socket
        .bind(&bind_addr.into())
        .context("failed to bind addr")?;

    match interface {
        Some(int) => {
            socket
                .bind_device(Some(int.name.as_bytes()))
                .context("SO_BINDTODEVICE failed")?;
            if bind_addr.is_ipv6() {
                socket
                    .join_multicast_v6(&"ff02::1:2".parse::<Ipv6Addr>().unwrap(), int.index)
                    .context("join v6 multicast")?;
                socket
                    .set_multicast_if_v6(int.index)
                    .context("set multicast_if")?;
            }
        }
        None => {
            if bind_addr.is_ipv6() {
                bail!("an interface must be specified for ipv6");
            }
        }
    }
    let socket = {
        #[cfg(windows)]
        unsafe {
            UdpSocket::from_raw_socket(socket.into_raw_socket())?
        }
        #[cfg(unix)]
        unsafe {
            UdpSocket::from_raw_fd(socket.into_raw_fd())
        }
    };
    let soc = Arc::new(socket);

    let shutdown_rx = ctrl_channel()?;
    // messages put on `send_tx` will go out on the socket
    let (send_tx, send_rx) = crossbeam_channel::bounded(1);
    // messages coming from `recv_rx` were received from the socket
    let (recv_tx, recv_rx) = crossbeam_channel::bounded(1);

    runner::sender_thread(send_rx, soc.clone());
    runner::recv_thread(recv_tx, soc);

    let start = Instant::now();

    #[cfg(feature = "script")]
    if let Some(path) = &args.script {
        info!("evaluating rhai script");
        let mut args = args.clone();
        // TODO: fix retries for script
        args.no_retry = true;

        if let Err(err) = script::main(
            path,
            TimeoutRunner {
                args,
                shutdown_rx,
                send_tx,
                recv_rx,
            },
        ) {
            error!(?err, "error running rhai script");
        }
        info!(elapsed = %util::PrettyTime(start.elapsed()), "script completed");
        return Ok(());
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
    send_tx: Sender<(Msg, SocketAddr, bool)>,
    recv_rx: Receiver<(Msg, SocketAddr)>,
) -> Result<Msg> {
    let args = f();
    let runner = TimeoutRunner {
        args,
        shutdown_rx,
        send_tx,
        recv_rx,
    };
    match runner.send() {
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

#[derive(Debug, FromArgs, Clone, PartialEq, Eq)]
#[argh(description = "dhcpm is a cli tool for sending dhcpv4/v6 messages

ex  dhcpv4:
        dhcpm 255.255.255.255 discover          (broadcast discover to default dhcp port)
        dhcpm 192.168.0.255 discover            (broadcast discover on interface bound to 192.168.0.x)
        dhcpm 0.0.0.0 -p 9901 discover          (unicast discover to 0.0.0.0:9901)
        dhcpm 192.168.0.1 dora                  (unicast DORA to 192.168.0.1)
        dhcpm 192.168.0.1 dora -o 118,C0A80001  (unicast DORA, incl opt 118:192.168.0.1)
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
    /// interface to use (requires root or `cap_net_raw`) [default: None - selected by OS]
    #[argh(option, short = 'i')]
    pub interface: Option<String>,
    /// which port use. [default: 67 (v4) or 546 (v6)]
    #[argh(option, short = 'p')]
    pub port: Option<u16>,
    /// query timeout in seconds [default: 5]
    #[argh(option, short = 't', default = "opts::default_timeout()")]
    pub timeout: u64,
    /// select the log output format (json|pretty|debug) [default: pretty]
    #[argh(option, default = "LogStructure::Pretty")]
    pub output: LogStructure,
    /// pass in a path to a rhai script (https://github.com/rhaiscript/rhai)
    /// NOTE: must compile dhcpm with `script` feature
    #[cfg(feature = "script")]
    #[argh(option)]
    pub script: Option<PathBuf>,
    /// setting to "true" will prevent re-sending if we don't get a response [default: false]
    #[argh(option, default = "false")]
    pub no_retry: bool,
}

impl Args {
    pub fn get_target(&self) -> (SocketAddr, bool) {
        match self.target {
            IpAddr::V4(addr) => {
                let [_, _, _, brd] = addr.octets();
                if addr.is_broadcast() || brd == 255_u8 {
                    trace!("using broadcast address");
                    ((self.target, self.port.unwrap()).into(), true)
                } else {
                    ((self.target, self.port.unwrap()).into(), false)
                }
            }
            IpAddr::V6(addr) if addr.is_multicast() => ((addr, self.port.unwrap()).into(), true),
            IpAddr::V6(addr) => ((IpAddr::V6(addr), self.port.unwrap()).into(), false),
        }
    }
}

#[derive(PartialEq, Eq, Debug, Clone, FromArgs)]
#[argh(subcommand)]
pub enum MsgType {
    Discover(DiscoverArgs),
    Request(RequestArgs),
    Release(ReleaseArgs),
    Inform(InformArgs),
    Decline(DeclineArgs),
    Dora(DoraArgs),
    Solicit(SolicitArgs),
    InformationReq(InformationReqArgs),
}

#[derive(FromArgs, PartialEq, Eq, Debug, Clone)]
/// Sends Discover then Request
#[argh(subcommand, name = "dora")]
pub struct DoraArgs {
    /// supply a mac address for DHCPv4 (use "random" for a random mac) [default: first interface mac]
    #[argh(
        option,
        short = 'c',
        from_str_fn(parse_mac),
        default = "opts::get_mac()"
    )]
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

#[derive(FromArgs, PartialEq, Eq, Debug, Clone, Copy)]
/// Send a SOLICIT msg (dhcpv6)
#[argh(subcommand, name = "solicit")]
pub struct SolicitArgs {}

pub mod util {
    use std::{fmt, time::Duration};

    use anyhow::Result;
    use dhcproto::{v4, v6, Encodable};

    #[derive(Clone, PartialEq, Eq)]
    pub enum Msg {
        V4(v4::Message),
        V6(v6::Message),
    }

    impl Msg {
        pub fn get_type(&self) -> String {
            match self {
                Msg::V4(m) => format!("{:?}", m.opts().msg_type().unwrap()),
                Msg::V6(m) => format!("{:?}", m.opts()),
            }
        }
        #[cfg(feature = "script")]
        pub fn unwrap_v4(self) -> v4::Message {
            match self {
                Msg::V4(m) => m,
                _ => panic!("unwrapped wrong variant on message"),
            }
        }
        // pub fn unwrap_v6(self) -> v6::Message {
        //     match self {
        //         Msg::V6(m) => m,
        //         _ => panic!("unwrapped wrong variant on message"),
        //     }
        // }
        pub fn to_vec(&self) -> Result<Vec<u8>> {
            Ok(match self {
                Msg::V4(m) => m.to_vec()?,
                Msg::V6(m) => m.to_vec()?,
            })
        }
    }

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

    impl fmt::Debug for Msg {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Msg::V4(msg) => f
                    .debug_struct("v4::Message")
                    .field("xid", &msg.xid())
                    .field("secs", &msg.secs())
                    .field("broadcast_flag", &msg.flags().broadcast())
                    .field("ciaddr", &msg.ciaddr())
                    .field("yiaddr", &msg.yiaddr())
                    .field("siaddr", &msg.siaddr())
                    .field("giaddr", &msg.giaddr())
                    .field(
                        "chaddr",
                        &hex::encode(msg.chaddr())
                            .chars()
                            .enumerate()
                            .flat_map(|(i, c)| {
                                if i != 0 && i % 2 == 0 {
                                    Some(':')
                                } else {
                                    None
                                }
                                .into_iter()
                                .chain(std::iter::once(c))
                            })
                            .collect::<String>(),
                    )
                    .field(
                        "opts",
                        &msg.opts().iter().map(|(_, v)| v).collect::<Vec<_>>(),
                    )
                    .finish(),
                Msg::V6(msg) => f
                    .debug_struct("v6::Message")
                    .field("xid", &msg.xid_num())
                    .field("opts", &msg.opts())
                    .finish(),
            }
        }
    }
}

/// Returns:
/// - interfaces matching the list supplied that are 'up' and have an IPv6
/// - OR any 'up' interfaces that also have an IPv6
pub fn find_interface(interface: &Option<String>) -> Result<Option<NetworkInterface>> {
    let found_interfaces = pnet_datalink::interfaces()
        .into_iter()
        .filter(|e| e.is_up() && !e.ips.is_empty())
        .collect::<Vec<_>>();
    match interface {
        Some(interface) => match found_interfaces.iter().find(|i| &i.name == interface) {
            Some(i) => Ok(Some(i.clone())),
            None => bail!("unable to find interface {}", interface),
        },
        None => Ok(None),
    }
}
