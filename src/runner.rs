use std::{
    fmt,
    net::{IpAddr, SocketAddr, UdpSocket},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use anyhow::Result;
use crossbeam_channel::{bounded, select, tick, Receiver, Sender};
use tracing::{error, info, trace};

use dhcproto::{
    decoder::{Decodable, Decoder},
    encoder::Encodable,
    v4, v6,
};

use crate::{Args, MsgType};

const MAX_RETRIES: usize = 3;

#[derive(Debug)]
pub struct Runner {
    pub args: Args,
    pub shutdown_rx: Receiver<()>,
}

impl Runner {
    pub fn run(&mut self) -> Result<()> {
        let start = Instant::now();
        let bind_addr: SocketAddr = self.args.bind.unwrap();
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
        while count < MAX_RETRIES {
            select! {
                recv(rx) -> res => {
                    match res {
                        Ok(msg) => {
                            info!(msg_type = ?msg.get_type(), msg = %PrettyPrint(PrettyMsg(&msg)), "decoded");
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

    // unfinished
    pub fn dora(&mut self) -> Result<()> {
        let start = Instant::now();
        let bind_addr: SocketAddr = self.args.bind.unwrap();
        let send = Arc::new(UdpSocket::bind(bind_addr)?);
        let recv = Arc::clone(&send);

        // this channel is for receiving a decoded v4/v6 message
        let (tx, rx) = bounded(1);
        // this is for controlling when we send so we're able to retry
        let (retry_tx, retry_rx) = bounded(1);
        self.recv(tx, recv);
        // self.retry_send(retry_rx, send);

        let timeout = tick(Duration::from_secs(self.args.timeout));

        retry_tx.send(()).expect("retry channel send failed");
        let mut count = 0;
        while count < MAX_RETRIES {
            select! {
                recv(rx) -> res => {
                    match res {
                        Ok(msg) => {
                            info!(msg_type = ?msg.get_type(), msg = %PrettyPrint(PrettyMsg(&msg)), "decoded");
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
    fn retry_send(
        &self,
        retry_rx: Receiver<()>,
        send: Arc<UdpSocket>,
        (msg, target): (Msg, SocketAddr),
    ) {
        thread::spawn(move || {
            while retry_rx.recv().is_ok() {
                info!(msg_type = ?msg.get_type(), ?target, msg = %PrettyPrint(PrettyMsg(&msg)), "sending msg");
                send.send_to(&msg.to_vec()?[..], target)?;
            }

            Ok::<_, anyhow::Error>(())
        });
    }

    fn send(&self, retry_rx: Receiver<()>, send: Arc<UdpSocket>) {
        let args = self.args.clone();
        thread::spawn(move || {
            let mut count = 0;
            while retry_rx.recv().is_ok() {
                if let Err(err) = try_send(&args, &send) {
                    error!(?err, "error sending");
                }
                count += 1;
            }

            if count >= MAX_RETRIES {
                error!("max retries-- exiting");
            }
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
    let (msg, target) = build_msg(args, send)?;
    send.send_to(&msg.to_vec()?[..], target)?;
    Ok(())
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
        // dhcpv6
        MsgType::Solicit(_) => todo!("solicit unimplemented at the moment"),
    };
    info!(msg_type = ?msg.get_type(), ?target, msg = %PrettyPrint(PrettyMsg(&msg)), "sending msg");

    Ok((msg, target))
}

#[derive(Copy, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
struct PrettyDuration(Duration);

impl fmt::Display for PrettyDuration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}s", &self.0.as_secs_f32().to_string()[0..=4])
    }
}

#[derive(Copy, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
struct PrettyPrint<T>(T);

impl<T: fmt::Debug> fmt::Display for PrettyPrint<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:#?}", &self.0)
    }
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
