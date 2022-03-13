use std::{
    fmt,
    net::{IpAddr, SocketAddr, UdpSocket},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use crossbeam_channel::{select, tick, Receiver, Sender};
use tracing::{debug, error, info, trace};

use dhcproto::{
    decoder::{Decodable, Decoder},
    encoder::Encodable,
    v4, v6,
};

use crate::{
    util::{PrettyPrint, PrettyTime},
    Args, MsgType,
};

const MAX_RETRIES: usize = 2;

// TODO: only a single Runner can exist at a time right now

#[derive(Debug, Clone)]
pub struct Runner {
    // TODO: de-couple runner from Args?
    // would make it easier to swap message types/targets
    pub args: Args,
    pub shutdown_rx: Receiver<()>,
    pub send_tx: Sender<(Msg, SocketAddr)>,
    pub recv_rx: Receiver<(Msg, SocketAddr)>,
}

impl Runner {
    pub fn run(mut self) -> Result<Msg> {
        let total = Instant::now();
        let mut start = Instant::now();
        let timeout = tick(Duration::from_secs(self.args.timeout));

        // do send
        self.send_msg()?;
        let mut count = 0;
        while count < MAX_RETRIES {
            select! {
                // we will recv on this channel
                recv(self.recv_rx) -> res => {
                    match res {
                        Ok((msg, _addr)) => {
                            info!(msg_type = ?msg.get_type(), elapsed = %PrettyTime(start.elapsed()), msg = %PrettyPrint(PrettyMsg(&msg)),"RECEIVED");
                            return Ok(msg);
                        }
                        Err(err) => {
                            error!(?err, "channel returned error");
                            break;
                        }
                    }
                }
                // or recv a shutdown
                recv(self.shutdown_rx) -> _ => {
                    trace!("shutdown signal received");
                    break;
                }
                // or eventually time out
                recv(timeout) -> _ => {
                    debug!(elapsed = %PrettyTime(start.elapsed()), "received timeout-- retrying");
                    count += 1;
                    // try again
                    self.send_msg()?;
                    start = Instant::now();
                    if self.args.no_retry {
                        break;
                    } else {
                        continue;
                    }
                }
            }
        }
        let Runner { send_tx, .. } = self;
        drop(send_tx);

        Err(anyhow::anyhow!(
            "{} no message received",
            PrettyTime(total.elapsed())
        ))
    }

    fn send_msg(&mut self) -> Result<()> {
        let (target, broadcast) = self.args.get_target();
        let msg = match &self
            .args
            .msg
            .as_ref()
            .context("message type required, run --help")?
        {
            // dhcpv4
            MsgType::Discover(args) => Msg::V4(args.build(broadcast)),
            MsgType::Request(args) => Msg::V4(args.build(broadcast)),
            MsgType::Release(args) => Msg::V4(args.build()),
            MsgType::Inform(args) => Msg::V4(args.build()),
            // should be removed by now
            MsgType::Dora(_) => panic!("should be removed in main"),
            // dhcpv6
            MsgType::Solicit(_) => panic!("solicit unimplemented"),
        };
        self.send_tx.send((msg, target))?;
        Ok(())
    }
}

pub fn sender_thread(send_rx: Receiver<(Msg, SocketAddr)>, soc: Arc<UdpSocket>) {
    thread::spawn(move || {
        while let Ok((msg, target)) = send_rx.recv() {
            let port = target.port();
            // set broadcast appropriately
            let target: SocketAddr = match target.ip() {
                IpAddr::V4(addr) if addr.is_broadcast() => {
                    soc.set_broadcast(true)?;
                    (target.ip(), port).into()
                }
                IpAddr::V4(addr) => (addr, port).into(),
                IpAddr::V6(addr) if addr.is_multicast() => {
                    soc.join_multicast_v6(&addr, 0)?;
                    (addr, port).into()
                }
                IpAddr::V6(addr) => (IpAddr::V6(addr), port).into(),
            };
            soc.send_to(&msg.to_vec()?[..], target)?;
            info!(msg_type = ?msg.get_type(), ?target, msg = %PrettyPrint(PrettyMsg(&msg)), "SENT");
        }
        trace!("sender thread exited");
        Ok::<_, anyhow::Error>(())
    });
}

pub fn recv_thread(tx: Sender<(Msg, SocketAddr)>, soc: Arc<UdpSocket>) {
    thread::spawn(move || {
        let mut buf = vec![0; 1024];
        while let Ok((len, addr)) = soc.recv_from(&mut buf) {
            trace!(buf = ?&buf[..len], "recv");
            let msg = if addr.is_ipv6() {
                Msg::V6(v6::Message::decode(&mut Decoder::new(&buf[..len]))?)
            } else {
                Msg::V4(v4::Message::decode(&mut Decoder::new(&buf[..len]))?)
            };
            // reset buffer
            buf = vec![0; 1024];
            tx.send_timeout((msg, addr), Duration::from_secs(1))?;
        }
        trace!("recv thread exited");
        Ok::<_, anyhow::Error>(())
    });
}

#[derive(Clone, Debug, PartialEq, Eq)]
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

#[derive(Clone)]
struct PrettyMsg<'a>(&'a Msg);

impl<'a> fmt::Debug for PrettyMsg<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Msg::V4(msg) => {
                f.debug_struct("v4::Message")
                    .field("opcode", &msg.opcode())
                    .field("htype", &msg.htype())
                    .field("hlen", &msg.hlen())
                    .field("hops", &msg.hops())
                    .field("xid", &msg.xid())
                    .field("secs", &msg.secs())
                    .field("flags", &msg.flags())
                    .field("ciaddr", &msg.ciaddr())
                    .field("yiaddr", &msg.yiaddr())
                    .field("siaddr", &msg.siaddr())
                    .field("giaddr", &msg.giaddr())
                    .field("chaddr", &hex::encode(msg.chaddr()))
                    .field("sname", &msg.sname())
                    .field("fname", &msg.fname())
                    // .field("magic", &String::from_utf8_lossy(self.magic()))
                    .field("opts", &msg.opts())
                    .finish()
            }
            Msg::V6(_msg) => {
                // unfinished
                todo!("unfinished")
            }
        }
    }
}
