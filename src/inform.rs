use std::net::Ipv4Addr;

use argh::FromArgs;
use dhcproto::v4;
use mac_address::MacAddress;

use crate::opts::{self, parse_mac, parse_opts, parse_params};

#[derive(FromArgs, PartialEq, Eq, Debug, Clone)]
/// Send an INFORM msg
#[argh(subcommand, name = "inform")]
pub struct InformArgs {
    /// supply a mac address for DHCPv4 (use "random" for a random mac) [default: first interface mac]
    #[argh(
        option,
        short = 'c',
        from_str_fn(parse_mac),
        default = "opts::get_mac()"
    )]
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

impl Default for InformArgs {
    fn default() -> Self {
        Self {
            chaddr: opts::get_mac(),
            ciaddr: Ipv4Addr::UNSPECIFIED,
            giaddr: Ipv4Addr::UNSPECIFIED,
            yiaddr: Ipv4Addr::UNSPECIFIED,
            sident: None,
            subnet_select: None,
            relay_link: None,
            opt: Vec::new(),
            params: opts::default_params(),
        }
    }
}

impl InformArgs {
    pub fn build(&self) -> v4::Message {
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

#[cfg(feature = "script")]
use rhai::{plugin::*, EvalAltResult};

// exposing InformArgs
#[cfg(feature = "script")]
#[export_module]
pub mod inform_mod {
    use tracing::trace;
    #[rhai_fn()]
    pub fn args_default() -> InformArgs {
        InformArgs::default()
    }
    #[rhai_fn(global, name = "to_string", name = "to_debug", pure)]
    pub fn to_string(args: &mut InformArgs) -> String {
        format!("{:?}", args)
    }
    // ciaddr
    #[rhai_fn(global, get = "ciaddr", pure)]
    pub fn get_ciaddr(args: &mut InformArgs) -> String {
        args.ciaddr.to_string()
    }
    #[rhai_fn(global, set = "ciaddr")]
    pub fn set_ciaddr(args: &mut InformArgs, ciaddr: &str) {
        trace!(?ciaddr, "setting ciaddr");
        args.ciaddr = ciaddr.parse::<Ipv4Addr>().expect("failed to parse ciaddr");
    }
    // yiaddr
    #[rhai_fn(global, get = "yiaddr", pure)]
    pub fn get_yiaddr(args: &mut InformArgs) -> String {
        args.yiaddr.to_string()
    }
    #[rhai_fn(global, set = "yiaddr")]
    pub fn set_yiaddr(args: &mut InformArgs, yiaddr: &str) {
        trace!(?yiaddr, "setting yiaddr");
        args.yiaddr = yiaddr.parse::<Ipv4Addr>().expect("failed to parse ciaddr");
    }
    // giaddr
    #[rhai_fn(global, get = "giaddr", pure)]
    pub fn get_giaddr(args: &mut InformArgs) -> String {
        args.giaddr.to_string()
    }
    #[rhai_fn(global, set = "giaddr")]
    pub fn set_giaddr(args: &mut InformArgs, giaddr: &str) {
        trace!(?giaddr, "setting giaddr");
        args.giaddr = giaddr.parse::<Ipv4Addr>().expect("failed to parse giaddr");
    }
    // relay_link
    #[rhai_fn(global, get = "relay_link", pure)]
    pub fn get_relay_link(args: &mut InformArgs) -> Option<String> {
        args.relay_link.map(|r| r.to_string())
    }
    #[rhai_fn(global, set = "relay_link")]
    pub fn set_relay_link(args: &mut InformArgs, relay_link: &str) {
        trace!(?relay_link, "setting relay_link");
        args.relay_link = Some(
            relay_link
                .parse::<Ipv4Addr>()
                .expect("failed to parse relay_link"),
        );
    }
    // chaddr
    #[rhai_fn(global, get = "chaddr", pure)]
    pub fn get_chaddr(args: &mut InformArgs) -> rhai::Blob {
        args.chaddr.bytes().to_vec()
    }
    #[rhai_fn(global, set = "chaddr")]
    pub fn set_chaddr(args: &mut InformArgs, chaddr: rhai::Blob) {
        trace!(?chaddr, "setting chaddr");
        let bytes: [u8; 6] = chaddr.try_into().expect("failed to convert macaddress");
        args.chaddr = bytes.into();
    }
    #[rhai_fn(global, name = "rand_chaddr")]
    pub fn rand_chaddr(args: &mut InformArgs) {
        let chaddr = rand::random::<[u8; 6]>().into();
        trace!(?chaddr, "setting random chaddr");
        args.chaddr = chaddr;
    }
    // sident
    #[rhai_fn(global, get = "sident", pure)]
    pub fn get_sident(args: &mut InformArgs) -> Option<String> {
        args.sident.map(|r| r.to_string())
    }
    #[rhai_fn(global, set = "sident")]
    pub fn set_sident(args: &mut InformArgs, sident: &str) {
        trace!(?sident, "setting req_addr");
        args.sident = Some(sident.parse::<Ipv4Addr>().expect("failed to parse sident"));
    }
    // opt
    #[rhai_fn(global, set = "opt")]
    pub fn set_opt(args: &mut InformArgs, opt: String) {
        trace!(?opt, "adding opt to message");
        args.opt
            .push(crate::opts::parse_opts(&opt).expect("failed to parse opt"));
    }
    // params
    #[rhai_fn(global, get = "params")]
    pub fn get_params(args: &mut InformArgs) -> String {
        crate::opts::params_to_str(&args.params)
    }
    #[rhai_fn(global, set = "params")]
    pub fn set_params(args: &mut InformArgs, params: String) {
        trace!(?params, "setting params");
        args.params = crate::opts::parse_params(&params).expect("failed to parse params");
    }
}
