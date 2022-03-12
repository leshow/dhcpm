use std::{
    fmt,
    net::{IpAddr, SocketAddr, UdpSocket},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use anyhow::Result;
use crossbeam_channel::{bounded, select, tick, Receiver, Sender};
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

const MAX_RETRIES: usize = 3;

#[derive(Debug)]
pub struct Runner {
    pub args: Args,
    pub shutdown_rx: Receiver<()>,
    pub soc: Arc<UdpSocket>,
}

impl Runner {
    pub fn run(&mut self) -> Result<Msg> {
        let total = Instant::now();
        let mut start = Instant::now();
        let (mut sender, rx) = SingleMessage::new(&self.args, self.soc.clone())?;
        let timeout = tick(Duration::from_secs(self.args.timeout));

        // do send and wait for recv
        sender.run();
        let mut count = 0;
        while count < MAX_RETRIES {
            select! {
                // we will recv on this channel
                recv(rx) -> res => {
                    match res {
                        Ok(msg) => {
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
                    sender.retry();
                    start = Instant::now();
                    continue;
                }
            }
        }
        let SingleMessage { tx, retry_tx, .. } = sender;
        drop(tx);
        drop(retry_tx);

        Err(anyhow::anyhow!(
            "{} no message received",
            PrettyTime(total.elapsed())
        ))
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
    // match wrap {
    //     Ctrl::Done(_) => tx.send_timeout(Ctrl::Done(msg), Duration::from_secs(1))?,
    //     Ctrl::Continue(_) => tx.send_timeout(Ctrl::Continue(msg), Duration::from_secs(1))?,
    // }
    tx.send_timeout(msg, Duration::from_secs(1))?;
    Ok(())
}

fn try_send(args: &Args, send: &Arc<UdpSocket>) -> Result<()> {
    let (msg, target) = build_msg(args, send)?;
    send.send_to(&msg.to_vec()?[..], target)?;
    Ok(())
}

fn sender_thread(rx: Receiver<(Msg, SocketAddr)>, soc: Arc<UdpSocket>) {
    thread::spawn(move || {
        loop {
            let (msg, target) = rx.recv()?;
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
        }
        Ok::<_, anyhow::Error>(())
    });
}

fn recv_thread(tx: Sender<(Msg, SocketAddr)>, soc: Arc<UdpSocket>) {
    thread::spawn(move || {
        loop {
            let mut buf = vec![0; 1024];
            let (len, addr) = soc.recv_from(&mut buf)?;
            trace!(buf = ?&buf[..len], "recv");
            let msg = if addr.is_ipv6() {
                Msg::V6(v6::Message::decode(&mut Decoder::new(&buf[..len]))?)
            } else {
                Msg::V4(v4::Message::decode(&mut Decoder::new(&buf[..len]))?)
            };
            tx.send_timeout((msg, addr), Duration::from_secs(1))?;
        }
        Ok::<_, anyhow::Error>(())
    });
}

fn set_broadcast(args: &Args, send: &Arc<UdpSocket>) -> Result<(SocketAddr, bool)> {
    Ok(match args.target {
        IpAddr::V4(addr) if addr.is_broadcast() => {
            send.set_broadcast(true)?;
            ((args.target, args.port.unwrap()).into(), true)
        }
        IpAddr::V4(addr) => ((addr, args.port.unwrap()).into(), false),
        IpAddr::V6(addr) if addr.is_multicast() => {
            send.join_multicast_v6(&addr, 0)?;
            ((addr, args.port.unwrap()).into(), false)
        }
        IpAddr::V6(addr) => ((IpAddr::V6(addr), args.port.unwrap()).into(), false),
    })
}

fn build_msg(args: &Args, send: &Arc<UdpSocket>) -> Result<(Msg, SocketAddr)> {
    let (target, broadcast) = set_broadcast(args, send)?;
    let msg = match &args.msg {
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
    info!(msg_type = ?msg.get_type(), ?target, msg = %PrettyPrint(PrettyMsg(&msg)), "SENT");

    Ok((msg, target))
}

#[derive(Clone, Debug)]
pub enum Msg {
    V4(v4::Message),
    V6(v6::Message),
}

impl Msg {
    fn get_type(&self) -> String {
        match self {
            Msg::V4(m) => format!("{:?}", m.opts().msg_type().unwrap()),
            Msg::V6(m) => format!("{:?}", m.opts()),
        }
    }

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

#[derive(Debug)]
pub struct SingleMessage {
    args: Args,
    tx: Sender<Msg>,
    retry_tx: Sender<()>,
    retry_rx: Receiver<()>,
    soc: Arc<UdpSocket>,
}

impl SingleMessage {
    pub fn new(args: &Args, soc: Arc<UdpSocket>) -> Result<(Self, Receiver<Msg>)> {
        // this channel is for receiving a decoded v4/v6 message
        let (tx, rx) = bounded(1);
        // this is for controlling when we send so we're able to retry
        let (retry_tx, retry_rx) = bounded(1);

        Ok((
            Self {
                tx,
                retry_rx,
                retry_tx,
                soc,
                args: args.clone(),
            },
            rx,
        ))
    }
    pub fn run(&mut self) {
        send(&self.args, self.retry_rx.clone(), self.soc.clone());
        recv(&self.args, self.tx.clone(), self.soc.clone());
        // allow first send to happen
        self.retry();
    }
    pub fn retry(&mut self) {
        self.retry_tx.send(()).expect("retry channel send failed");
    }
}

fn send(args: &Args, retry_rx: Receiver<()>, soc: Arc<UdpSocket>) {
    let args = args.clone();
    thread::spawn(move || {
        let mut count = 0;
        while retry_rx.recv().is_ok() {
            if let Err(err) = try_send(&args, &soc) {
                error!(?err, "error sending");
            }
            count += 1;
        }

        if count >= MAX_RETRIES {
            error!("max retries-- exiting");
        }
        trace!("thread dropped send");
        Ok::<_, anyhow::Error>(())
    });
}

// TODO: need to get this thread to exit
fn recv(args: &Args, tx: Sender<Msg>, soc: Arc<UdpSocket>) {
    let args = args.clone();
    thread::spawn(move || {
        if let Err(err) = try_recv(&args, &tx, &soc) {
            error!(?err, "could not receive");
        }
        trace!("thread dropped rec");
        Ok::<_, anyhow::Error>(())
    });
}
