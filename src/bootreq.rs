use std::net::Ipv4Addr;

use argh::FromArgs;
use dhcproto::v4;
use mac_address::MacAddress;

use crate::opts::{self, parse_mac, parse_opts};

#[derive(FromArgs, PartialEq, Eq, Debug, Clone)]
/// Send a DISCOVER msg
#[argh(subcommand, name = "bootreq")]
pub struct BootReqArgs {
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
    /// giaddr [default: 0.0.0.0]
    #[argh(option, short = 'g', default = "Ipv4Addr::UNSPECIFIED")]
    pub giaddr: Ipv4Addr,
    /// fname [default: None]
    #[argh(option)]
    pub fname: Option<String>,
    /// sname [default: None]
    #[argh(option)]
    pub sname: Option<String>,
    /// add opts to the message
    /// [ex: these are equivalent- "118,hex,C0A80001" or "118,ip,192.168.0.1"]
    #[argh(option, short = 'o', from_str_fn(parse_opts))]
    pub opt: Vec<v4::DhcpOption>,
}

impl Default for BootReqArgs {
    fn default() -> Self {
        Self {
            chaddr: opts::get_mac(),
            ciaddr: Ipv4Addr::UNSPECIFIED,
            giaddr: Ipv4Addr::UNSPECIFIED,
            opt: Vec::new(),
            fname: None,
            sname: None,
        }
    }
}

impl BootReqArgs {
    pub fn build(&self, broadcast: bool) -> v4::Message {
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

        if let Some(fname) = &self.fname {
            msg.set_fname_str(fname);
        }

        if let Some(sname) = &self.sname {
            msg.set_sname_str(sname);
        }

        // insert manually entered opts
        for opt in &self.opt {
            msg.opts_mut().insert(opt.clone());
        }

        msg
    }
}

#[cfg(feature = "script")]
use rhai::{plugin::*, EvalAltResult};

// exposing DiscoverArgs
#[cfg(feature = "script")]
#[export_module]
pub mod bootreq_mod {
    use tracing::trace;
    #[rhai_fn()]
    pub fn args_default() -> BootReqArgs {
        BootReqArgs::default()
    }
    #[rhai_fn(global, name = "to_string", name = "to_debug", pure)]
    pub fn to_string(args: &mut BootReqArgs) -> String {
        format!("{:?}", args)
    }
    // ciaddr
    #[rhai_fn(global, get = "ciaddr", pure)]
    pub fn get_ciaddr(args: &mut BootReqArgs) -> String {
        args.ciaddr.to_string()
    }
    #[rhai_fn(global, set = "ciaddr")]
    pub fn set_ciaddr(args: &mut BootReqArgs, ciaddr: &str) {
        trace!(?ciaddr, "setting ciaddr");
        args.ciaddr = ciaddr.parse::<Ipv4Addr>().expect("failed to parse ciaddr");
    }
    // giaddr
    #[rhai_fn(global, get = "giaddr", pure)]
    pub fn get_giaddr(args: &mut BootReqArgs) -> String {
        args.giaddr.to_string()
    }
    #[rhai_fn(global, set = "giaddr")]
    pub fn set_giaddr(args: &mut BootReqArgs, giaddr: &str) {
        trace!(?giaddr, "setting giaddr");
        args.giaddr = giaddr.parse::<Ipv4Addr>().expect("failed to parse giaddr");
    }
    // chaddr
    #[rhai_fn(global, get = "chaddr", pure)]
    pub fn get_chaddr(args: &mut BootReqArgs) -> rhai::Blob {
        args.chaddr.bytes().to_vec()
    }
    #[rhai_fn(global, set = "chaddr")]
    pub fn set_chaddr(args: &mut BootReqArgs, chaddr: rhai::Blob) {
        trace!(?chaddr, "setting chaddr");
        let bytes: [u8; 6] = chaddr.try_into().expect("failed to convert macaddress");
        args.chaddr = bytes.into();
    }
    #[rhai_fn(global, name = "rand_chaddr")]
    pub fn rand_chaddr(args: &mut BootReqArgs) {
        let chaddr = rand::random::<[u8; 6]>().into();
        trace!(?chaddr, "setting random chaddr");
        args.chaddr = chaddr;
    }
    // opt
    #[rhai_fn(global, set = "opt")]
    pub fn set_opt(args: &mut BootReqArgs, opt: String) {
        trace!(?opt, "adding opt to message");
        args.opt
            .push(crate::opts::parse_opts(&opt).expect("failed to parse opt"));
    }
}
