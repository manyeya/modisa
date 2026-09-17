// Agent ↔ agent messages. Queued per recipient, typed in only when the recipient is idle.
// replyTo: the sender as "<id>:<instance>", which reaches only that pane, even after it's renamed
export type Message = { id: number; from: string; fromName: string; replyTo?: string; to: string; toName: string; body: string; hops: number; at: number; delivered?: number };

export class Mailbox {
  log: Message[] = []; // ponytail: last 500 in memory; persist if anyone needs history across restarts
  paused = false;
  private seq = Date.now(); // ids keep rising across restarts, so an id never names two messages
  private lastDeliveredTo = new Map<string, Message>();

  constructor(private limits: () => { max_hops: number; per_minute: number }) {}

  send(from: string, fromName: string, to: string, toName: string, body: string, replyTo?: string): Message {
    // a reply inherits the hop count of the message that prompted it
    const prompt = this.lastDeliveredTo.get(from);
    const hops = prompt && prompt.from === to && Date.now() - prompt.delivered! < 10 * 60_000 ? prompt.hops + 1 : 0;
    const { max_hops, per_minute } = this.limits();
    if (from !== "user" && hops >= max_hops) throw new Error(`hop limit reached (${max_hops}) between ${fromName} and ${toName}; a human needs to step in`);
    const recent = this.log.filter((m) => m.from === from && m.to === to && Date.now() - m.at < 60_000).length;
    if (from !== "user" && recent >= per_minute) throw new Error(`rate limit: ${per_minute} messages/minute from ${fromName} to ${toName}`);
    const m: Message = { id: ++this.seq, from, fromName, replyTo, to, toName, body, hops, at: Date.now() };
    this.log.push(m);
    if (this.log.length > 500) this.log.shift();
    return m;
  }

  pending(to: string) {
    return this.log.filter((m) => m.to === to && !m.delivered);
  }

  markDelivered(m: Message) {
    m.delivered = Date.now();
    this.lastDeliveredTo.set(m.to, m);
  }

  // Pull mode: hand over everything queued and count it delivered.
  take(to: string) {
    const ms = this.pending(to);
    ms.forEach((m) => this.markDelivered(m));
    return ms;
  }

  static frame(m: Message) {
    const from = m.from === "user" ? "the user" : `@${m.fromName}`;
    const reply = m.from === "user" ? "" : ` (reply: modisa send ${m.replyTo ?? m.from} "...")`;
    return `[modisa] message from ${from}${reply}:\n${m.body}`;
  }
}
