use argh::FromArgs;
use dhcproto::v6;
use mac_address::MacAddress;

use crate::opts::{self, parse_mac, v6::parse_params};

#[derive(FromArgs, PartialEq, Eq, Debug, Clone)]
/// Send a INFORMATION-REQUEST msg (dhcpv6)
#[argh(subcommand, name = "inforeq")]
pub struct InformationReqArgs {
    /// supply a mac address for DHCPv6 (use "random" for a random mac) [default: first interface mac]
    #[argh(
        option,
        short = 'c',
        from_str_fn(parse_mac),
        default = "opts::get_mac()"
    )]
    pub chaddr: MacAddress,
    /// params to include: [default: 23,24,39,59]
    #[argh(option, from_str_fn(parse_params), default = "default_opts()")]
    pub params: Vec<v6::OptionCode>,
}

pub fn default_opts() -> Vec<v6::OptionCode> {
    vec![
        v6::OptionCode::DomainNameServers,
        v6::OptionCode::DomainSearchList,
        v6::OptionCode::Unknown(39),
        v6::OptionCode::Unknown(59),
    ]
}

impl Default for InformationReqArgs {
    fn default() -> Self {
        Self {
            chaddr: opts::get_mac(),
            params: default_opts(),
        }
    }
}

impl InformationReqArgs {
    pub fn build(&self) -> v6::Message {
        let mut msg = v6::Message::new(v6::MessageType::InformationRequest);

        msg.opts_mut().insert(v6::DhcpOption::ORO(v6::ORO {
            opts: self.params.clone(),
        }));

        msg
    }
}

// #[cfg(feature = "script")]
// use rhai::{plugin::*, EvalAltResult};

// // exposing ReleaseArgs
// #[cfg(feature = "script")]
// #[export_module]
// pub mod decline_mod {
//     use tracing::trace;
//     #[rhai_fn()]
//     pub fn args_default() -> DeclineArgs {
//         DeclineArgs::default()
//     }
//     #[rhai_fn(global, name = "to_string", name = "to_debug", pure)]
//     pub fn to_string(args: &mut DeclineArgs) -> String {
//         format!("{:?}", args)
//     }
//     // chaddr
//     #[rhai_fn(global, get = "chaddr", pure)]
//     pub fn get_chaddr(args: &mut DeclineArgs) -> rhai::Blob {
//         args.chaddr.bytes().to_vec()
//     }
//     #[rhai_fn(global, set = "chaddr")]
//     pub fn set_chaddr(args: &mut DeclineArgs, chaddr: rhai::Blob) {
//         trace!(?chaddr, "setting chaddr");
//         let bytes: [u8; 6] = chaddr.try_into().expect("failed to convert macaddress");
//         args.chaddr = bytes.into();
//     }
//     #[rhai_fn(global, name = "rand_chaddr")]
//     pub fn rand_chaddr(args: &mut DeclineArgs) {
//         let chaddr = rand::random::<[u8; 6]>().into();
//         trace!(?chaddr, "setting random chaddr");
//         args.chaddr = chaddr;
//     }
//     // opt
//     #[rhai_fn(global, set = "opt")]
//     pub fn set_opt(args: &mut DeclineArgs, opt: String) {
//         trace!(?opt, "adding opt to message");
//         args.opt
//             .push(crate::opts::parse_opts(&opt).expect("failed to parse opt"));
//     }
//     // params
//     #[rhai_fn(global, get = "params")]
//     pub fn get_params(args: &mut DeclineArgs) -> String {
//         crate::opts::params_to_str(&args.params)
//     }
//     #[rhai_fn(global, set = "params")]
//     pub fn set_params(args: &mut DeclineArgs, params: String) {
//         trace!(?params, "setting params");
//         args.params = crate::opts::parse_params(&params).expect("failed to parse params");
//     }
// }
