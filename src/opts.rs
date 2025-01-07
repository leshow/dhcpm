use std::{net::Ipv4Addr, str::FromStr};

use anyhow::{anyhow, Error, Result};
use dhcproto::{v4, Decodable, Decoder, Encoder};
use mac_address::MacAddress;
use tracing_subscriber::{
    fmt::{
        self,
        format::{Format, Pretty, PrettyFields},
    },
    prelude::__tracing_subscriber_SubscriberExt,
    util::SubscriberInitExt,
    EnvFilter,
};

use crate::Args;

/// default timeout is set to 5 (seconds)
pub fn default_timeout() -> u64 {
    5
}

pub fn get_mac() -> MacAddress {
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

pub fn init_tracing(args: &Args) {
    let filter_layer = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("info"))
        .unwrap();
    match args.output {
        LogStructure::Pretty => {
            tracing_subscriber::registry()
                .with(filter_layer)
                .with(
                    fmt::layer()
                        .event_format(Format::default().with_source_location(false))
                        .fmt_fields(PrettyFields::new())
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

/// takes input like: "118,hex,C0A80001" or "118,ip,192.168.0.1"
/// and converts to a valid DhcpOption
pub fn parse_opts(input: &str) -> Result<v4::DhcpOption, String> {
    match &input.splitn(3, ',').collect::<Vec<&str>>()[..] {
        [code, ty, val] => {
            let code = code.parse::<u8>().map_err(|_| "error parsing OptionCode")?;
            let opt = match *ty {
                "hex" => Ok(hex::decode(val).map_err(|_| "decoding hex failed")?),
                "str" => Ok(val.as_bytes().to_vec()),
                "ip" => Ok(val
                    .parse::<Ipv4Addr>()
                    .map_err(|_| "decoding IP failed")?
                    .octets()
                    .to_vec()),
                _ => Err("failed to decode with a type we understand \"hex\" or \"ip\" or \"str\""),
            }?;
            Ok(write_opt(code, opt).map_err(|e| {
                eprintln!("{e}");
                "failed to encode to DhcpOption"
            })?)
        }
        _ => Err("parsing options failed".to_string()),
    }
}

fn write_opt(code: u8, opt: Vec<u8>) -> Result<v4::DhcpOption> {
    let mut buf = vec![];
    let mut enc = Encoder::new(&mut buf);
    enc.write_u8(code)?;
    enc.write_u8(opt.len() as u8)?;
    enc.write_slice(&opt)?;

    Ok(v4::DhcpOption::decode(&mut Decoder::new(&buf))?)
}

pub fn default_params() -> Vec<v4::OptionCode> {
    vec![
        v4::OptionCode::SubnetMask,
        v4::OptionCode::Router,
        v4::OptionCode::DomainNameServer,
        v4::OptionCode::DomainName,
    ]
}

pub fn parse_params(params: &str) -> Result<Vec<v4::OptionCode>, String> {
    params
        .split(',')
        .map(|code| {
            code.parse::<u8>()
                .map(v4::OptionCode::from)
                .map_err(|_| "parsing OptionCode failed".to_string())
        })
        .collect()
}

pub fn parse_mac(mac: &str) -> Result<MacAddress, String> {
    match mac {
        "random" => Ok(rand::random::<[u8; 6]>().into()),
        mac => match MacAddress::from_str(mac) {
            Ok(mac) => Ok(mac),
            Err(err) => MacAddress::from_str(
                &mac.chars()
                    .collect::<Vec<_>>()
                    .chunks(2)
                    .map(|chunk| chunk.iter().collect::<String>())
                    .collect::<Vec<String>>()
                    .join(":"),
            )
            .map_err(|_| format!("{:?}", err)),
        }
        .map_err(|err| format!("{:?}", err)),
    }
}

#[cfg(feature = "script")]
pub fn params_to_str(params: &[v4::OptionCode]) -> String {
    params
        .iter()
        .map(|code| u8::from(*code).to_string())
        .collect::<Vec<_>>()
        .join(",")
}

pub mod v6 {
    use dhcproto::v6;

    pub fn parse_params(params: &str) -> Result<Vec<v6::OptionCode>, String> {
        params
            .split(',')
            .map(|code| {
                code.parse::<u16>()
                    .map(v6::OptionCode::from)
                    .map_err(|_| "parsing OptionCode failed".to_string())
            })
            .collect()
    }
}
