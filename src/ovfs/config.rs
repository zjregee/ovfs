use std::time::Duration;
use std::collections::HashMap;

#[derive(Debug)]
pub struct Config {
    /// How long the FUSE client should consider file and directory attributes to be valid.
    pub attr_timeout: Duration,
    /// How long the FUSE client should consider directory entries to be valid.
    pub entry_timeout: Duration,
    /// The path of the root directory.
    pub root_dir: String,
    /// Whether to enable logs.
    pub enabled_log: bool,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            attr_timeout: Duration::from_secs(5),
            entry_timeout: Duration::from_secs(5),
            root_dir: String::from("/"),
            enabled_log: true,
        }
    }
}

impl From<HashMap<String, String>> for Config {
    fn from(map: HashMap<String, String>) -> Config {
        let mut config = Config::default();
        for (key, value) in map.into_iter() {
            match key.as_str() {
                "attr_timeout" => {
                    if let Ok(v) = value.parse::<u64>() {
                        config.attr_timeout = Duration::from_secs(v);
                    }
                },
                "entry_timeout" => {
                    if let Ok(v) = value.parse::<u64>() {
                        config.entry_timeout = Duration::from_secs(v);
                    }
                },
                "root_dir" => {
                    config.root_dir = value;
                },
                "enabled_log" => {
                    if let Ok(v) = value.parse::<bool>() {
                        config.enabled_log = v;
                    }
                },
                _ => (),
            }
        }
        return config;
    }
}
