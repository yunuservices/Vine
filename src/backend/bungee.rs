use uuid::Uuid;

use crate::config::Config;
use crate::protocol::ProtocolVersion;
use crate::protocol::text::{parse_message, plain_text};
use crate::session::{PlayerAction, PlayerSession, SessionManager, send_reliably};

/// Channels backend plugins use to ask the proxy for things, before and since 1.13
pub(super) fn is_channel(channel: &str) -> bool {
    channel == "BungeeCord" || channel == "bungeecord:main"
}

/// What the tunnel does after a request on the channel
pub(super) enum Outcome {
    /// Send this back to the backend on the channel the request came on
    Reply(Vec<u8>),
    /// Move the player who asked to this server
    Connect(String),
    Done,
}

/// The player whose backend sent the request
pub(super) struct Asker<'a> {
    pub(super) username: &'a str,
    pub(super) uuid: Uuid,
    pub(super) server: &'a str,
}

pub(super) fn handle(
    data: &[u8],
    asker: &Asker<'_>,
    config: &Config,
    sessions: &SessionManager,
) -> Outcome {
    let mut input = data;
    handle_request(&mut input, asker, config, sessions).unwrap_or(Outcome::Done)
}

fn handle_request(
    input: &mut &[u8],
    asker: &Asker<'_>,
    config: &Config,
    sessions: &SessionManager,
) -> Option<Outcome> {
    let subchannel = read_utf(input)?;
    let mut out = Vec::new();
    write_utf(&mut out, &subchannel);
    match subchannel.as_str() {
        "Connect" => return Some(Outcome::Connect(read_utf(input)?)),
        "ConnectOther" => {
            let player = sessions.get_session(&read_utf(input)?)?;
            let server_name = read_utf(input)?;
            send_reliably(&player.action_tx, PlayerAction::Connect { server_name });
            return Some(Outcome::Done);
        }
        "IP" => {
            let me = sessions.get_session(asker.username)?;
            write_utf(&mut out, &me.client_ip);
            out.extend_from_slice(&i32::from(me.client_port).to_be_bytes());
        }
        "IPOther" => {
            let player = sessions.get_session(&read_utf(input)?)?;
            write_utf(&mut out, &player.username);
            write_utf(&mut out, &player.client_ip);
            out.extend_from_slice(&i32::from(player.client_port).to_be_bytes());
        }
        "PlayerCount" => {
            let server = read_utf(input)?;
            let count = on_server(sessions, &server, config)?.len();
            write_utf(&mut out, &server);
            out.extend_from_slice(&(count as i32).to_be_bytes());
        }
        "PlayerList" => {
            let server = read_utf(input)?;
            let names: Vec<String> = on_server(sessions, &server, config)?
                .into_iter()
                .map(|s| s.username)
                .collect();
            write_utf(&mut out, &server);
            write_utf(&mut out, &names.join(", "));
        }
        "GetServers" => {
            let names: Vec<&str> = config
                .sorted_servers()
                .into_iter()
                .map(|(name, _)| name.as_str())
                .collect();
            write_utf(&mut out, &names.join(", "));
        }
        "GetServer" => write_utf(&mut out, asker.server),
        "GetPlayerServer" => {
            let player = sessions.get_session(&read_utf(input)?)?;
            write_utf(&mut out, &player.username);
            write_utf(&mut out, &player.current_server);
        }
        "UUID" => write_utf(&mut out, &asker.uuid.simple().to_string()),
        "UUIDOther" => {
            let player = sessions.get_session(&read_utf(input)?)?;
            write_utf(&mut out, &player.username);
            write_utf(&mut out, &player.uuid.simple().to_string());
        }
        "ServerIP" => {
            let server = read_utf(input)?;
            let address = &config.servers.get(&server)?.address;
            let (host, port) = address.rsplit_once(':')?;
            write_utf(&mut out, &server);
            write_utf(&mut out, host.trim_start_matches('[').trim_end_matches(']'));
            out.extend_from_slice(&port.parse::<u16>().ok()?.to_be_bytes());
        }
        "Message" | "MessageRaw" => {
            let target = read_utf(input)?;
            let text = read_utf(input)?;
            let message = if subchannel == "Message" {
                parse_message(&text)
            } else {
                serde_json::from_str(&text).ok()?
            };
            for player in named_or_all(sessions, &target) {
                send_reliably(&player.action_tx, PlayerAction::Message(message.clone()));
            }
            return Some(Outcome::Done);
        }
        "KickPlayer" | "KickPlayerRaw" => {
            let player = sessions.get_session(&read_utf(input)?)?;
            let reason = read_utf(input)?;
            let reason = if subchannel == "KickPlayer" {
                reason
            } else {
                plain_text(&serde_json::from_str(&reason).ok()?)
            };
            send_reliably(&player.action_tx, PlayerAction::Disconnect(reason));
            return Some(Outcome::Done);
        }
        "Forward" | "ForwardToPlayer" => {
            let target = read_utf(input)?;
            let channel = read_utf(input)?;
            let len = usize::from(u16::from_be_bytes(take(input, 2)?.try_into().ok()?));
            let mut message = Vec::new();
            write_utf(&mut message, &channel);
            message.extend_from_slice(&(len as u16).to_be_bytes());
            message.extend_from_slice(take(input, len)?);
            let receivers = if subchannel == "Forward" {
                one_per_server(sessions, &target, asker.server)
            } else {
                sessions.get_session(&target).into_iter().collect()
            };
            for player in receivers {
                let channel = if ProtocolVersion(player.protocol_version) >= ProtocolVersion::V1_13
                {
                    "bungeecord:main"
                } else {
                    "BungeeCord"
                };
                send_reliably(
                    &player.action_tx,
                    PlayerAction::ServerPluginMessage {
                        channel: channel.to_string(),
                        data: message.clone(),
                    },
                );
            }
            return Some(Outcome::Done);
        }
        _ => return None,
    }
    Some(Outcome::Reply(out))
}

/// Players on a server, or everyone for "ALL"; None for a server that does not exist
fn on_server(
    sessions: &SessionManager,
    server: &str,
    config: &Config,
) -> Option<Vec<PlayerSession>> {
    let all = sessions.list_sessions();
    if server == "ALL" {
        return Some(all);
    }
    config.servers.get(server)?;
    Some(
        all.into_iter()
            .filter(|s| s.current_server == server)
            .collect(),
    )
}

fn named_or_all(sessions: &SessionManager, target: &str) -> Vec<PlayerSession> {
    if target == "ALL" {
        sessions.list_sessions()
    } else {
        sessions.get_session(target).into_iter().collect()
    }
}

/// A message only reaches a backend through a player on it, so one player per server is enough.
/// "ALL" and "ONLINE" mean every server with players other than the asker's.
fn one_per_server(sessions: &SessionManager, target: &str, from: &str) -> Vec<PlayerSession> {
    let mut seen = Vec::new();
    let mut receivers = Vec::new();
    for session in sessions.list_sessions() {
        let wanted = match target {
            "ALL" | "ONLINE" => session.current_server != from,
            server => session.current_server == server,
        };
        if wanted && !seen.contains(&session.current_server) {
            seen.push(session.current_server.clone());
            receivers.push(session);
        }
    }
    receivers
}

fn take<'a>(input: &mut &'a [u8], len: usize) -> Option<&'a [u8]> {
    if input.len() < len {
        return None;
    }
    let (head, rest) = input.split_at(len);
    *input = rest;
    Some(head)
}

/// Java's `DataInput.readUTF`: a u16 length, then modified UTF-8
fn read_utf(input: &mut &[u8]) -> Option<String> {
    let len = usize::from(u16::from_be_bytes(take(input, 2)?.try_into().ok()?));
    let mut bytes = take(input, len)?.iter();
    let mut units = Vec::with_capacity(len);
    while let Some(&a) = bytes.next() {
        let mut next = || bytes.next().map(|&b| u16::from(b & 0x3F));
        let unit = match a {
            0x00..=0x7F => u16::from(a),
            0xC0..=0xDF => (u16::from(a & 0x1F) << 6) | next()?,
            0xE0..=0xEF => (u16::from(a & 0x0F) << 12) | (next()? << 6) | next()?,
            _ => return None,
        };
        units.push(unit);
    }
    Some(String::from_utf16_lossy(&units))
}

/// Java's `DataOutput.writeUTF`: NUL takes two bytes and characters past U+FFFF are written as
/// their two UTF-16 halves
fn write_utf(out: &mut Vec<u8>, text: &str) {
    let mut bytes = Vec::with_capacity(text.len());
    for unit in text.encode_utf16() {
        match unit {
            0x0001..=0x007F => bytes.push(unit as u8),
            0x0000 | 0x0080..=0x07FF => {
                bytes.push(0xC0 | (unit >> 6) as u8);
                bytes.push(0x80 | (unit & 0x3F) as u8);
            }
            _ => {
                bytes.push(0xE0 | (unit >> 12) as u8);
                bytes.push(0x80 | ((unit >> 6) & 0x3F) as u8);
                bytes.push(0x80 | (unit & 0x3F) as u8);
            }
        }
    }
    let len = u16::try_from(bytes.len()).unwrap_or(u16::MAX);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&bytes[..usize::from(len)]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_utf_matches_java() {
        // Bytes from java.io.DataOutputStream.writeUTF
        let cases: [(&str, &[u8]); 3] = [
            ("lobby", &[0, 5, b'l', b'o', b'b', b'b', b'y']),
            ("a\0ş", &[0, 5, b'a', 0xC0, 0x80, 0xC5, 0x9F]),
            ("😀", &[0, 6, 0xED, 0xA0, 0xBD, 0xED, 0xB8, 0x80]),
        ];
        for (text, java) in cases {
            let mut out = Vec::new();
            write_utf(&mut out, text);
            assert_eq!(out, java, "{text}");
            assert_eq!(read_utf(&mut &java[..]).as_deref(), Some(text));
        }
    }
}
