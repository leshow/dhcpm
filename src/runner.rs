use std::{
    fmt,
    net::{Ipv4Addr, SocketAddr, UdpSocket},
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
    encoder::{Encodable, Encoder},
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

        let (tx, rx) = bounded(100);
        self.send_recv(tx, send, recv);

        let timeout = tick(Duration::from_secs(self.args.timeout));
        loop {
            select! {
                recv(rx) -> res => {
                    match res {
                        Ok(msg) => {
                            trace!(?msg, "downstream responded");
                        }
                        Err(err) => {
                            error!(?err, "downstream responded with an error-- trying again");
                            // TODO
                        }
                    }
                }
                recv(self.shutdown_rx) -> _ => {
                    trace!("shutdown signal received");
                    break;
                }
                recv(timeout) -> _ => {
                    trace!(elapsed = %PrettyDuration(start.elapsed()), "received timeout");
                    break;
                }
            }
        }
        Ok(())
    }

    fn send_recv(&self, tx: Sender<Msg>, send: Arc<UdpSocket>, recv: Arc<UdpSocket>) {
        // start recv thread
        self.recv(tx, recv);
        // start sender
        let args = self.args.clone();
        thread::spawn(move || {
            let target: SocketAddr = (args.target, args.port.unwrap()).into();
            let buf = match args.msg {
                MsgType::Discover(discover) => v4_discover(discover.chaddr).to_vec(),
                _ => todo!(),
            }?;
            send.send_to(&buf[..], target)?;
            Ok::<_, anyhow::Error>(())
        });
    }

    fn recv(&self, tx: Sender<Msg>, recv: Arc<UdpSocket>) {
        let args = self.args.clone();
        thread::spawn(move || {
            let mut buf = vec![0; 1024];
            let (len, addr) = recv.recv_from(&mut buf)?;
            trace!(buf = ?&buf[..len], "recv");
            let msg = if args.target.is_ipv6() {
                Msg::V6(v6::Message::decode(&mut Decoder::new(&buf[..len]))?)
            } else {
                Msg::V4(v4::Message::decode(&mut Decoder::new(&buf[..len]))?)
            };
            info!(buf = ?msg, "decoded");
            tx.send_timeout(msg, Duration::from_secs(1))?;
            Ok::<_, anyhow::Error>(())
        });
    }
}

fn v4_discover(chaddr: MacAddress) -> v4::Message {
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
    msg
}

fn v4_request(chaddr: MacAddress, requested_addr: Ipv4Addr, server: Ipv4Addr) -> v4::Message {
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
        .insert(v4::DhcpOption::RequestedIpAddress(requested_addr));
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
