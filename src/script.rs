use std::{net::Ipv4Addr, path::PathBuf};

use mac_address::MacAddress;
use rhai::EvalAltResult;
use rhai::{plugin::*, Engine};
use tracing::trace;

use crate::{runner::Msg, DiscoverArgs};

#[export_module]
mod discover_mod {
    #[rhai_fn()]
    pub fn new_args() -> DiscoverArgs {
        DiscoverArgs::default()
    }
    // ciaddr
    #[rhai_fn(global, get = "ciaddr", pure)]
    pub fn get_ciaddr(args: &mut DiscoverArgs) -> Ipv4Addr {
        args.ciaddr
    }
    #[rhai_fn(global, set = "ciaddr")]
    pub fn set_ciaddr(args: &mut DiscoverArgs, ciaddr: Ipv4Addr) {
        trace!(?ciaddr, "setting ciaddr");
        args.ciaddr = ciaddr;
    }
    // giaddr
    #[rhai_fn(global, get = "giaddr", pure)]
    pub fn get_giaddr(args: &mut DiscoverArgs) -> Ipv4Addr {
        args.giaddr
    }
    #[rhai_fn(global, set = "giaddr")]
    pub fn set_giaddr(args: &mut DiscoverArgs, giaddr: Ipv4Addr) {
        trace!(?giaddr, "setting giaddr");
        args.giaddr = giaddr;
    }
    // relay_link
    #[rhai_fn(global, get = "relay_link", pure)]
    pub fn get_relay_link(args: &mut DiscoverArgs) -> Option<Ipv4Addr> {
        args.relay_link
    }
    #[rhai_fn(global, set = "relay_link")]
    pub fn set_relay_link(args: &mut DiscoverArgs, relay_link: Ipv4Addr) {
        trace!(?relay_link, "setting relay_link");
        args.relay_link = Some(relay_link);
    }
    // chaddr
    #[rhai_fn(global, get = "chaddr", pure)]
    pub fn get_chaddr(args: &mut DiscoverArgs) -> MacAddress {
        args.chaddr
    }
    #[rhai_fn(global, set = "chaddr")]
    pub fn set_chaddr(args: &mut DiscoverArgs, chaddr: MacAddress) {
        trace!(?chaddr, "setting chaddr");
        args.chaddr = chaddr;
    }
    #[rhai_fn(global, name = "rand_chaddr")]
    pub fn rand_chaddr(args: &mut DiscoverArgs) {
        let chaddr = rand::random::<[u8; 6]>().into();
        trace!(?chaddr, "setting random chaddr");
        args.chaddr = chaddr;
    }
    // req_addr
    #[rhai_fn(global, get = "req_addr", pure)]
    pub fn get_req_addr(args: &mut DiscoverArgs) -> Option<Ipv4Addr> {
        args.req_addr
    }
    #[rhai_fn(global, set = "req_addr")]
    pub fn set_req_addr(args: &mut DiscoverArgs, req_addr: Ipv4Addr) {
        trace!(?req_addr, "setting req_addr");
        args.req_addr = Some(req_addr);
    }
    // opt
    #[rhai_fn(global, set = "opt")]
    pub fn set_opt(args: &mut DiscoverArgs, opt: String) {
        trace!(?opt, "adding opt to message");
        args.opt
            .push(crate::opts::parse_opts(&opt).expect("failed to parse opt"));
    }
    // opt
    #[rhai_fn(global, set = "params")]
    pub fn set_params(args: &mut DiscoverArgs, params: String) {
        trace!(?params, "setting params");
        args.params = crate::opts::parse_params(&params).expect("failed to parse params");
    }
    // do the sending
    #[rhai_fn(global, return_raw)]
    pub fn send(args: &mut DiscoverArgs) -> Result<Msg, Box<EvalAltResult>> {
        todo!()
    }
}

// exposing Msg
#[export_module]
mod msg_mod {
    #[allow(non_snake_case)]
    pub fn V4(msg: dhcproto::v4::Message) -> Msg {
        Msg::V4(msg)
    }
    #[allow(non_snake_case)]
    pub fn V6(msg: dhcproto::v6::Message) -> Msg {
        Msg::V6(msg)
    }
    #[rhai_fn(global, get = "enum_type", pure)]
    pub fn get_type(msg: &mut Msg) -> String {
        match msg {
            Msg::V4(_) => "V4".to_string(),
            Msg::V6(_) => "V6".to_string(),
        }
    }
    #[rhai_fn(global, get = "inner", pure)]
    pub fn get_inner(my_enum: &mut Msg) -> Dynamic {
        match my_enum {
            Msg::V4(m) => Dynamic::from(m.clone()),
            Msg::V6(m) => Dynamic::from(m.clone()),
        }
    }
    // '==' and '!=' operators
    #[rhai_fn(global, name = "==", pure)]
    pub fn eq(msg: &mut Msg, msg2: Msg) -> bool {
        msg == &msg2
    }
    #[rhai_fn(global, name = "!=", pure)]
    pub fn neq(msg: &mut Msg, msg2: Msg) -> bool {
        msg != &msg2
    }
}

pub fn main<P: Into<PathBuf>>(path: P) -> Result<(), Box<EvalAltResult>> {
    let mut engine = Engine::new();
    engine
        // register types
        .register_type_with_name::<DiscoverArgs>("DiscoverArgs")
        .register_type_with_name::<Msg>("Msg")
        // register modules
        .register_static_module("Msg", exported_module!(msg_mod).into())
        .register_static_module("discover", exported_module!(discover_mod).into());

    // run the script
    // engine.eval_file(path.into())?;
    engine.run_file(path.into())?;
    Ok(())
}
