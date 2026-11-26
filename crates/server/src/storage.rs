//! The storage thread: owns the `WorldStore`, loads columns on request and
//! writes columns it is handed, so region-file I/O never runs in the tick.

use std::sync::Arc;
use std::thread::JoinHandle;

use crossbeam_channel::{Receiver, Sender, unbounded};
use world::storage::{ColumnRecord, WorldStore};
use world::{Column, ColumnPos, Registry};

/// A request to the storage thread.
pub enum Request {
    /// Load a column; answered with [`Reply::Loaded`].
    Load(ColumnPos),
    /// Write columns.
    Save(Vec<ColumnRecord>),
    /// Write everything and stop.
    Stop,
}

/// An answer from the storage thread.
pub enum Reply {
    /// A load result: `None` if the column was never saved (generate it).
    Loaded(ColumnPos, Option<Column>),
    /// A write failed; the message is for the log.
    Error(String),
}

/// Handle to the storage thread.
pub struct Storage {
    /// Requests.
    pub tx: Sender<Request>,
    /// Replies.
    pub rx: Receiver<Reply>,
    thread: Option<JoinHandle<()>>,
}

impl Storage {
    /// Starts the thread.
    pub fn start(mut store: WorldStore, registry: Arc<Registry>) -> std::io::Result<Self> {
        let (tx, requests) = unbounded::<Request>();
        let (replies, rx) = unbounded::<Reply>();
        let thread = std::thread::Builder::new()
            .name("storage".into())
            .spawn(move || {
                for req in requests {
                    match req {
                        Request::Load(pos) => {
                            let column = match store.load_column(pos, &registry) {
                                Ok(c) => c.map(|(col, _)| col),
                                Err(e) => {
                                    // A corrupt column is regenerated rather
                                    // than stopping the server.
                                    let _ = replies
                                        .send(Reply::Error(format!("loading column {pos:?}: {e}")));
                                    None
                                }
                            };
                            if replies.send(Reply::Loaded(pos, column)).is_err() {
                                break;
                            }
                        }
                        Request::Save(records) => {
                            if let Err(e) = store.save_all(&records) {
                                let _ = replies.send(Reply::Error(format!("saving: {e}")));
                            }
                        }
                        Request::Stop => break,
                    }
                }
            })?;
        Ok(Self {
            tx,
            rx,
            thread: Some(thread),
        })
    }

    /// Finishes pending writes and stops the thread.
    pub fn stop(&mut self) {
        let _ = self.tx.send(Request::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Storage {
    fn drop(&mut self) {
        self.stop();
    }
}
