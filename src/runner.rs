use std::{
    fmt,
    time::{Duration, Instant},
};

use anyhow::Result;
use tokio::{
    sync::{broadcast, mpsc},
    time,
};
use tracing::{error, info, trace};

use dhcproto::{
    decoder::{Decodable, Decoder},
    encoder::{Encodable, Encoder},
    v4, v6,
};

use crate::{shutdown::Shutdown, Args};

#[derive(Debug)]
pub struct Runner {
    pub args: Args,
    pub notify_shutdown: broadcast::Sender<()>,
    pub shutdown_complete_rx: mpsc::Receiver<()>,
    pub shutdown_complete_tx: mpsc::Sender<()>,
}

impl Runner {
    pub async fn run(&mut self) -> Result<()> {
        let mut gen = MsgGen {
            args: self.args.clone(),
            // Receive shutdown notifications.
            shutdown: Shutdown::new(self.notify_shutdown.subscribe()),
            // Notifies the receiver half once all clones are
            // dropped.
            _shutdown_complete: self.shutdown_complete_tx.clone(),
        };
        if let Err(err) = gen.run().await {
            error!(?err, "generator exited with error");
        }

        Ok(())
    }
}

#[derive(Debug)]
pub struct MsgGen {
    args: Args,
    shutdown: Shutdown,
    _shutdown_complete: mpsc::Sender<()>,
}

impl MsgGen {
    pub async fn run(&mut self) -> Result<()> {
        // timers
        let sleep = time::sleep(Duration::from_secs(1));
        tokio::pin!(sleep);
        let start = Instant::now();

        if !self.shutdown.is_shutdown() {
            tokio::select! {
                _ = &mut sleep => {
                    let elapsed = start.elapsed();
                    trace!(elapsed = %PrettyDuration(elapsed), "timeout hit");
                }
                _ = self.shutdown.recv() => {
                    trace!("shutdown received");
                    return Ok(());
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
