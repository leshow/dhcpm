use std::{
    fmt,
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use anyhow::Result;
use crossbeam_channel::{bounded, select, tick, Receiver, Sender};
use mac_address::MacAddress;
use tracing::{error, info, trace};

use dhcproto::{
    decoder::{Decodable, Decoder},
    encoder::Encodable,
    v4, v6,
};

use crate::{Args, MsgType};

#[derive(Debug)]
pub struct Runner {
    pub args: Args,
    pub shutdown_rx: Receiver<()>,
}

impl Runner {
    pub fn run(&mut self) -> Result<()> {
        let start = Instant::now();
        let bind_addr: SocketAddr = (self.args.bind.unwrap(), 0).into();
        let send = Arc::new(UdpSocket::bind(bind_addr)?);
        let recv = Arc::clone(&send);

        // this channel is for receiving a decoded v4/v6 message
        let (tx, rx) = bounded(1);
        // this is for controlling when we send so we're able to retry
        let (retry_tx, retry_rx) = bounded(1);
        self.recv(tx, recv);
        self.send(retry_rx, send);

        let timeout = tick(Duration::from_secs(self.args.timeout));

        retry_tx.send(()).expect("retry channel send failed");
        let mut count = 0;
        while count < 3 {
            select! {
                recv(rx) -> res => {
                    match res {
                        Ok(msg) => {
                            info!(buf = ?msg, "decoded");
                            break;
                        }
                        Err(err) => {
                            error!(?err, "channel returned error");
                            break;
                        }
                    }
                }
                recv(self.shutdown_rx) -> _ => {
                    trace!("shutdown signal received");
                    break;
                }
                recv(timeout) -> _ => {
                    trace!(elapsed = %PrettyDuration(start.elapsed()), "received timeout-- retrying");
                    count += 1;
                    retry_tx.send(()).expect("retry channel send failed");
                    continue;
                }
            }
        }
        drop(retry_tx);

        Ok(())
    }

    fn send(&self, retry_rx: Receiver<()>, send: Arc<UdpSocket>) {
        let args = self.args.clone();
        thread::spawn(move || {
            while retry_rx.recv().is_ok() {
                if let Err(err) = try_send(&args, &send) {
                    error!(?err, "error sending");
                }
            }
            error!("max retries-- exiting");
            Ok::<_, anyhow::Error>(())
        });
    }

    fn recv(&self, tx: Sender<Msg>, recv: Arc<UdpSocket>) {
        let args = self.args.clone();
        thread::spawn(move || {
            if let Err(err) = try_recv(&args, &tx, &recv) {
                error!(?err, "could not receive");
            }
            Ok::<_, anyhow::Error>(())
        });
    }
}

fn try_recv(args: &Args, tx: &Sender<Msg>, recv: &Arc<UdpSocket>) -> Result<()> {
    let mut buf = vec![0; 1024];
    let (len, _addr) = recv.recv_from(&mut buf)?;
    trace!(buf = ?&buf[..len], "recv");
    let msg = if args.target.is_ipv6() {
        Msg::V6(v6::Message::decode(&mut Decoder::new(&buf[..len]))?)
    } else {
        Msg::V4(v4::Message::decode(&mut Decoder::new(&buf[..len]))?)
    };
    tx.send_timeout(msg, Duration::from_secs(1))?;

    Ok(())
}

fn try_send(args: &Args, send: &Arc<UdpSocket>) -> Result<()> {
    let target: SocketAddr = (args.target, args.port.unwrap()).into();
    let msg = match args.msg {
        MsgType::Discover(discover) => v4_discover(discover.chaddr, args.req_addr),
        _ => todo!(),
    };
    trace!(?msg, "sending msg");
    send.send_to(&msg.to_vec()?[..], target)?;
    Ok(())
}

fn v4_discover(chaddr: MacAddress, req_addr: Option<IpAddr>) -> v4::Message {
    let mut msg = v4::Message::new(
        Ipv4Addr::UNSPECIFIED,
        Ipv4Addr::UNSPECIFIED,
        Ipv4Addr::UNSPECIFIED,
        Ipv4Addr::UNSPECIFIED,
        &chaddr.bytes(),
    );

    msg.opts_mut()
        .insert(v4::DhcpOption::MessageType(v4::MessageType::Discover));
    msg.opts_mut()
        .insert(v4::DhcpOption::ClientIdentifier(chaddr.bytes().to_vec()));
    msg.opts_mut().insert(v4::DhcpOption::AddressLeaseTime(120));
    msg.opts_mut()
        .insert(v4::DhcpOption::ParameterRequestList(vec![
            v4::OptionCode::SubnetMask,
            v4::OptionCode::Router,
            v4::OptionCode::DomainNameServer,
            v4::OptionCode::DomainName,
        ]));
    // TODO: add more?
    // add requested ip
    if let Some(IpAddr::V4(ip)) = req_addr {
        msg.opts_mut()
            .insert(v4::DhcpOption::RequestedIpAddress(ip));
    }
    msg
}

fn v4_request(chaddr: MacAddress, req_addr: Ipv4Addr, server: Ipv4Addr) -> v4::Message {
    let mut msg = v4::Message::new(
        Ipv4Addr::UNSPECIFIED,
        Ipv4Addr::UNSPECIFIED,
        Ipv4Addr::UNSPECIFIED,
        Ipv4Addr::UNSPECIFIED,
        &chaddr.bytes(),
    );

    msg.opts_mut()
        .insert(v4::DhcpOption::MessageType(v4::MessageType::Request));
    msg.opts_mut()
        .insert(v4::DhcpOption::RequestedIpAddress(req_addr));
    msg.opts_mut()
        .insert(v4::DhcpOption::ServerIdentifier(server));
    msg
}

#[derive(Copy, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
struct PrettyDuration(Duration);

impl fmt::Display for PrettyDuration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}s", &self.0.as_secs_f32().to_string()[0..=4])
    }
}

#[derive(Clone, Debug)]
pub enum Msg {
    V4(v4::Message),
    V6(v6::Message),
}
