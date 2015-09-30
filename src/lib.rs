#![crate_id = "rasputin"]
#![crate_type = "lib"]

pub use serialization::{
    Mutation, MutationType,
    Version, KV,
    SetReq, SetRes,
    GetReq, GetRes,
    CASReq, CASRes,
    WatchReq, WatchRes,
    RedirectRes,
    CliReq, CliRes,
    VoteReq, VoteRes,
    Append, AppendRes,
    PeerMsg,
};

pub use codec::{
    Codec,
    Framed,
};

pub use clock::{
    Clock,
    RealClock,
    TestClock,
};

pub use range_bounds::{
    RangeBounds,
};

pub use client::{
    Client,
};

pub mod client;
pub mod clock;
pub mod codec;
pub mod logging;
pub mod range_bounds;
pub mod serialization;
pub mod server;

extern crate bytes;
#[macro_use] extern crate log;
#[macro_use] extern crate lazy_static;
extern crate mio;
extern crate protobuf;
extern crate rand;
extern crate rocksdb;
extern crate time;
extern crate uuid;
extern crate threadpool;
