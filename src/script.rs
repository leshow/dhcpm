use std::path::PathBuf;

use dhcproto::{v4, v6};
// use rhai::packages::Package;
use rhai::EvalAltResult;
use rhai::{plugin::*, Engine};
use tracing::{debug, info};

use crate::{
    runner::{Msg, Runner},
    DiscoverArgs, InformArgs, MsgType, ReleaseArgs, RequestArgs,
};

// exposing Msg
#[export_module]
mod msg_mod {
    #[allow(non_snake_case)]
    pub fn V4(msg: v4::Message) -> Msg {
        Msg::V4(msg)
    }
    #[allow(non_snake_case)]
    pub fn V6(msg: v6::Message) -> Msg {
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
    #[rhai_fn(global, name = "to_string", name = "to_debug", pure)]
    pub fn to_string(msg: &mut Msg) -> String {
        format!("{:?}", msg)
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

// exposing v4::Message
#[export_module]
mod v4_msg_mod {
    #[rhai_fn()]
    pub fn msg_default() -> v4::Message {
        v4::Message::default()
    }
    // ciaddr
    #[rhai_fn(global, get = "ciaddr", pure)]
    pub fn get_ciaddr(msg: &mut v4::Message) -> String {
        msg.ciaddr().to_string()
    }
    // yiaddr
    #[rhai_fn(global, get = "yiaddr", pure)]
    pub fn get_yiaddr(msg: &mut v4::Message) -> String {
        msg.yiaddr().to_string()
    }

    // giaddr
    #[rhai_fn(global, get = "giaddr", pure)]
    pub fn get_giaddr(msg: &mut v4::Message) -> String {
        msg.giaddr().to_string()
    }

    // siaddr
    #[rhai_fn(global, get = "siaddr", pure)]
    pub fn get_siaddr(msg: &mut v4::Message) -> String {
        msg.siaddr().to_string()
    }

    #[rhai_fn(global, name = "to_string", name = "to_debug", pure)]
    pub fn to_string(msg: &mut v4::Message) -> String {
        format!("{:?}", msg)
    }
    // '==' and '!=' operators
    #[rhai_fn(global, name = "==", pure)]
    pub fn eq(msg: &mut v4::Message, msg2: v4::Message) -> bool {
        msg == &msg2
    }
    #[rhai_fn(global, name = "!=", pure)]
    pub fn neq(msg: &mut v4::Message, msg2: v4::Message) -> bool {
        msg != &msg2
    }
}

pub fn main<P: Into<PathBuf>>(path: P, runner: Runner) -> Result<(), Box<EvalAltResult>> {
    let mut engine = Engine::new();
    // TODO: this is gross
    let discover_run = runner.clone();
    let request_run = runner.clone();
    let release_run = runner.clone();
    let inform_run = runner;

    engine
        // load random package for rhai scripts
        // .register_global_module(rhai_rand::RandomPackage::new().as_shared_module())
        // register types
        .register_type_with_name::<DiscoverArgs>("DiscoverArgs")
        .register_type_with_name::<RequestArgs>("RequestArgs")
        .register_type_with_name::<ReleaseArgs>("ReleaseArgs")
        .register_type_with_name::<InformArgs>("InformArgs")
        .register_type_with_name::<Msg>("Msg")
        .register_type_with_name::<v4::Message>("v4::Message")
        // register modules
        .register_static_module("Msg", exported_module!(msg_mod).into())
        .register_static_module("v4::Message", exported_module!(v4_msg_mod).into())
        .register_static_module(
            "discover",
            exported_module!(crate::discover::discover_mod).into(),
        )
        .register_static_module(
            "request",
            exported_module!(crate::request::request_mod).into(),
        )
        .register_static_module(
            "release",
            exported_module!(crate::release::release_mod).into(),
        )
        .register_static_module("inform", exported_module!(crate::inform::inform_mod).into())
        // TODO: return result?
        .register_fn("send", {
            move |args: &mut DiscoverArgs| {
                let mut new_runner = discover_run.clone();
                // replace runner args so it knows which message type to run
                new_runner.args.msg = Some(MsgType::Discover(args.clone()));
                new_runner.run().expect("runner failed").unwrap_v4()
            }
        })
        .register_fn("send", move |args: &mut RequestArgs| {
            let mut new_runner = request_run.clone();
            // replace runner args so it knows which message type to run
            new_runner.args.msg = Some(MsgType::Request(args.clone()));
            new_runner.run().expect("runner failed").unwrap_v4()
        })
        .register_fn("send", move |args: &mut ReleaseArgs| {
            let mut new_runner = release_run.clone();
            // replace runner args so it knows which message type to run
            new_runner.args.msg = Some(MsgType::Release(args.clone()));
            new_runner.run().expect("runner failed").unwrap_v4()
        })
        .register_fn("send", move |args: &mut InformArgs| {
            let mut new_runner = inform_run.clone();
            // replace runner args so it knows which message type to run
            new_runner.args.msg = Some(MsgType::Inform(args.clone()));
            new_runner.run().expect("runner failed").unwrap_v4()
        });
    // Any function or closure that takes an '&str' argument can be used to override 'print'.
    engine.on_print(|msg| info!(rhai = msg));

    // Any function or closure that takes a '&str', an 'Option<&str>' and a 'Position' argument
    // can be used to override 'debug'.
    engine.on_debug(|msg, src, pos| {
        let src = src.unwrap_or("unknown");
        debug!(?src, ?pos, rhai = ?msg)
    });

    // run the script
    engine.run_file(path.into())?;
    Ok(())
}
