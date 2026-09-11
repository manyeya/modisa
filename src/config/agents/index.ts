// Every agent shepherd knows: the process names that identify it, how to launch and resume it, and
// the screen rules that read its state. The rules in ./manifests are third-party detection manifests
// (Apache-2.0, see ./manifests/LICENSE), kept as close to upstream as possible so they can be
// re-synced; `extra` adds rules of ours on top. Imported so a compiled binary carries them.
import amp from "./manifests/amp.toml";
import antigravity from "./manifests/antigravity.toml";
import claude from "./manifests/claude.toml";
import cline from "./manifests/cline.toml";
import codex from "./manifests/codex.toml";
import cursor from "./manifests/cursor.toml";
import devin from "./manifests/devin.toml";
import droid from "./manifests/droid.toml";
import gemini from "./manifests/gemini.toml";
import copilot from "./manifests/github-copilot.toml";
import grok from "./manifests/grok.toml";
import hermes from "./manifests/hermes.toml";
import kilo from "./manifests/kilo.toml";
import kimi from "./manifests/kimi.toml";
import kiro from "./manifests/kiro.toml";
import maki from "./manifests/maki.toml";
import muse from "./manifests/muse.toml";
import opencode from "./manifests/opencode.toml";
import pi from "./manifests/pi.toml";
import qodercli from "./manifests/qodercli.toml";
import qwen from "./manifests/qwen.toml";

export type RuleState = "idle" | "working" | "blocked" | "unknown";

// A screen rule (the manifest schema) or a nested gate: every direct matcher must hold, every
// `all` gate, at least one `any` gate (when present), and no `not` gate. The highest priority wins.
export type RawGate = { all?: RawGate[]; any?: RawGate[]; not?: RawGate[]; contains?: string[]; regex?: string[]; line_regex?: string[] };
export type RawRule = RawGate & {
  id: string;
  state?: RuleState;
  priority?: number;
  region?: string; // which part of the screen, e.g. bottom_non_empty_lines(8), after_last_horizontal_rule, osc_title
  visible_idle?: boolean;
  visible_blocker?: boolean;
  visible_working?: boolean;
  skip_state_update?: boolean; // an agent-owned viewer (transcript, model picker): keep the last state
};

export type AgentDef = {
  id: string; // stable: saved sessions, [agents.<id>] config and integrations refer to it
  name: string;
  process: string[]; // executable names (lowercase, without .exe/.js); the first is what we launch
  launch: string;
  resume?: string; // relaunch after a restart when no exact session id was reported
  resumeSession?: string; // relaunch into the exact session an integration reported: {id} is replaced
  activity?: boolean; // no rules matched: output in the last 2s counts as working
  rules: RawRule[];
};

const rules = (manifest: any, extra: RawRule[] = []): RawRule[] => [...manifest.rules, ...extra];

export const BUILTIN_AGENTS: AgentDef[] = [
  { id: "claude-code", name: "Claude Code", process: ["claude", "claude-code"], launch: "claude", resumeSession: "claude --resume {id}", resume: "claude --continue", rules: rules(claude) },
  {
    id: "codex", name: "Codex", process: ["codex"], launch: "codex", resumeSession: "codex resume {id}", resume: "codex resume --last",
    rules: rules(codex, [
      // Codex's rate-limit "switch model" picker ends in "esc to go back", which the manifest misses.
      { id: "shepherd_confirm_go_back", state: "blocked", priority: 900, region: "bottom_non_empty_lines(3)", visible_blocker: true, contains: ["press enter to confirm or esc to go back"] },
    ]),
  },
  { id: "gemini", name: "Gemini CLI", process: ["gemini"], launch: "gemini", resume: "gemini --resume", rules: rules(gemini) },
  { id: "cursor-agent", name: "Cursor Agent", process: ["cursor-agent", "cursor"], launch: "cursor-agent", resumeSession: "cursor-agent --resume {id}", rules: rules(cursor) },
  { id: "copilot", name: "Copilot CLI", process: ["copilot", "github-copilot", "ghcs"], launch: "copilot", resumeSession: "copilot --resume={id}", rules: rules(copilot) },
  { id: "opencode", name: "OpenCode", process: ["opencode", "opencode2", "open-code"], launch: "opencode", resumeSession: "opencode --session {id}", resume: "opencode --continue", rules: rules(opencode) },
  { id: "pi", name: "Pi", process: ["pi"], launch: "pi", resumeSession: "pi --session {id}", resume: "pi --continue", rules: rules(pi) },
  { id: "omp", name: "OMP", process: ["omp"], launch: "omp", resumeSession: "omp --resume={id}", activity: true, rules: [] }, // state comes from its extension
  { id: "droid", name: "Droid", process: ["droid"], launch: "droid", resumeSession: "droid --resume {id}", rules: rules(droid) },
  { id: "amp", name: "Amp", process: ["amp", "amp-local"], launch: "amp", rules: rules(amp) },
  { id: "kiro", name: "Kiro CLI", process: ["kiro-cli", "kiro"], launch: "kiro-cli", rules: rules(kiro) },
  { id: "kimi", name: "Kimi Code", process: ["kimi", "kimi-code"], launch: "kimi", resumeSession: "kimi --session {id}", rules: rules(kimi) },
  { id: "kilo", name: "Kilo Code", process: ["kilo", "kilo-code"], launch: "kilo", resumeSession: "kilo --session {id}", rules: rules(kilo) },
  { id: "devin", name: "Devin CLI", process: ["devin", "devin-cli"], launch: "devin", resumeSession: "devin --resume {id}", rules: rules(devin) },
  { id: "grok", name: "Grok CLI", process: ["grok", "grok-build"], launch: "grok", resumeSession: "grok --resume {id}", rules: rules(grok) },
  { id: "hermes", name: "Hermes Agent", process: ["hermes", "hermes-agent"], launch: "hermes", resumeSession: "hermes --resume {id}", rules: rules(hermes) },
  { id: "qodercli", name: "Qoder CLI", process: ["qodercli", "qoderclicn", "qoder", "qodercn"], launch: "qodercli", resumeSession: "qodercli --resume {id}", rules: rules(qodercli) },
  { id: "qwen", name: "Qwen Code", process: ["qwen", "qwen-code"], launch: "qwen", resumeSession: "qwen --resume {id}", rules: rules(qwen) },
  { id: "antigravity", name: "Antigravity CLI", process: ["agy", "antigravity", "antigravity-cli"], launch: "agy", resumeSession: "agy --conversation {id}", rules: rules(antigravity) },
  { id: "cline", name: "Cline", process: ["cline"], launch: "cline", rules: rules(cline) },
  { id: "mastracode", name: "MastraCode", process: ["mastracode", "mastra-code"], launch: "mastracode", resumeSession: "mastracode --thread {id}", activity: true, rules: [] }, // state comes from its hooks
  { id: "maki", name: "Maki", process: ["maki"], launch: "maki", rules: rules(maki) },
  { id: "muse", name: "Muse", process: ["muse", "muse-code", "muse-cli"], launch: "muse", rules: rules(muse) },
  {
    id: "aider", name: "Aider", process: ["aider"], launch: "aider", activity: true,
    rules: [
      { id: "confirm", state: "blocked", priority: 20, regex: ["(?im)Do you want to|Allow (this|command)|\\(y/n\\)|\\[y/N\\]"] },
      { id: "working", state: "working", priority: 10, regex: ["(?im)esc to (interrupt|cancel)"] },
    ],
  },
  // any agent spawned by name that shepherd doesn't know: output in the last 2s = working
  { id: "generic", name: "Agent", process: [], launch: "", activity: true, rules: [] },
];
