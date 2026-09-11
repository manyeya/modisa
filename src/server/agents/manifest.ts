// Screen manifests: per-agent rules that read an agent's live UI (and its terminal title / OSC 9;4
// progress) to decide idle, working or blocked: rules with priorities, regions and nested
// all/any/not gates, in the manifest format of ../../config/agents/manifests (Apache-2.0).

import type { RawGate, RawRule, RuleState } from "../../config/agents";

export type { RawRule, RuleState };

type Gate = { all: Gate[]; any: Gate[]; not: Gate[]; contains: string[]; regex: RegExp[]; lineRegex: RegExp[] };
export type Rule = { id: string; state: RuleState; priority: number; region: string; visible: { idle: boolean; blocker: boolean; working: boolean }; skip: boolean; gate: Gate };

export type Evidence = { screen: string; title?: string; progress?: string };
export type Verdict = { state: RuleState; rule?: string; visibleIdle: boolean; visibleBlocker: boolean; skip: boolean };

// Rust regex → JavaScript: leading inline flags become RegExp flags, \x{…} becomes \u{…}, and the
// text anchors \A / \z become lookarounds (they ignore multiline mode, like Rust's).
export function toRegExp(pattern: string): RegExp {
  let flags = "u";
  const body = pattern
    .replace(/^\(\?([ims]+)\)/, (_, f: string) => ((flags += f), ""))
    .replace(/\\x\{([0-9a-fA-F]+)\}/g, "\\u{$1}")
    .replace(/\\A/g, "(?<![\\s\\S])")
    .replace(/\\z/g, "(?![\\s\\S])");
  return new RegExp(body, flags);
}

function compileGate(g: RawGate): Gate {
  return {
    all: (g.all ?? []).map(compileGate),
    any: (g.any ?? []).map(compileGate),
    not: (g.not ?? []).map(compileGate),
    contains: (g.contains ?? []).map((s) => s.toLowerCase()),
    regex: (g.regex ?? []).map(toRegExp),
    lineRegex: (g.line_regex ?? []).map(toRegExp),
  };
}

export function compileRules(raw: RawRule[]): Rule[] {
  return raw.map((r) => {
    try {
      return {
        id: r.id, state: r.state ?? "unknown", priority: r.priority ?? 0, region: (r.region ?? "whole_recent").trim(),
        visible: { idle: !!r.visible_idle, blocker: !!r.visible_blocker, working: !!r.visible_working },
        skip: !!r.skip_state_update, gate: compileGate(r),
      };
    } catch (e) {
      throw new Error(`rule ${r.id}: ${(e as Error).message}`);
    }
  });
}

function matches(g: Gate, text: string, lower: string, lines: string[]): boolean {
  return g.contains.every((c) => lower.includes(c))
    && g.regex.every((r) => r.test(text))
    && g.lineRegex.every((r) => lines.some((l) => r.test(l)))
    && g.all.every((n) => matches(n, text, lower, lines))
    && (!g.any.length || g.any.some((n) => matches(n, text, lower, lines)))
    && !g.not.some((n) => matches(n, text, lower, lines));
}

// The highest-priority matching rule wins (the earlier one on a tie); no match means idle.
export function evaluate(rules: Rule[], input: Evidence): Verdict {
  let best: Rule | undefined;
  for (const rule of rules) {
    if (best && best.priority >= rule.priority) continue;
    const text = region(input, rule.region);
    const lines = splitLines(text);
    if (matches(rule.gate, text, text.toLowerCase(), lines)) best = rule;
  }
  if (!best) return { state: "idle", visibleIdle: false, visibleBlocker: false, skip: false };
  return { state: best.state, rule: best.id, visibleIdle: best.visible.idle && best.state === "idle", visibleBlocker: best.visible.blocker && best.state === "blocked", skip: best.skip };
}

// ---------- regions: which part of the screen a rule reads ----------

// Like Rust's str::lines: no trailing empty line, and \r\n endings trimmed.
function splitLines(text: string): string[] {
  if (!text) return [];
  const lines = text.split("\n").map((l) => (l.endsWith("\r") ? l.slice(0, -1) : l));
  if (text.endsWith("\n")) lines.pop();
  return lines;
}

// Character offset where line `index` starts (each line plus its \n).
const lineStart = (content: string, lines: string[], index: number) =>
  Math.min(content.length, lines.slice(0, index).reduce((n, l) => n + l.length + 1, 0));

const count = (spec: string, name: string) => {
  const m = new RegExp(`^${name}\\((\\d+)\\)$`).exec(spec);
  return m ? Number(m[1]) : undefined;
};

export function region(input: Evidence, spec: string): string {
  if (spec === "osc_title") return input.title ?? "";
  if (spec === "osc_progress") return input.progress ?? "";
  const content = input.screen;
  const lines = splitLines(content);
  const from = (index: number) => content.slice(lineStart(content, lines, index));
  switch (spec) {
    case "whole_recent": return content;
    case "after_last_prompt_marker": {
      const i = findLastIndex(lines, codexPrompt);
      return i < 0 ? content : from(i + 1);
    }
    case "before_current_prompt_marker": {
      const i = currentCodexPrompt(lines);
      return i === undefined ? content : content.slice(0, lineStart(content, lines, i));
    }
    case "whole_recent_without_current_prompt_marker": return currentCodexPrompt(lines) === undefined ? content : "";
    case "current_prompt_block_marker": {
      const p = currentCodexPrompt(lines);
      return p === undefined ? "" : (lines.slice(0, p).reverse().find(codexBlockMarker) ?? "");
    }
    case "after_current_prompt_block_marker": {
      const p = currentCodexPrompt(lines);
      const b = p === undefined ? -1 : findLastIndex(lines.slice(0, p), codexBlockMarker);
      return b < 0 ? "" : from(b);
    }
    case "prompt_box_body": {
      const top = promptBoxTop(lines);
      if (top === undefined) return "";
      const rel = lines.slice(top + 1).findIndex(isHorizontalRule);
      const end = rel < 0 ? lines.length : top + 1 + rel;
      return content.slice(lineStart(content, lines, top + 1), lineStart(content, lines, end));
    }
    case "above_prompt_box": return aboveBox(content, lines);
    case "last_non_empty_above_prompt_box": return splitLines(aboveBox(content, lines)).reverse().find((l) => l.trim()) ?? "";
    case "after_last_horizontal_rule": {
      let end = 0, offset = 0;
      for (const l of lines) {
        offset += l.length + 1;
        if (isHorizontalRule(l)) end = Math.min(offset, content.length);
      }
      return content.slice(end);
    }
  }
  let n = count(spec, "bottom_lines");
  if (n !== undefined) return from(Math.max(0, lines.length - n));
  n = count(spec, "bottom_non_empty_lines");
  if (n !== undefined) {
    const nonEmpty = lines.flatMap((l, i) => (l.trim() ? [i] : []));
    return nonEmpty.length && n ? from(nonEmpty[Math.max(0, nonEmpty.length - n)]!) : "";
  }
  n = count(spec, "top_non_empty_lines");
  if (n !== undefined) {
    const nonEmpty = lines.flatMap((l, i) => (l.trim() ? [i] : []));
    return nonEmpty.length && n ? content.slice(0, lineStart(content, lines, nonEmpty[Math.min(n, nonEmpty.length) - 1]! + 1)) : "";
  }
  return "";
}

function findLastIndex<T>(list: T[], ok: (x: T) => boolean) {
  for (let i = list.length - 1; i >= 0; i--) if (ok(list[i]!)) return i;
  return -1;
}

const codexPrompt = (l: string) => l === "›" || l.startsWith("› ");
const codexBlockMarker = (l: string) => /^[•■✗✓]/.test(l);
// Codex's live prompt: the last › line with no transcript block after it.
function currentCodexPrompt(lines: string[]) {
  const i = findLastIndex(lines, codexPrompt);
  return i < 0 || lines.slice(i + 1).some(codexBlockMarker) ? undefined : i;
}

// A rule line of ─ (optionally with a short label after at least three of them).
function isHorizontalRule(line: string) {
  const t = line.trim();
  const rule = /^─*/.exec(t)![0].length;
  return rule > 0 && (!t.slice(rule).trimStart() || rule >= 3);
}

// The prompt box is framed by the last two horizontal rules; its top is the second-to-last.
function promptBoxTop(lines: string[]) {
  let seen = 0;
  for (let i = lines.length - 1; i >= 0; i--) if (isHorizontalRule(lines[i]!) && ++seen === 2) return i;
}

function aboveBox(content: string, lines: string[]) {
  const top = promptBoxTop(lines);
  return top === undefined ? content : content.slice(0, lineStart(content, lines, top));
}
