use std::cmp;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::ops::Add;
use std::process;
use std::sync::{Arc, Mutex};
use std::sync::mpsc;
use std::thread;

use bytes::{Buf, ByteBuf};
use mio;
use mio::{EventLoop, Token};
use rand::{Rng, thread_rng};
use protobuf;
use protobuf::Message;
use uuid::Uuid;

use {Append, AppendRes, CliReq, CliRes, Clock, GetReq, GetRes, Mutation, MutationType, PeerMsg,
     RealClock, RedirectRes, SetReq, SetRes, Version, CASReq, CASRes, DelReq, DelRes, VoteReq,
     VoteRes};
use server::{Envelope, LEADER_DURATION, PEER_BROADCAST, State, AckedLog, InMemoryLog, LogEntry,
             PeerID, RepPeer, TXID, Term, SendChannel, Store, KV, Range};
use server::traffic_cop::TrafficCop;

pub struct Server<C: Clock, RE> {
    pub clock: Arc<C>,
    pub peer_port: u16,
    pub cli_port: u16,
    pub id: PeerID,
    pub rpc_tx: Box<SendChannel<Envelope, RE> + Send>,
    pub ranges: BTreeMap<Vec<u8>, Box<Range<C, RE>>>,
    pub pending: BTreeMap<TXID, (Envelope, u64)>,
    pub kv: Arc<KV>,
}

unsafe impl<C: Clock, RE> Sync for Server<C, RE>{}

impl<C: Clock, RE> Server<C, RE> {
    pub fn run(peer_port: u16, cli_port: u16, storage_dir: String, peers: Vec<String>) {
        // All long-running worker threads get a clone of this
        // Sender.  When they exit, they send over it.  If the
        // Receiver ever completes a read, it means something
        // unexpectedly exited.  It's vital that we shut down
        // immediately, so we don't repeat the ZK bug where
        // the heartbeater keeps running while other vital threads
        // have exited, falsely communicating healthiness.
        let (thread_exit_tx, thread_exit_rx) = mpsc::channel();

        // The TrafficCop manages our sockets, sends deserialized
        // messages over the request channel, and receives completed
        // responses over the response channel.
        let (peer_req_tx, peer_req_rx) = mpsc::channel();
        let (cli_req_tx, cli_req_rx) = mpsc::channel();

        let mut tc = TrafficCop::new(peer_port, cli_port, peers.clone(), peer_req_tx, cli_req_tx)
                         .unwrap();

        // A single MIO EventLoop handles our IO
        let mut event_loop = EventLoop::new().unwrap();

        // All RPC's are sent over the event_loop's
        // notification channel.
        let rpc_tx = event_loop.channel();

        // start server periodic tasks
        event_loop.timeout_ms((), thread_rng().gen_range(200, 500)).unwrap();

        // IO event loop thread
        let tex1 = thread_exit_tx.clone();
        thread::Builder::new()
            .name("IO loop".to_string())
            .spawn(move || {
                tc.run_event_loop(event_loop);
                tex1.send(());
            });

        let clock = Arc::new(RealClock);

        let server = Arc::new(Mutex::new(Server {
            clock: clock.clone(),
            peer_port: peer_port,
            cli_port: cli_port,
            id: Uuid::new_v4().to_string(), // TODO(tyler) read from rocksdb
            rpc_tx: Box::new(rpc_tx),
            pending: BTreeMap::new(),
            kv: Arc::new(KV::new(storage_dir)),
            ranges: BTreeMap::new(),
        }));

        // peer request handler thread
        let srv1 = server.clone();
        let tex2 = thread_exit_tx.clone();
        thread::Builder::new()
            .name("peer request handler".to_string())
            .spawn(move || {
                for req in peer_req_rx {
                    match srv1.lock() {
                        Ok(mut srv) => srv.handle_peer(req),
                        Err(e) => {
                            error!("{}", e);
                            process::exit(1);
                        }
                    }
                }
                tex2.send(());
            });

        // cli request handler thread
        let srv2 = server.clone();
        let tex3 = thread_exit_tx.clone();
        thread::Builder::new()
            .name("cli request handler".to_string())
            .spawn(move || {
                for req in cli_req_rx {
                    match srv2.lock() {
                        Ok(mut srv) => srv.handle_cli(req),
                        Err(e) => {
                            error!("{}", e);
                            process::exit(1);
                        }
                    }
                }
                tex3.send(());
            });

        // cron thread
        let srv3 = server.clone();
        let tex4 = thread_exit_tx.clone();
        thread::Builder::new()
            .name("server cron".to_string())
            .spawn(move || {
                let mut rng = thread_rng();
                loop {
                    clock.sleep_ms(rng.gen_range(400, 500));
                    match srv3.lock() {
                        Ok(mut srv) => {
                            for (_, range) in srv.ranges {
                                range.cron()
                            }
                        }
                        Err(e) => {
                            error!("{}", e);
                            process::exit(1);
                        }
                    }
                }
                tex4.send(());
            });

        // this should never receive
        thread_exit_rx.recv();
        let msg = "A worker thread unexpectedly exited! Shutting down.";
        error!("{}", msg);
        panic!("A worker thread unexpectedly exited! Shutting down.");
    }

    pub fn handle_peer(&mut self, env: Envelope) {
        let peer_msg: PeerMsg = protobuf::parse_from_bytes(env.msg.bytes()).unwrap();
        self.ranges.get(peer_msg.get_range_prefix()).unwrap().handle_peer(env);
    }
    fn handle_cli(&mut self, env: Envelope) {
        let cli_req: CliReq = protobuf::parse_from_bytes(env.msg.bytes()).unwrap();
        let key = cli_req.get_key();
        let ranges = self.ranges.keys().filter(|k| key.starts_with(k)).collect();
        if ranges.len() == 0 {
            // TODO(tyler) reply with range-aware redirect
        }
        self.ranges.get(ranges.last().unwrap()).unwrap().handle_peer(env);
    }
}
