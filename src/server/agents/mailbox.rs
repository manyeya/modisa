// Agent ↔ agent messages. Queued per recipient, typed in only when the recipient is idle.
use std::collections::HashMap;

use serde::Serialize;

use crate::protocol::conn::{error, RpcResult};
use crate::server::session::pane::now_ms;

// reply_to: the sender as "<id>:<instance>", which reaches only that pane, even after it's renamed
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: u64,
    pub from: String,
    pub from_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    pub to: String,
    pub to_name: String,
    pub body: String,
    pub hops: u32,
    pub at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivered: Option<u64>,
}

pub struct Mailbox {
    pub log: Vec<Message>, // ponytail: last 500 in memory; persist if anyone needs history across restarts
    pub paused: bool,
    seq: u64, // ids keep rising across restarts, so an id never names two messages
    last_delivered_to: HashMap<String, Message>,
}

impl Mailbox {
    pub fn new() -> Mailbox {
        Mailbox { log: vec![], paused: false, seq: now_ms(), last_delivered_to: HashMap::new() }
    }

    pub fn send(&mut self, from: &str, from_name: &str, to: &str, to_name: &str, body: &str, reply_to: Option<String>, max_hops: u32, per_minute: usize) -> RpcResult<Message> {
        let now = now_ms();
        // a reply inherits the hop count of the message that prompted it
        let prompt = self.last_delivered_to.get(from);
        let hops = match prompt {
            Some(p) if p.from == to && now - p.delivered.unwrap_or(0) < 10 * 60_000 => p.hops + 1,
            _ => 0,
        };
        if from != "user" && hops >= max_hops {
            return Err(error(format!("hop limit reached ({max_hops}) between {from_name} and {to_name}; a human needs to step in")));
        }
        let recent = self.log.iter().filter(|m| m.from == from && m.to == to && now - m.at < 60_000).count();
        if from != "user" && recent >= per_minute {
            return Err(error(format!("rate limit: {per_minute} messages/minute from {from_name} to {to_name}")));
        }
        self.seq += 1;
        let m = Message { id: self.seq, from: from.into(), from_name: from_name.into(), reply_to, to: to.into(), to_name: to_name.into(), body: body.into(), hops, at: now, delivered: None };
        self.log.push(m.clone());
        if self.log.len() > 500 {
            self.log.remove(0);
        }
        Ok(m)
    }

    pub fn pending(&self, to: &str) -> Vec<Message> {
        self.log.iter().filter(|m| m.to == to && m.delivered.is_none()).cloned().collect()
    }

    pub fn mark_delivered(&mut self, id: u64) {
        let Some(m) = self.log.iter_mut().find(|m| m.id == id) else { return };
        m.delivered = Some(now_ms());
        self.last_delivered_to.insert(m.to.clone(), m.clone());
    }

    // Pull mode: hand over everything queued and count it delivered.
    pub fn take(&mut self, to: &str) -> Vec<Message> {
        let ms = self.pending(to);
        for m in &ms {
            self.mark_delivered(m.id);
        }
        ms
    }

    pub fn frame(m: &Message) -> String {
        let from = if m.from == "user" { "the user".to_string() } else { format!("@{}", m.from_name) };
        let reply = if m.from == "user" { String::new() } else { format!(" (reply: modisa send {} \"...\")", m.reply_to.as_deref().unwrap_or(&m.from)) };
        format!("[modisa] message from {from}{reply}:\n{}", m.body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hops_and_rate_limits() {
        let mut mb = Mailbox::new();
        let m = mb.send("p1", "a", "p2", "b", "hi", Some("p1:x".into()), 2, 5).unwrap();
        mb.mark_delivered(m.id);
        let r = mb.send("p2", "b", "p1", "a", "re", None, 2, 5).unwrap();
        assert_eq!(r.hops, 1);
        mb.mark_delivered(r.id);
        assert!(mb.send("p1", "a", "p2", "b", "again", None, 2, 5).unwrap_err().message.contains("hop limit"));
        assert_eq!(Mailbox::frame(&m), "[modisa] message from @a (reply: modisa send p1:x \"...\"):\nhi");
        for _ in 0..5 {
            let _ = mb.send("p3", "c", "p4", "d", "x", None, 10, 5);
        }
        assert!(mb.send("p3", "c", "p4", "d", "x", None, 10, 5).unwrap_err().message.contains("rate limit"));
    }
}
