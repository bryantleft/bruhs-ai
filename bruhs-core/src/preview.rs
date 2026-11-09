//! Local dev-server discovery for the Preview surface (web) and the
//! "Open a local server" palette (desktop): TCP listeners on localhost via
//! `lsof`, minus ephemeral ports and the caller's own port.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct LocalServer {
    pub port: u16,
    pub command: String,
    pub pid: u32,
}

/// IANA dynamic range: transient OS/app sockets, never a dev server worth listing.
pub const EPHEMERAL_PORT_START: u16 = 49152;

fn host_is_local(host: &str) -> bool {
    matches!(
        host,
        "*" | "0.0.0.0" | "127.0.0.1" | "localhost" | "[::]" | "[::1]" | "::" | "::1"
    )
}

/// Parse `lsof -iTCP -sTCP:LISTEN -P -n` output into unique local ports,
/// sorted ascending, skipping `exclude_ports` and the ephemeral range.
pub fn parse_lsof(out: &str, exclude_ports: &[u16]) -> Vec<LocalServer> {
    let mut servers: Vec<LocalServer> = Vec::new();
    for line in out.lines().skip_while(|l| l.starts_with("COMMAND")) {
        if !line.contains("(LISTEN)") {
            continue;
        }
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 9 {
            continue;
        }
        // lsof escapes spaces in command names as `\x20`.
        let command = cols[0].replace("\\x20", " ");
        let Ok(pid) = cols[1].parse::<u32>() else {
            continue;
        };
        let Some(name_idx) = cols.iter().position(|c| *c == "(LISTEN)") else {
            continue;
        };
        if name_idx == 0 {
            continue;
        }
        let name = cols[name_idx - 1];
        let Some((host, port)) = name.rsplit_once(':') else {
            continue;
        };
        if !host_is_local(host) {
            continue;
        }
        let Ok(port) = port.parse::<u16>() else {
            continue;
        };
        if port >= EPHEMERAL_PORT_START
            || exclude_ports.contains(&port)
            || servers.iter().any(|s| s.port == port)
        {
            continue;
        }
        servers.push(LocalServer { port, command, pid });
    }
    servers.sort_by_key(|s| s.port);
    servers
}

/// Run `lsof` and parse it; an unavailable `lsof` yields an empty list.
pub async fn scan_local_servers(exclude_ports: &[u16]) -> Vec<LocalServer> {
    let out = tokio::process::Command::new("lsof")
        .args(["-iTCP", "-sTCP:LISTEN", "-P", "-n"])
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(LSOF_TIMEOUT, out).await {
        Ok(Ok(o)) => parse_lsof(&String::from_utf8_lossy(&o.stdout), exclude_ports),
        // The list is a convenience: without lsof there are no suggestions.
        Ok(Err(error)) => {
            tracing::debug!("lsof unavailable: {error}");
            Vec::new()
        }
        Err(_) => {
            tracing::warn!("lsof did not finish within {LSOF_TIMEOUT:?}");
            Vec::new()
        }
    }
}

/// `lsof` answers in well under a second; one that hangs is killed.
const LSOF_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
COMMAND     PID     USER   FD   TYPE             DEVICE SIZE/OFF NODE NAME
node      12345 bryantle   23u  IPv6 0x1234abcd      0t0  TCP *:3000 (LISTEN)
node      12345 bryantle   24u  IPv4 0x1234abce      0t0  TCP 127.0.0.1:3000 (LISTEN)
bruhs-ser 92208 bryantle   10u  IPv4 0x9999          0t0  TCP 127.0.0.1:3001 (LISTEN)
Python      555 bryantle    3u  IPv6 0x5555          0t0  TCP [::1]:8081 (LISTEN)
vite       7777 bryantle   18u  IPv4 0x7777          0t0  TCP 127.0.0.1:5173 (LISTEN)
sshd        123 root        3u  IPv4 0x0001          0t0  TCP 192.168.1.20:22 (LISTEN)
postgres    888 bryantle    7u  IPv4 0x8888          0t0  TCP 127.0.0.1:5432 (LISTEN)
Discord    6466 bryantle   40u  IPv4 0x6466          0t0  TCP 127.0.0.1:51320 (LISTEN)
";

    #[test]
    fn parse_lsof_extracts_localhost_listeners() {
        let servers = parse_lsof(SAMPLE, &[3001]);
        let ports: Vec<u16> = servers.iter().map(|s| s.port).collect();
        assert_eq!(ports, vec![3000, 5173, 5432, 8081]);
        assert_eq!(servers[0].command, "node");
        assert_eq!(servers[0].pid, 12345);
        assert_eq!(servers[3].command, "Python");
    }

    #[test]
    fn parse_lsof_unescapes_spaces_in_command_names() {
        let line = "T3\\x20Cod 24852 bryantle 30u IPv4 0x1 0t0 TCP 127.0.0.1:3773 (LISTEN)\n";
        let servers = parse_lsof(line, &[]);
        assert_eq!(servers[0].command, "T3 Cod");
    }

    #[test]
    fn parse_lsof_handles_empty_and_garbage() {
        assert!(parse_lsof("", &[]).is_empty());
        assert!(parse_lsof("COMMAND PID\nnot a line\n", &[]).is_empty());
    }
}
