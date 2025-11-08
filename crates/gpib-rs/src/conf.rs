//! Minimal parser for gpib.conf and helpers to find it.

use std::{env, fs, path::PathBuf};

#[derive(Debug, Clone)]
pub(crate) struct InterfaceDef {
    pub name: String,
    pub minor: i32,
    #[allow(dead_code)]
    pub board_type: Option<String>,
    pub pad: Option<i32>,
}

#[derive(Debug, Clone)]
pub(crate) struct DeviceDef {
    #[allow(dead_code)]
    pub name: Option<String>,
    pub minor: i32, // default 0
    pub pad: i32,
    #[allow(dead_code)]
    pub sad: Option<i32>,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct GpibConf {
    pub interfaces: Vec<InterfaceDef>,
    pub devices: Vec<DeviceDef>,
}

pub(crate) fn default_conf_paths() -> Vec<PathBuf> {
    let mut v = Vec::new();
    for k in ["GPIB_CONF", "GPIB_CONF_PATH"] {
        if let Ok(p) = env::var(k) {
            if !p.is_empty() {
                v.push(PathBuf::from(p));
            }
        }
    }
    v.push(PathBuf::from("/etc/gpib.conf"));
    v.push(PathBuf::from("/usr/local/etc/gpib.conf"));

    // NixOS: /nix/store/*linux-gpib*/etc/gpib.conf
    if let Ok(rd) = fs::read_dir("/nix/store") {
        for ent in rd.flatten() {
            let name_os = ent.file_name();
            let Some(name) = name_os.to_str() else {
                continue;
            };
            if !name.contains("gpib") {
                continue;
            }
            let cand = ent.path().join("etc/gpib.conf");
            if cand.is_file() {
                v.push(cand);
            }
        }
    }
    v
}

pub(crate) fn load_gpib_conf() -> Option<(GpibConf, PathBuf)> {
    for p in default_conf_paths() {
        if let Ok(txt) = fs::read_to_string(&p) {
            if let Some(conf) = parse_gpib_conf(&txt) {
                // eprintln!("(info) gpib.conf loaded from: {}", p.display());
                return Some((conf, p));
            }
        }
    }
    eprintln!("(info) gpib.conf not found in default paths");
    None
}

pub(crate) fn parse_gpib_conf(s: &str) -> Option<GpibConf> {
    #[derive(Copy, Clone, PartialEq)]
    enum State {
        Top,
        InInterface,
        InDevice,
    }

    let mut st = State::Top;
    let mut conf = GpibConf::default();

    let (mut cur_name, mut cur_minor, mut cur_board_type, mut cur_pad) =
        (None::<String>, None::<i32>, None::<String>, None::<i32>);
    let (mut dev_name, mut dev_minor, mut dev_pad, mut dev_sad) =
        (None::<String>, None::<i32>, None::<i32>, None::<i32>);

    for mut line in s.lines() {
        if let Some(i) = line.find('#') {
            line = &line[..i];
        }
        let line = line.trim().trim_end_matches(';').trim();
        if line.is_empty() {
            continue;
        }

        match st {
            State::Top => {
                if (line.starts_with("interface") && line.ends_with('{')) || line == "interface {" {
                    st = State::InInterface;
                    cur_name = None;
                    cur_minor = None;
                    cur_board_type = None;
                    cur_pad = None;
                } else if (line.starts_with("device") && line.ends_with('{')) || line == "device {"
                {
                    st = State::InDevice;
                    dev_name = None;
                    dev_minor = None;
                    dev_pad = None;
                    dev_sad = None;
                }
            }
            State::InInterface => {
                if line == "}" {
                    if let (Some(name), Some(minor)) = (cur_name.take(), cur_minor.take()) {
                        conf.interfaces.push(InterfaceDef {
                            name,
                            minor,
                            board_type: cur_board_type.take(),
                            pad: cur_pad.take(),
                        });
                    }
                    st = State::Top;
                    continue;
                }
                if let Some((k, v)) = parse_kv(line) {
                    match k.as_str() {
                        "name" => cur_name = Some(unquote(&v).to_string()),
                        "minor" => cur_minor = v.parse::<i32>().ok(),
                        "board_type" => cur_board_type = Some(unquote(&v).to_string()),
                        "pad" => cur_pad = v.parse::<i32>().ok(),
                        _ => {}
                    }
                }
            }
            State::InDevice => {
                if line == "}" {
                    if let Some(pad) = dev_pad.take() {
                        let minor = dev_minor.unwrap_or(0);
                        conf.devices.push(DeviceDef {
                            name: dev_name.take(),
                            minor,
                            pad,
                            sad: dev_sad.take(),
                        });
                    }
                    st = State::Top;
                    continue;
                }
                if let Some((k, v)) = parse_kv(line) {
                    match k.as_str() {
                        "name" => dev_name = Some(unquote(&v).to_string()),
                        "minor" => dev_minor = v.parse::<i32>().ok(),
                        "pad" => dev_pad = v.parse::<i32>().ok(),
                        "sad" => dev_sad = v.parse::<i32>().ok(),
                        _ => {}
                    }
                }
            }
        }
    }
    if conf.interfaces.is_empty() {
        None
    } else {
        Some(conf)
    }
}

fn parse_kv(line: &str) -> Option<(String, String)> {
    let mut it = line.splitn(2, '=');
    let k = it.next()?.trim();
    let v = it.next()?.trim();
    if k.is_empty() || v.is_empty() {
        return None;
    }
    Some((k.to_string(), v.to_string()))
}
fn unquote(v: &str) -> &str {
    let v = v.trim();
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        &v[1..v.len() - 1]
    } else {
        v
    }
}

/// "gpib0" => Some(0), "gpib12" => Some(12), otherwise None
pub(crate) fn parse_board_index(name: &str) -> Option<i32> {
    name.strip_prefix("gpib")
        .and_then(|rest| rest.parse::<i32>().ok())
}
