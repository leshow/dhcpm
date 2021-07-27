use std::{
    fmt,
    net::{Ipv4Addr, SocketAddr, UdpSocket},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use anyhow::Result;
use crossbeam_channel::{bounded, select, tick, Receiver};
use tracing::{error, info, trace};

use dhcproto::{
    decoder::{Decodable, Decoder},
    encoder::{Encodable, Encoder},
    v4, v6,
};

use crate::{Args, Family};

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
        // recv
        let args = self.args.clone();
        thread::spawn(move || {
            let mut buf = vec![0; 1024];
            let (len, addr) = recv.recv_from(&mut buf)?;
            trace!(buf = ?&buf[..len], "recv");
            let msg = if args.ip.is_ipv6() {
                Msg::V6(v6::Message::decode(&mut Decoder::new(&buf[..len]))?)
            } else {
                Msg::V4(v4::Message::decode(&mut Decoder::new(&buf[..len]))?)
            };
            trace!(buf = ?msg, "decoded");
            tx.send_timeout(msg, Duration::from_secs(1))?;
            Ok::<_, anyhow::Error>(())
        });
        // send
        let args = self.args.clone();
        thread::spawn(move || {
            let target: SocketAddr = (args.ip, args.port.unwrap()).into();
            let buf = if args.ip.is_ipv6() {
                todo!()
            } else {
                let msg = v4::Message::new(
                    Ipv4Addr::UNSPECIFIED,
                    // TODO: use different yiaddr
                    "192.168.0.1".parse().unwrap(),
                    "192.168.0.1".parse().unwrap(),
                    Ipv4Addr::UNSPECIFIED,
                    &[222, 173, 192, 222, 202, 254],
                );
                msg.to_vec()
            }?;
            send.send_to(&buf[..], target)?;
            Ok::<_, anyhow::Error>(())
        });

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
