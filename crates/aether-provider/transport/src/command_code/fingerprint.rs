//! Deterministic CLI identity; never reads the gateway host's machine identity.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const SALT: &str = "command-code:device-fingerprint:v1";

fn digest(key: &str, field: &str) -> Vec<u8> {
    Sha256::digest(format!("\0{key}\0{field}").as_bytes()).to_vec()
}

fn hex(key: &str, field: &str, count: usize) -> String {
    digest(key, field)[..count]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn pick<'a>(key: &str, field: &str, choices: &'a [&str]) -> &'a str {
    choices
        .iter()
        .max_by_key(|label| digest(key, &format!("{field}\0{label}")))
        .copied()
        .unwrap_or_default()
}

fn hash(value: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(format!("{SALT}\0{}", value.trim().to_lowercase()).as_bytes())
    )
}

pub fn fingerprint(key: &str) -> Value {
    let cpu = pick(
        key,
        "cpu",
        &[
            "12th Gen Intel(R) Core(TM) i7-12650H|10",
            "12th Gen Intel(R) Core(TM) i5-12400F|6",
            "12th Gen Intel(R) Core(TM) i9-12900K|16",
            "13th Gen Intel(R) Core(TM) i7-13700K|16",
            "13th Gen Intel(R) Core(TM) i5-13600K|14",
            "13th Gen Intel(R) Core(TM) i9-13900K|24",
            "Intel(R) Core(TM) Ultra 7 155H|16",
            "Intel(R) Core(TM) Ultra 9 285H|16",
            "Intel(R) Core(TM) i9-14900K|24",
            "Intel(R) Core(TM) i7-14700K|20",
            "AMD Ryzen 7 7800X3D|8",
            "AMD Ryzen 9 7950X|16",
            "AMD Ryzen 5 7600|6",
            "AMD Ryzen 9 7900X|12",
            "AMD Ryzen 7 5800X3D|8",
        ],
    );
    let (cpu_model, cpu_count) = cpu.rsplit_once('|').unwrap_or((cpu, "8"));
    let memory = pick(key, "mem", &["8", "16", "24", "32", "48", "64"]);
    let timezone = pick(
        key,
        "timezone",
        &[
            "America/New_York",
            "America/Chicago",
            "America/Los_Angeles",
            "America/Toronto",
            "Europe/London",
            "Europe/Berlin",
            "Europe/Paris",
            "Europe/Moscow",
            "Asia/Shanghai",
            "Asia/Tokyo",
            "Asia/Singapore",
            "Asia/Seoul",
            "Asia/Hong_Kong",
            "Australia/Sydney",
            "Pacific/Auckland",
        ],
    );
    let count = pick(key, "macCount", &["2", "3", "4", "5"])
        .parse::<usize>()
        .unwrap_or(2);
    let user = pick(
        key,
        "osUser",
        &["dev", "user", "admin", "coder", "engineer", "work"],
    );
    let domain = pick(
        key,
        "mailDomain",
        &["gmail.com", "outlook.com", "qq.com", "163.com"],
    );
    let mid = hex(key, "machineId", 16);
    let machine_id = format!(
        "{}-{}-{}-{}-{}",
        &mid[..8],
        &mid[8..12],
        &mid[12..16],
        &mid[16..20],
        &mid[20..]
    );
    let mut macs = (0..count)
        .map(|i| {
            digest(key, &format!("mac{i}"))[..6]
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<Vec<_>>()
                .join(":")
        })
        .collect::<Vec<_>>();
    macs.sort();
    let thumb = format!("{SALT}\0machine\0{machine_id}|{}", macs.join(","));
    json!({
        "thumbmark": format!("{:x}", Sha256::digest(thumb.as_bytes())),
        "components": {
            "machineIdHash": hash(&machine_id), "macHashes": macs.iter().map(|mac| hash(mac)).collect::<Vec<_>>(),
            "osUserHash": hash(user), "hostnameHash": hash(&format!("DESKTOP-{}", hex(key, "hostname", 4).to_uppercase())),
            "gitEmailHash": hash(&format!("{user}.{}@{domain}", hex(key, "gitEmail", 3))),
            "platform": "win32", "arch": "x64", "osRelease": "10.0.22631",
            "cpuModel": cpu_model, "cpuCount": cpu_count.parse::<u32>().unwrap_or(8),
            "memGiB": memory.parse::<u32>().unwrap_or(16), "isContainer": false,
            "timezone": timezone, "runtime": "cli", "collectorVersion": 1
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identity_is_stable_and_never_contains_the_credential() {
        let a = fingerprint("user_fixture_a");
        assert_eq!(a, fingerprint("user_fixture_a"));
        assert_ne!(a["thumbmark"], fingerprint("user_fixture_b")["thumbmark"]);
        assert!(!a.to_string().contains("user_fixture_a"));
        assert_eq!(a["components"]["platform"], "win32");
    }
}
