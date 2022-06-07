use std::{
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
    v4, v6,
};

use crate::{
    util::{Msg, PrettyPrint, PrettyTime},
    Args, MsgType,
};

const MAX_RETRIES: usize = 2;

// Runner is still fundamentally written to send a single
// DHCP message over a single socket at a time.

#[derive(Debug, Clone)]
pub struct TimeoutRunner {
    // TODO: de-couple runner from Args?
    // would make it easier to swap message types/targets
    pub args: Args,
    pub shutdown_rx: Receiver<()>,
    pub send_tx: Sender<(Msg, SocketAddr)>,
    pub recv_rx: Receiver<(Msg, SocketAddr)>,
}

impl TimeoutRunner {
    // TODO: can probably &mut self & take Msg as param
    /// Generate a message from `Args` and send it, waiting for a reply
    /// if `args.no_retry` is false (by default) we will retry `MAX_RETRIES` times
    /// for a timeout of `args.timeout`
    pub fn send(mut self) -> Result<Msg> {
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
                            info!(msg_type = ?msg.get_type(), elapsed = %PrettyTime(start.elapsed()), msg = %PrettyPrint(&msg),"RECEIVED");
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
        let TimeoutRunner { send_tx, .. } = self;
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
            MsgType::Decline(args) => Msg::V4(args.build()),
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
            info!(msg_type = ?msg.get_type(), ?target, msg = %PrettyPrint(&msg), "SENT");
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
