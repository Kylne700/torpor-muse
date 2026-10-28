//! The Meta API side: where the key comes from, which address is allowed, the one HTTP call, and what a response
//! means. The key is never printed, logged or put on a command line. Owner: S3b-writer.
mod config;
mod http;
mod reply;

pub use config::{
    Key, config_path, day_cap, env_key_ok, key_file_gap, load_config, load_key, parse_config,
};
pub(super) use http::post_to;
pub use http::{Base, Net, Reply, api_base, clean, post};
pub use reply::{Bad, Reading, from_net, interpret};
